use anyhow::Context;
use clap::{Subcommand, ValueEnum};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{emit, events_log_path, hook_installer, opencode, socket_path, BridgeEvent, CardStage};

#[derive(Subcommand, Debug)]
pub enum BridgeSubcommand {
    /// Start the Unix socket listener (daemon mode).
    Listen {
        #[arg(long, default_value = "false")]
        verbose: bool,
    },
    /// Emit a single event to the socket (called by hooks/scripts).
    Emit {
        #[arg(long)]
        cli: String,
        #[arg(long)]
        event: String,
        #[arg(long = "session", alias = "session-id")]
        session_id: Option<String>,
        #[arg(long)]
        card_id: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(long)]
        stage: Option<String>,
    },
    /// Install Claude Code hooks.
    Install {
        #[arg(long, value_enum, default_value = "claude")]
        target: HookTarget,
    },
    /// Remove installed hooks.
    Uninstall {
        #[arg(long, value_enum, default_value = "claude")]
        target: HookTarget,
    },
    /// Connect to opencode SSE bus and forward events.
    Opencode {
        #[arg(long)]
        port: Option<u16>,
    },
}

#[derive(Clone, Debug, ValueEnum)]
pub enum HookTarget {
    Claude,
}

pub async fn cmd_bridge(sub: BridgeSubcommand) -> anyhow::Result<()> {
    match sub {
        BridgeSubcommand::Listen { verbose } => run_listener(verbose).await,
        BridgeSubcommand::Emit {
            cli,
            event,
            session_id,
            card_id,
            tool,
            stage,
        } => {
            // Hook callers should never fail the parent CLI session.
            if let Ok(parsed) = build_emit_event(cli, event, session_id, card_id, tool, stage) {
                let _ = emit(&parsed);
            }
            Ok(())
        }
        BridgeSubcommand::Install { target } => {
            let bop_bin = std::env::current_exe().context("failed to resolve bop binary path")?;
            match target {
                HookTarget::Claude => hook_installer::install_claude_hooks(&bop_bin),
            }
        }
        BridgeSubcommand::Uninstall { target } => match target {
            HookTarget::Claude => hook_installer::uninstall_claude_hooks(),
        },
        BridgeSubcommand::Opencode { port } => {
            let discovered_port = port
                .or_else(|| {
                    std::env::var("OPENCODE_PORT")
                        .ok()
                        .and_then(|raw| raw.parse::<u16>().ok())
                })
                .unwrap_or(4096);
            opencode::listen_opencode(discovered_port).await
        }
    }
}

fn build_emit_event(
    cli: String,
    event: String,
    session_id: Option<String>,
    card_id: Option<String>,
    tool: Option<String>,
    stage: Option<String>,
) -> anyhow::Result<BridgeEvent> {
    let cli = cli.trim().to_string();
    let event_name = event.trim().to_ascii_lowercase();
    let session_id = session_id
        .or_else(|| std::env::var("SESSION_ID").ok())
        .or_else(|| std::env::var("CLAUDE_SESSION_ID").ok())
        .unwrap_or_else(|| "unknown".to_string());

    let card_id = card_id.or_else(|| std::env::var("CARD_ID").ok());
    let tool = tool
        .or_else(|| std::env::var("TOOL_NAME").ok())
        .or_else(|| std::env::var("CLAUDE_TOOL_NAME").ok())
        .unwrap_or_else(|| "unknown".to_string());

    match event_name.as_str() {
        "session-start" => Ok(BridgeEvent::SessionStart {
            cli,
            session_id,
            card_id,
        }),
        "stage-change" => {
            let stage = stage
                .context("missing --stage for stage-change")?
                .parse::<CardStage>()?;
            Ok(BridgeEvent::StageChange {
                cli,
                session_id,
                card_id,
                stage,
            })
        }
        "tool-start" => Ok(BridgeEvent::ToolStart {
            cli,
            session_id,
            tool,
        }),
        "tool-done" => Ok(BridgeEvent::ToolDone {
            cli,
            session_id,
            tool,
            success: true,
        }),
        "awaiting-human" => Ok(BridgeEvent::AwaitingHuman {
            cli,
            session_id,
            reason: None,
        }),
        "session-end" => Ok(BridgeEvent::SessionEnd {
            cli,
            session_id,
            card_id,
            exit_ok: true,
        }),
        _ => anyhow::bail!("unsupported bridge event: {event_name}"),
    }
}

