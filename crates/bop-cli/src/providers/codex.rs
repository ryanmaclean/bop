use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::{Provider, ProviderSnapshot};

/// Base URL for the Codex OAuth usage endpoint.
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// Number of days after which a stale `last_refresh` triggers a warning.
const REFRESH_WARN_DAYS: i64 = 8;

// ---------------------------------------------------------------------------
// Codex OAuth usage API response types
// ---------------------------------------------------------------------------

/// Top-level response from `GET /backend-api/wham/usage`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UsageResponse {
    #[serde(default)]
    pub session: Option<UsageWindow>,
    #[serde(default)]
    pub weekly: Option<UsageWindow>,
}

/// A single rate-limit window inside the usage response.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UsageWindow {
    #[serde(default)]
    pub percent_used: Option<f64>,
    #[serde(alias = "reset_at", default)]
    pub reset_at: Option<String>,
}

// ---------------------------------------------------------------------------
// JSON-RPC fallback response types
// ---------------------------------------------------------------------------

/// A JSON-RPC response envelope (only the fields we care about).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RpcResponse {
    #[allow(dead_code)]
    pub id: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
    #[allow(dead_code)]
    pub error: Option<serde_json::Value>,
}

/// Parsed rate-limit window from the JSON-RPC `account/rateLimits/read` result.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RpcRateLimitWindow {
    /// Window type, e.g. "session" or "weekly".
    #[serde(rename = "type", default)]
    pub window_type: Option<String>,
    /// Percentage of the window already consumed.
    #[serde(rename = "percentUsed", alias = "percent_used", default)]
    pub percent_used: Option<f64>,
    /// When this window resets (ISO 8601).
    #[serde(rename = "resetAt", alias = "reset_at", default)]
    pub reset_at: Option<String>,
}

/// The `result` object from `account/rateLimits/read`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RpcRateLimitsResult {
    #[serde(default)]
    pub windows: Vec<RpcRateLimitWindow>,
}

/// OAuth credentials as stored by Codex CLI at `~/.codex/auth.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCredentials {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub last_refresh: Option<String>,
}

/// Provider implementation for Codex CLI OAuth quota monitoring.
pub struct CodexProvider;

#[allow(dead_code)] // Methods used by Provider::fetch() and tests; struct registered in subtask-3-1.
impl CodexProvider {
    pub fn new() -> Self {
        Self
    }

