use chrono::{SecondsFormat, Utc};
use serde_json::json;
use std::env;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use uuid::Uuid;

use super::ProviderSnapshot;

const PRODUCER_URL: &str = "https://github.com/ryanmaclean/bop";
const FACET_SCHEMA_URL: &str = "bop://facets/provider-quota/v1";
const BOPDECK_SOCKET_DIR: &str = "/tmp";

#[derive(Debug, Clone)]
pub struct BopDeckWriter {
    socket_path: PathBuf,
}

impl BopDeckWriter {
    pub fn new() -> Self {
        let username = env::var("USER")
            .or_else(|_| env::var("LOGNAME"))
            .unwrap_or_else(|_| "unknown".to_string());
        let socket_path =
            PathBuf::from(BOPDECK_SOCKET_DIR).join(format!("bop-deck-{username}.sock"));
        Self { socket_path }
    }

    #[cfg(test)]
    fn with_socket_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    pub fn detect(&self) -> bool {
        self.socket_path.exists()
    }

    pub async fn emit(&self, snapshot: &ProviderSnapshot) -> anyhow::Result<()> {
        if !self.detect() {
            return Ok(());
        }

        let reset_at = snapshot
            .reset_at
            .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true));
        let event_time = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let run_id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, snapshot.provider.as_bytes()).to_string();

        let event = json!({
            "eventType": "RUNNING",
            "eventTime": event_time,
            "run": {
                "runId": run_id,
                "facets": {},
            },
            "job": {
                "namespace": "bop",
                "name": format!("provider.{}", snapshot.provider),
            },
            "inputs": [],
            "outputs": [],
            "facets": {
                "bop_provider_quota": {
                    "_producer": PRODUCER_URL,
                    "_schemaURL": FACET_SCHEMA_URL,
                    "provider": snapshot.provider,
                    "displayName": snapshot.display_name,
                    "primaryPct": snapshot.primary_pct,
                    "secondaryPct": snapshot.secondary_pct,
                    "primaryLabel": snapshot.primary_label,
                    "secondaryLabel": snapshot.secondary_label,
                    "resetAt": reset_at,
                    "tokensUsed": snapshot.tokens_used,
                    "costUsd": snapshot.cost_usd,
                    "source": snapshot.source,
                    "error": snapshot.error,
                }
            }
        });

        let mut payload = serde_json::to_vec(&event)?;
        payload.push(b'\n');

        let mut stream = match UnixStream::connect(&self.socket_path).await {
            Ok(stream) => stream,
            Err(err) => {
                debug_log(format_args!(
                    "bopdeck connect failed ({}): {}",
                    self.socket_path.display(),
                    err
                ));
                return Ok(());
            }
        };

        if let Err(err) = stream.write_all(&payload).await {
            debug_log(format_args!(
                "bopdeck write failed ({}): {}",
                self.socket_path.display(),
                err
            ));
            return Ok(());
        }

        Ok(())
    }
}

fn debug_log(args: std::fmt::Arguments<'_>) {
    let enabled = env::var("RUST_LOG")
        .ok()
        .map(|value| value.to_ascii_lowercase().contains("debug"))
        .unwrap_or(false);
    if enabled {
        eprintln!("[providers][bopdeck][debug] {args}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use tokio::io::AsyncReadExt;

    fn snapshot(provider: &str) -> ProviderSnapshot {
        ProviderSnapshot {
            provider: provider.to_string(),
            display_name: "Claude Code".to_string(),
            primary_pct: Some(57),
            secondary_pct: Some(38),
            primary_label: Some("5h".to_string()),
            secondary_label: Some("7d".to_string()),
            tokens_used: None,
            cost_usd: None,
            reset_at: None,
            source: "oauth".to_string(),
            error: None,
            loaded_models: None,
        }
    }

    #[test]
    fn detect_false_when_socket_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sock");
        let writer = BopDeckWriter::with_socket_path(path);
        assert!(!writer.detect());
    }

    #[tokio::test]
    async fn emit_writes_openlineage_event() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("bopdeck.sock");
        let listener = match tokio::net::UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return,
            Err(err) => panic!("failed to bind unix listener: {err}"),
        };
        let writer = BopDeckWriter::with_socket_path(socket_path.clone());

        let recv_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            String::from_utf8(buf).unwrap()
        });

        writer.emit(&snapshot("claude")).await.unwrap();

        let payload = recv_task.await.unwrap();
        assert!(payload.ends_with('\n'));

        let parsed: serde_json::Value = serde_json::from_str(payload.trim_end()).unwrap();
        assert_eq!(parsed["eventType"], "RUNNING");
        assert_eq!(parsed["job"]["namespace"], "bop");
        assert_eq!(parsed["job"]["name"], "provider.claude");
        assert_eq!(
            parsed["run"]["runId"],
            Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"claude").to_string()
        );
        assert_eq!(
            parsed["facets"]["bop_provider_quota"]["_schemaURL"],
            FACET_SCHEMA_URL
        );
    }
}