async fn run_listener(verbose: bool) -> anyhow::Result<()> {
    let socket_path = socket_path().context("cannot resolve ~/.bop/bridge.sock")?;
    let events_log_path =
        events_log_path().unwrap_or_else(|| socket_path.with_file_name("bridge-events.jsonl"));

    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = events_log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    if socket_path.exists() {
        match StdUnixStream::connect(&socket_path) {
            Ok(_) => {
                println!("bridge already running at {}", socket_path.display());
                return Ok(());
            }
            Err(_) => {
                fs::remove_file(&socket_path).with_context(|| {
                    format!("failed to remove stale socket {}", socket_path.display())
                })?;
            }
        }
    }

    let listener = tokio::net::UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;

    let mut log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_log_path)
        .with_context(|| format!("failed to open {}", events_log_path.display()))?;

    if verbose {
        eprintln!("bridge listening on {}", socket_path.display());
    }

    let result = run_accept_loop(listener, verbose, &mut log_file).await;
    let _ = fs::remove_file(&socket_path);
    result
}

async fn run_accept_loop(
    listener: tokio::net::UnixListener,
    verbose: bool,
    log_file: &mut fs::File,
) -> anyhow::Result<()> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("failed to register SIGTERM handler")?;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = ctrl_c.as_mut() => {
                break;
            }
            _ = sigterm.recv() => {
                break;
            }
            accept_res = listener.accept() => {
                let (stream, _) = match accept_res {
                    Ok(tuple) => tuple,
                    Err(err) => {
                        if verbose {
                            eprintln!("bridge accept error: {err}");
                        }
                        continue;
                    }
                };

                if let Err(err) = handle_connection(stream, verbose, log_file).await {
                    if verbose {
                        eprintln!("bridge connection error: {err}");
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    verbose: bool,
    log_file: &mut fs::File,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            break;
        }

        let payload = line.trim_end_matches(['\n', '\r']);
        if payload.is_empty() {
            continue;
        }

        match serde_json::from_str::<BridgeEvent>(payload) {
            Ok(event) => {
                if verbose {
                    println!("{}", serde_json::to_string(&event)?);
                }
                append_event(log_file, &event)?;
            }
            Err(err) => {
                if verbose {
                    eprintln!("bridge ignored malformed event: {err}");
                }
            }
        }
    }

    Ok(())
}

fn append_event(log_file: &mut fs::File, event: &BridgeEvent) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    log_file.write_all(line.as_bytes())?;
    log_file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_builder_parses_stage_change() {
        let event = build_emit_event(
            "claude".to_string(),
            "stage-change".to_string(),
            Some("s1".to_string()),
            Some("c1".to_string()),
            None,
            Some("in-progress".to_string()),
        )
        .unwrap();

        assert_eq!(
            event,
            BridgeEvent::StageChange {
                cli: "claude".to_string(),
                session_id: "s1".to_string(),
                card_id: Some("c1".to_string()),
                stage: CardStage::InProgress,
            }
        );
    }

    #[test]
    fn emit_builder_parses_tool_done() {
        let event = build_emit_event(
            "claude".to_string(),
            "tool-done".to_string(),
            Some("s2".to_string()),
            None,
            Some("edit".to_string()),
            None,
        )
        .unwrap();
        assert_eq!(
            event,
            BridgeEvent::ToolDone {
                cli: "claude".to_string(),
                session_id: "s2".to_string(),
                tool: "edit".to_string(),
                success: true,
            }
        );
    }
}