    /// Path to the Codex credentials file: `~/.codex/auth.json`.
    fn credentials_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".codex").join("auth.json"))
    }

    /// Check if the `codex` binary is on PATH.
    fn binary_on_path() -> bool {
        std::process::Command::new("which")
            .arg("codex")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if Codex credentials exist on disk.
    fn has_credentials_file() -> bool {
        Self::credentials_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Read and parse credentials from `~/.codex/auth.json`.
    fn read_credentials() -> anyhow::Result<CodexCredentials> {
        let path = Self::credentials_path()
            .context("cannot determine home directory for Codex credentials")?;
        if !path.exists() {
            anyhow::bail!("no Codex credentials found at {}", path.display());
        }
        let json = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read Codex credentials: {}", path.display()))?;
        serde_json::from_str(&json)
            .with_context(|| format!("malformed Codex credentials at {}", path.display()))
    }

    /// Check if `last_refresh` is older than `REFRESH_WARN_DAYS` days.
    /// Returns `Some(warning_message)` if stale, `None` if fresh or unparseable.
    fn check_refresh_staleness(creds: &CodexCredentials) -> Option<String> {
        let last = creds.last_refresh.as_deref()?;
        let parsed = DateTime::parse_from_rfc3339(last).ok()?;
        let age = Utc::now() - parsed.with_timezone(&Utc);
        if age.num_days() >= REFRESH_WARN_DAYS {
            Some(format!(
                "token last refreshed {} days ago (>{}d)",
                age.num_days(),
                REFRESH_WARN_DAYS
            ))
        } else {
            None
        }
    }

    /// Parse a JSON-RPC `account/rateLimits/read` result into a `ProviderSnapshot`.
    /// Separated from `fetch_via_rpc()` so it can be unit-tested without spawning
    /// a real `codex` process.
    fn parse_rpc_response(body: &str) -> anyhow::Result<ProviderSnapshot> {
        let rpc: RpcResponse = match serde_json::from_str(body) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex CLI".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: Some("weekly".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "rpc".into(),
                    error: Some(format!("failed to parse RPC JSON: {e}")),
                    loaded_models: None,
                });
            }
        };

        if let Some(ref err) = rpc.error {
            return Ok(ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "rpc".into(),
                error: Some(format!("RPC error: {err}")),
                loaded_models: None,
            });
        }

        let result_val = match rpc.result {
            Some(v) => v,
            None => {
                return Ok(ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex CLI".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: Some("weekly".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "rpc".into(),
                    error: Some("RPC response missing result field".into()),
                    loaded_models: None,
                });
            }
        };

        let limits: RpcRateLimitsResult = match serde_json::from_value(result_val) {
            Ok(l) => l,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex CLI".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: Some("weekly".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "rpc".into(),
                    error: Some(format!("failed to parse RPC result: {e}")),
                    loaded_models: None,
                });
            }
        };

        // Find session and weekly windows by type.
        let session_window = limits
            .windows
            .iter()
            .find(|w| w.window_type.as_deref() == Some("session"));
        let weekly_window = limits
            .windows
            .iter()
            .find(|w| w.window_type.as_deref() == Some("weekly"));

        let primary_pct = session_window
            .and_then(|w| w.percent_used)
            .map(|p| p.round().clamp(0.0, 100.0) as u8);

        let secondary_pct = weekly_window
            .and_then(|w| w.percent_used)
            .map(|p| p.round().clamp(0.0, 100.0) as u8);

        let reset_at: Option<DateTime<Utc>> = session_window
            .and_then(|w| w.reset_at.as_deref())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(ProviderSnapshot {
            provider: "codex".into(),
            display_name: "Codex CLI".into(),
            primary_pct,
            secondary_pct,
            primary_label: Some("session".into()),
            secondary_label: Some("weekly".into()),
            tokens_used: None,
            cost_usd: None,
            reset_at,
            source: "rpc".into(),
            error: None,
            loaded_models: None,
        })
    }

    /// Extract a percentage number after a given anchor string.
    ///
    /// E.g., `extract_pct_after("5h limit: 42%", "5h limit:")` → `Some(42)`
    ///
    /// Scans for `anchor`, skips whitespace, parses consecutive digits, and
    /// clamps the result to 0–100.
    fn extract_pct_after(text: &str, anchor: &str) -> Option<u8> {
        let idx = text.find(anchor)?;
        let after = &text[idx + anchor.len()..];
        let trimmed = after.trim_start();
        let num_str: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
        if num_str.is_empty() {
            return None;
        }
        let pct: u32 = num_str.parse().ok()?;
        Some(pct.min(100) as u8)
    }

    /// Parse PTY screen output from `codex /status` for rate-limit percentages.
    ///
    /// Looks for patterns like:
    /// - `5h limit: NN%` → primary_pct (session)
    /// - `Weekly limit: NN%` → secondary_pct (weekly)
    ///
    /// Returns a `ProviderSnapshot` with source `"pty"`. If no percentages
    /// are found, the `error` field describes what was missing.
    fn parse_pty_output(output: &str) -> ProviderSnapshot {
        let primary_pct = Self::extract_pct_after(output, "5h limit:");
        let secondary_pct = Self::extract_pct_after(output, "Weekly limit:");

        let error = if primary_pct.is_none() && secondary_pct.is_none() {
            Some("no rate-limit percentages found in PTY output".into())
        } else {
            None
        };

        ProviderSnapshot {
            provider: "codex".into(),
            display_name: "Codex CLI".into(),
            primary_pct,
            secondary_pct,
            primary_label: Some("session".into()),
            secondary_label: Some("weekly".into()),
            tokens_used: None,
            cost_usd: None,
            reset_at: None,
            source: "pty".into(),
            error,
            loaded_models: None,
        }
    }

    /// Spawn `codex -s read-only -a untrusted app-server` and perform JSON-RPC
    /// handshake to fetch rate limits. Returns a snapshot with source "rpc".
    ///
    /// Protocol:
    /// 1. Send `initialize` (id:1) → read response
    /// 2. Send `account/rateLimits/read` (id:2) → read response
    /// 3. Parse `windows` array for session + weekly percent_used
    /// 4. Kill the child process
    ///
    /// Total timeout: 5 seconds.
    async fn fetch_via_rpc() -> anyhow::Result<ProviderSnapshot> {
        let rpc_result = tokio::time::timeout(Duration::from_secs(5), Self::rpc_exchange()).await;

        match rpc_result {
            Ok(Ok(snap)) => Ok(snap),
            Ok(Err(e)) => Ok(ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "rpc".into(),
                error: Some(format!("RPC exchange failed: {e}")),
                loaded_models: None,
            }),
            Err(_) => Ok(ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "rpc".into(),
                error: Some("RPC timed out (5s)".into()),
                loaded_models: None,
            }),
        }
    }

    /// Inner async function for the JSON-RPC exchange with the codex process.
    /// Separated from `fetch_via_rpc` so the timeout wrapper stays clean.
    async fn rpc_exchange() -> anyhow::Result<ProviderSnapshot> {
        let mut child = tokio::process::Command::new("codex")
            .args(["-s", "read-only", "-a", "untrusted", "app-server"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn `codex` process")?;

        let stdin = child.stdin.take().context("no stdin on codex process")?;
        let stdout = child.stdout.take().context("no stdout on codex process")?;

        let mut writer = tokio::io::BufWriter::new(stdin);
        let mut reader = BufReader::new(stdout);

        // Step 1: Send initialize request (id:1).
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"bop","version":"0.1"}}}"#;
        writer
            .write_all(init_req.as_bytes())
            .await
            .context("write initialize")?;
        writer.write_all(b"\n").await.context("write newline")?;
        writer.flush().await.context("flush initialize")?;

        // Read initialize response (line containing id:1).
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .context("read initialize response")?;
            if n == 0 {
                anyhow::bail!("codex process closed stdout before initialize response");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Check if this is a JSON-RPC response with id:1.
            if let Ok(resp) = serde_json::from_str::<RpcResponse>(trimmed) {
                if resp.id == Some(serde_json::Value::Number(1.into())) {
                    break;
                }
            }
            // Not the response we want — continue reading (could be a notification).
        }

        // Step 2: Send account/rateLimits/read request (id:2).
        let limits_req =
            r#"{"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":{}}"#;
        writer
            .write_all(limits_req.as_bytes())
            .await
            .context("write rateLimits")?;
        writer.write_all(b"\n").await.context("write newline")?;
        writer.flush().await.context("flush rateLimits")?;

        // Read rateLimits response (line containing id:2).
        let response_line;
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .context("read rateLimits response")?;
            if n == 0 {
                anyhow::bail!("codex process closed stdout before rateLimits response");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(resp) = serde_json::from_str::<RpcResponse>(trimmed) {
                if resp.id == Some(serde_json::Value::Number(2.into())) {
                    response_line = trimmed.to_string();
                    break;
                }
            }
        }

        // Kill the child process (kill_on_drop handles cleanup, but be explicit).
        let _ = child.kill().await;

        // Parse the response.
        Self::parse_rpc_response(&response_line)
    }

    /// Spawn `codex` via `tokio::process`, write `/status\n` to stdin,
    /// wait up to 3 seconds, read stdout, and parse for rate-limit percentages.
    /// Returns a snapshot with source `"pty"`.
    ///
    /// Note: without a real PTY crate, `codex` may detect non-TTY stdin and
    /// behave differently. This path is best-effort.
    async fn fetch_via_pty() -> anyhow::Result<ProviderSnapshot> {
        let pty_result = tokio::time::timeout(Duration::from_secs(3), Self::pty_exchange()).await;

        match pty_result {
            Ok(Ok(snap)) => Ok(snap),
            Ok(Err(e)) => Ok(ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "pty".into(),
                error: Some(format!("PTY exchange failed: {e}")),
                loaded_models: None,
            }),
            Err(_) => Ok(ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "pty".into(),
                error: Some("PTY timed out (3s)".into()),
                loaded_models: None,
            }),
        }
    }

    /// Inner async function for the PTY exchange with the codex process.
    /// Separated from `fetch_via_pty` so the timeout wrapper stays clean.
    async fn pty_exchange() -> anyhow::Result<ProviderSnapshot> {
        let mut child = tokio::process::Command::new("codex")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn `codex` for PTY fallback")?;

        let stdin = child
            .stdin
            .take()
            .context("no stdin on codex PTY process")?;
        let stdout = child
            .stdout
            .take()
            .context("no stdout on codex PTY process")?;

        let mut writer = tokio::io::BufWriter::new(stdin);
        let mut reader = BufReader::new(stdout);

        // Send /status command.
        writer
            .write_all(b"/status\n")
            .await
            .context("write /status")?;
        writer.flush().await.context("flush /status")?;

        // Give the process a moment to produce output.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Read all available output (up to 8KB).
        let mut output = String::new();
        let mut buf = [0u8; 4096];
        loop {
            match tokio::time::timeout(Duration::from_millis(500), reader.read(&mut buf)).await {
                Ok(Ok(0)) => break, // EOF
                Ok(Ok(n)) => output.push_str(&String::from_utf8_lossy(&buf[..n])),
                Ok(Err(_)) => break, // read error
                Err(_) => break,     // timeout — no more data
            }
        }

        // Kill the child process.
        let _ = child.kill().await;

        Ok(Self::parse_pty_output(&output))
    }

    /// Try JSON-RPC fallback first; if that returns an error snapshot with no
    /// data, try PTY as a last resort.
    async fn fetch_via_rpc_or_pty() -> anyhow::Result<ProviderSnapshot> {
        let rpc_snap = Self::fetch_via_rpc().await?;

        // If RPC succeeded (has data), use it directly.
        if rpc_snap.error.is_none() || rpc_snap.primary_pct.is_some() {
            return Ok(rpc_snap);
        }

        // RPC failed — try PTY as last resort.
        let pty_snap = Self::fetch_via_pty().await?;

        // If PTY produced useful data, use it.
        if pty_snap.error.is_none() || pty_snap.primary_pct.is_some() {
            return Ok(pty_snap);
        }

        // Both failed — return the RPC error (usually more informative).
        Ok(rpc_snap)
    }

    /// Parse a JSON response body from the Codex OAuth usage endpoint into a
    /// `ProviderSnapshot`. Separated from `fetch()` so it can be unit-tested
    /// without making network calls.
    fn parse_usage_response(body: &str) -> anyhow::Result<ProviderSnapshot> {
        let usage: UsageResponse = match serde_json::from_str(body) {
            Ok(u) => u,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex CLI".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: Some("weekly".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "oauth".into(),
                    error: Some(format!("failed to parse usage JSON: {e}")),
                    loaded_models: None,
                });
            }
        };

        // Map session.percent_used -> primary_pct (clamped to 0-100).
        let primary_pct = usage
            .session
            .as_ref()
            .and_then(|w| w.percent_used)
            .map(|p| p.round().clamp(0.0, 100.0) as u8);

        // Map weekly.percent_used -> secondary_pct (clamped to 0-100).
        let secondary_pct = usage
            .weekly
            .as_ref()
            .and_then(|w| w.percent_used)
            .map(|p| p.round().clamp(0.0, 100.0) as u8);

        // Parse session.reset_at if present.
        let reset_at: Option<DateTime<Utc>> = usage
            .session
            .as_ref()
            .and_then(|w| w.reset_at.as_deref())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(ProviderSnapshot {
            provider: "codex".into(),
            display_name: "Codex CLI".into(),
            primary_pct,
            secondary_pct,
            primary_label: Some("session".into()),
            secondary_label: Some("weekly".into()),
            tokens_used: None,
            cost_usd: None,
            reset_at,
            source: "oauth".into(),
            error: None,
            loaded_models: None,
        })
    }
}

