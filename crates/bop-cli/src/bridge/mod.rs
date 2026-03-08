use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod cmd;
pub mod hook_installer;
pub mod opencode;

pub use cmd::{cmd_bridge, BridgeSubcommand};

/// Unix socket path: `~/.bop/bridge.sock`.
pub fn socket_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".bop").join("bridge.sock"))
}

/// JSONL event log path: `~/.bop/bridge-events.jsonl`.
pub fn events_log_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".bop").join("bridge-events.jsonl"))
}

/// Five canonical card stages (BopDeck state machine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CardStage {
    Planning,
    InProgress,
    HumanReview,
    AiReview,
    Done,
}

impl std::fmt::Display for CardStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CardStage::Planning => "planning",
            CardStage::InProgress => "in-progress",
            CardStage::HumanReview => "human-review",
            CardStage::AiReview => "ai-review",
            CardStage::Done => "done",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for CardStage {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "planning" => Ok(CardStage::Planning),
            "in-progress" => Ok(CardStage::InProgress),
            "human-review" => Ok(CardStage::HumanReview),
            "ai-review" => Ok(CardStage::AiReview),
            "done" => Ok(CardStage::Done),
            other => anyhow::bail!("unknown stage: {other}"),
        }
    }
}

/// Session events emitted by CLI adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "kebab-case",
    rename_all_fields = "kebab-case",
    tag = "event"
)]
pub enum BridgeEvent {
    /// A CLI session started.
    SessionStart {
        cli: String,
        session_id: String,
        card_id: Option<String>,
    },
    /// Card stage changed.
    StageChange {
        cli: String,
        session_id: String,
        card_id: Option<String>,
        stage: CardStage,
    },
    /// A tool call started (PreToolUse equivalent).
    ToolStart {
        cli: String,
        session_id: String,
        tool: String,
    },
    /// A tool call finished.
    ToolDone {
        cli: String,
        session_id: String,
        tool: String,
        success: bool,
    },
    /// AI is waiting for human input / permission.
    AwaitingHuman {
        cli: String,
        session_id: String,
        reason: Option<String>,
    },
    /// Session ended.
    SessionEnd {
        cli: String,
        session_id: String,
        card_id: Option<String>,
        exit_ok: bool,
    },
}

/// Write one JSON-line event to the Unix socket.
///
/// Non-fatal if socket not present — bridge may not be running.
pub fn emit(event: &BridgeEvent) -> anyhow::Result<()> {
    let Some(path) = socket_path() else {
        return Ok(());
    };
    emit_to_path(&path, event)
}

fn emit_to_path(path: &Path, event: &BridgeEvent) -> anyhow::Result<()> {
    use std::io::{ErrorKind, Write};
    use std::os::unix::net::UnixStream;

    if !path.exists() {
        return Ok(());
    }

    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(err)
            if err.kind() == ErrorKind::NotFound || err.kind() == ErrorKind::ConnectionRefused =>
        {
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::tempdir;

    #[test]
    fn test_event_serialize_session_start() {
        let event = BridgeEvent::SessionStart {
            cli: "claude".to_string(),
            session_id: "s-1".to_string(),
            card_id: Some("card-1".to_string()),
        };
        let value: Value = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();

        assert_eq!(value["event"], "session-start");
        assert_eq!(value["session-id"], "s-1");
        assert_eq!(value["card-id"], "card-1");
    }

    #[test]
    fn test_event_serialize_stage_change() {
        let event = BridgeEvent::StageChange {
            cli: "claude".to_string(),
            session_id: "s-2".to_string(),
            card_id: None,
            stage: CardStage::InProgress,
        };
        let value: Value = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(value["event"], "stage-change");
        assert_eq!(value["stage"], "in-progress");
    }

    #[test]
    fn test_emit_no_socket_is_noop() {
        let td = tempdir().unwrap();
        let missing = td.path().join("missing.sock");
        let event = BridgeEvent::SessionStart {
            cli: "claude".to_string(),
            session_id: "noop".to_string(),
            card_id: None,
        };
        let result = emit_to_path(&missing, &event);
        assert!(result.is_ok());
    }

    #[test]
    fn test_roundtrip_all_variants() {
        let events = vec![
            BridgeEvent::SessionStart {
                cli: "claude".to_string(),
                session_id: "s1".to_string(),
                card_id: Some("c1".to_string()),
            },
            BridgeEvent::StageChange {
                cli: "claude".to_string(),
                session_id: "s1".to_string(),
                card_id: None,
                stage: CardStage::Planning,
            },
            BridgeEvent::ToolStart {
                cli: "opencode".to_string(),
                session_id: "s2".to_string(),
                tool: "read-file".to_string(),
            },
            BridgeEvent::ToolDone {
                cli: "opencode".to_string(),
                session_id: "s2".to_string(),
                tool: "read-file".to_string(),
                success: true,
            },
            BridgeEvent::AwaitingHuman {
                cli: "goose".to_string(),
                session_id: "s3".to_string(),
                reason: Some("permission".to_string()),
            },
            BridgeEvent::SessionEnd {
                cli: "claude".to_string(),
                session_id: "s4".to_string(),
                card_id: Some("c4".to_string()),
                exit_ok: true,
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let parsed: BridgeEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, event);
        }
    }

    #[test]
    fn test_stage_from_str() {
        let stage: CardStage = "human-review".parse().unwrap();
        assert_eq!(stage, CardStage::HumanReview);
    }
}