#[async_trait::async_trait]
impl Provider for CodexProvider {
    fn name(&self) -> &str {
        "codex"
    }

    /// Returns `true` if `~/.codex/auth.json` exists OR `codex` binary is on PATH.
    fn detect(&self) -> bool {
        Self::has_credentials_file() || Self::binary_on_path()
    }

    /// Fetch current quota/usage from the Codex OAuth API, falling back to
    /// JSON-RPC if credentials are missing or expired.
    ///
    /// Reads credentials, warns if refresh is stale, calls the usage endpoint,
    /// and maps the response to a `ProviderSnapshot`. If OAuth fails (no creds
    /// or token rejected), tries JSON-RPC via `codex app-server`. On failure,
    /// returns a snapshot with the `error` field set rather than propagating.
    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot> {
        let creds = match Self::read_credentials() {
            Ok(c) => c,
            Err(_) => {
                // No OAuth credentials — try JSON-RPC, then PTY fallback.
                return Self::fetch_via_rpc_or_pty().await;
            }
        };

        // Check if token refresh is stale (>8 days).
        let stale_warning = Self::check_refresh_staleness(&creds);

        // Fetch usage data from the Codex OAuth API.
        let client = reqwest::Client::new();
        let resp = match client
            .get(USAGE_URL)
            .header("Authorization", format!("Bearer {}", creds.access_token))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex CLI".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: Some("weekly".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "oauth".into(),
                    error: Some(format!("HTTP request failed: {e}")),
                    loaded_models: None,
                });
            }
        };

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // Token rejected — try JSON-RPC, then PTY fallback.
            return Self::fetch_via_rpc_or_pty().await;
        }

        if !status.is_success() {
            return Ok(ProviderSnapshot {
                provider: "codex".into(),
                display_name: "Codex CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("session".into()),
                secondary_label: Some("weekly".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "oauth".into(),
                error: Some(format!("API returned HTTP {status}")),
                loaded_models: None,
            });
        }

        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "codex".into(),
                    display_name: "Codex CLI".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("session".into()),
                    secondary_label: Some("weekly".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "oauth".into(),
                    error: Some(format!("failed to read response body: {e}")),
                    loaded_models: None,
                });
            }
        };

        let mut snap = Self::parse_usage_response(&body)?;

        // Attach stale-refresh warning if applicable (non-fatal).
        if snap.error.is_none() {
            snap.error = stale_warning;
        }

        Ok(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_credentials() {
        let json = r#"{
            "access_token": "test-codex-token-123",
            "refresh_token": "refresh-codex-456",
            "last_refresh": "2026-03-01T12:00:00Z"
        }"#;
        let creds: CodexCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.access_token, "test-codex-token-123");
        assert_eq!(creds.refresh_token, Some("refresh-codex-456".to_string()));
        assert_eq!(creds.last_refresh, Some("2026-03-01T12:00:00Z".to_string()));
    }

    #[test]
    fn parse_minimal_credentials() {
        let json = r#"{"access_token": "tok"}"#;
        let creds: CodexCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.access_token, "tok");
        assert!(creds.refresh_token.is_none());
        assert!(creds.last_refresh.is_none());
    }

    #[test]
    fn parse_credentials_missing_token_fails() {
        let json = r#"{"refresh_token": "ref"}"#;
        let result: Result<CodexCredentials, _> = serde_json::from_str(json);
        assert!(result.is_err(), "should fail without access_token");
    }

    /// Tests `parse_usage_response` with a realistic mock JSON response,
    /// verifying correct mapping to `ProviderSnapshot` fields -- no network calls.
    #[test]
    fn test_codex_snapshot_parse() {
        let json = r#"{
            "session": {
                "percent_used": 42.7,
                "reset_at": "2026-03-07T20:00:00Z"
            },
            "weekly": {
                "percent_used": 61.3
            }
        }"#;

        let snap = CodexProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "codex");
        assert_eq!(snap.display_name, "Codex CLI");
        assert_eq!(snap.source, "oauth");
        assert!(snap.error.is_none(), "no error expected: {:?}", snap.error);

        // session.percent_used (42.7) -> rounds to 43
        assert_eq!(snap.primary_pct, Some(43));
        assert_eq!(snap.primary_label.as_deref(), Some("session"));

        // weekly.percent_used (61.3) -> rounds to 61
        assert_eq!(snap.secondary_pct, Some(61));
        assert_eq!(snap.secondary_label.as_deref(), Some("weekly"));

        // session.reset_at parsed as DateTime<Utc>
        let reset = snap.reset_at.expect("reset_at should be Some");
        assert_eq!(
            reset,
            "2026-03-07T20:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn test_codex_snapshot_parse_empty_response() {
        // Minimal valid JSON with no windows -- all percentages should be None.
        let json = r#"{}"#;
        let snap = CodexProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "codex");
        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
        assert_eq!(snap.reset_at, None);
    }

    #[test]
    fn test_codex_snapshot_parse_malformed_json() {
        let json = "not json at all";
        let snap = CodexProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "codex");
        assert!(
            snap.error.is_some(),
            "malformed JSON should produce error field"
        );
        assert!(snap.error.unwrap().contains("failed to parse usage JSON"));
    }

    #[test]
    fn test_codex_snapshot_parse_clamping() {
        // percent_used > 100 should clamp to 100.
        let json = r#"{
            "session": { "percent_used": 150.0 },
            "weekly": { "percent_used": -5.0 }
        }"#;
        let snap = CodexProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.primary_pct, Some(100));
        assert_eq!(snap.secondary_pct, Some(0));
    }

    #[test]
    fn test_codex_snapshot_parse_null_windows() {
        // Explicit null values for windows.
        let json = r#"{
            "session": null,
            "weekly": null
        }"#;
        let snap = CodexProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "codex");
        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_codex_snapshot_parse_session_only() {
        let json = r#"{
            "session": { "percent_used": 75.0 }
        }"#;
        let snap = CodexProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.primary_pct, Some(75));
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_refresh_staleness_fresh() {
        let creds = CodexCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            last_refresh: Some(Utc::now().to_rfc3339()),
        };
        assert!(
            CodexProvider::check_refresh_staleness(&creds).is_none(),
            "fresh token should not warn"
        );
    }

    #[test]
    fn test_refresh_staleness_stale() {
        let old = Utc::now() - chrono::Duration::days(10);
        let creds = CodexCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            last_refresh: Some(old.to_rfc3339()),
        };
        let warning = CodexProvider::check_refresh_staleness(&creds);
        assert!(warning.is_some(), "stale token should warn");
        assert!(warning.unwrap().contains("days ago"));
    }

    #[test]
    fn test_refresh_staleness_no_field() {
        let creds = CodexCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            last_refresh: None,
        };
        assert!(
            CodexProvider::check_refresh_staleness(&creds).is_none(),
            "missing last_refresh should not warn"
        );
    }

    // -----------------------------------------------------------------------
    // JSON-RPC parse_rpc_response tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rpc_parse_valid_windows() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "windows": [
                    {"type": "session", "percentUsed": 42.7, "resetAt": "2026-03-07T20:00:00Z"},
                    {"type": "weekly", "percentUsed": 61.3}
                ]
            }
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert_eq!(snap.provider, "codex");
        assert_eq!(snap.display_name, "Codex CLI");
        assert_eq!(snap.source, "rpc");
        assert!(snap.error.is_none(), "no error expected: {:?}", snap.error);

        assert_eq!(snap.primary_pct, Some(43));
        assert_eq!(snap.primary_label.as_deref(), Some("session"));

        assert_eq!(snap.secondary_pct, Some(61));
        assert_eq!(snap.secondary_label.as_deref(), Some("weekly"));

        let reset = snap.reset_at.expect("reset_at should be Some");
        assert_eq!(
            reset,
            "2026-03-07T20:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn test_rpc_parse_empty_windows() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": { "windows": [] }
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert_eq!(snap.source, "rpc");
        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
        assert_eq!(snap.reset_at, None);
    }

    #[test]
    fn test_rpc_parse_session_only() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "windows": [
                    {"type": "session", "percentUsed": 88.0}
                ]
            }
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert_eq!(snap.primary_pct, Some(88));
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_rpc_parse_clamping() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "windows": [
                    {"type": "session", "percentUsed": 200.0},
                    {"type": "weekly", "percentUsed": -10.0}
                ]
            }
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert_eq!(snap.primary_pct, Some(100));
        assert_eq!(snap.secondary_pct, Some(0));
    }

    #[test]
    fn test_rpc_parse_malformed_json() {
        let snap = CodexProvider::parse_rpc_response("not json at all").unwrap();

        assert_eq!(snap.source, "rpc");
        assert!(snap.error.is_some());
        assert!(snap.error.unwrap().contains("failed to parse RPC JSON"));
    }

    #[test]
    fn test_rpc_parse_rpc_error() {
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "error": {"code": -32601, "message": "method not found"}
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert_eq!(snap.source, "rpc");
        assert!(snap.error.is_some());
        assert!(snap.error.unwrap().contains("RPC error"));
    }

    #[test]
    fn test_rpc_parse_missing_result() {
        let json = r#"{"jsonrpc": "2.0", "id": 2}"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert_eq!(snap.source, "rpc");
        assert!(snap.error.is_some());
        assert!(snap.error.unwrap().contains("missing result"));
    }

    #[test]
    fn test_rpc_parse_snake_case_fields() {
        // Verify that snake_case aliases also work (percent_used, reset_at).
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "windows": [
                    {"type": "session", "percent_used": 55.5, "reset_at": "2026-03-08T12:00:00Z"},
                    {"type": "weekly", "percent_used": 30.0}
                ]
            }
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(56));
        assert_eq!(snap.secondary_pct, Some(30));
        assert!(snap.reset_at.is_some());
    }

    #[test]
    fn test_rpc_parse_with_credits_field() {
        // The response may include a `credits` field we don't use — should not error.
        let json = r#"{
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "windows": [
                    {"type": "session", "percentUsed": 10.0}
                ],
                "credits": {"remaining": 500}
            }
        }"#;

        let snap = CodexProvider::parse_rpc_response(json).unwrap();

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(10));
    }

    // -----------------------------------------------------------------------
    // PTY parse_pty_output tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_pty_parse_valid_output() {
        let output = "Codex CLI v1.2.3\n5h limit: 42%\nWeekly limit: 61%\nTokens: 1234\n";
        let snap = CodexProvider::parse_pty_output(output);

        assert_eq!(snap.provider, "codex");
        assert_eq!(snap.display_name, "Codex CLI");
        assert_eq!(snap.source, "pty");
        assert!(snap.error.is_none(), "no error expected: {:?}", snap.error);

        assert_eq!(snap.primary_pct, Some(42));
        assert_eq!(snap.primary_label.as_deref(), Some("session"));

        assert_eq!(snap.secondary_pct, Some(61));
        assert_eq!(snap.secondary_label.as_deref(), Some("weekly"));
    }

    #[test]
    fn test_pty_parse_session_only() {
        let output = "5h limit: 75%\n";
        let snap = CodexProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(75));
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_pty_parse_weekly_only() {
        let output = "Weekly limit: 30%\n";
        let snap = CodexProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, Some(30));
    }

    #[test]
    fn test_pty_parse_no_percentages() {
        let output = "Codex CLI v1.2.3\nReady.\n";
        let snap = CodexProvider::parse_pty_output(output);

        assert_eq!(snap.source, "pty");
        assert!(snap.error.is_some());
        assert!(
            snap.error.unwrap().contains("no rate-limit percentages"),
            "error should mention missing percentages"
        );
    }

    #[test]
    fn test_pty_parse_empty_output() {
        let snap = CodexProvider::parse_pty_output("");

        assert_eq!(snap.source, "pty");
        assert!(snap.error.is_some());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_pty_parse_clamping() {
        let output = "5h limit: 150%\nWeekly limit: 0%\n";
        let snap = CodexProvider::parse_pty_output(output);

        assert_eq!(snap.primary_pct, Some(100));
        assert_eq!(snap.secondary_pct, Some(0));
    }

    #[test]
    fn test_pty_parse_with_ansi_noise() {
        // Terminal output may have ANSI escape codes interspersed — the anchor
        // search still works as long as the text between codes is intact.
        let output = "\x1b[1mStatus\x1b[0m\n5h limit: 88%\nWeekly limit: 12%\n";
        let snap = CodexProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(88));
        assert_eq!(snap.secondary_pct, Some(12));
    }

    #[test]
    fn test_extract_pct_after_basic() {
        assert_eq!(
            CodexProvider::extract_pct_after("5h limit: 42%", "5h limit:"),
            Some(42)
        );
        assert_eq!(
            CodexProvider::extract_pct_after("Weekly limit: 100%", "Weekly limit:"),
            Some(100)
        );
    }

    #[test]
    fn test_extract_pct_after_missing_anchor() {
        assert_eq!(
            CodexProvider::extract_pct_after("no match here", "5h limit:"),
            None
        );
    }

    #[test]
    fn test_extract_pct_after_no_digits() {
        assert_eq!(
            CodexProvider::extract_pct_after("5h limit: abc%", "5h limit:"),
            None
        );
    }

    #[test]
    fn test_detect_missing_creds_and_binary() {
        // Set HOME to a temp dir so credentials file won't resolve.
        let td = tempfile::tempdir().unwrap();
        let saved = std::env::var("HOME").ok();
        std::env::set_var("HOME", td.path());

        let provider = CodexProvider::new();
        // detect() may still return true if `codex` is on PATH.
        // We can at least verify it doesn't panic.
        let _ = provider.detect();

        // Restore original HOME.
        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn test_detect_with_creds_file() {
        let td = tempfile::tempdir().unwrap();
        let codex_dir = td.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(codex_dir.join("auth.json"), r#"{"access_token":"test"}"#).unwrap();

        let saved = std::env::var("HOME").ok();
        std::env::set_var("HOME", td.path());

        let provider = CodexProvider::new();
        assert!(
            provider.detect(),
            "detect() should return true when auth.json exists"
        );

        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
