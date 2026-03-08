use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{Provider, ProviderSnapshot};

/// Base URL for the Claude OAuth usage endpoint.
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Required beta header value for the OAuth usage endpoint.
const BETA_HEADER: &str = "oauth-2025-04-20";

// ---------------------------------------------------------------------------
// Claude OAuth usage API response types
// ---------------------------------------------------------------------------

/// Top-level response from `GET /api/oauth/usage`.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UsageResponse {
    #[serde(default)]
    pub five_hour: Option<UsageWindow>,
    #[serde(default)]
    pub seven_day: Option<UsageWindow>,
    #[serde(default)]
    #[allow(dead_code)] // exposed for future use (e.g. display tier in providers table)
    pub rate_limit_tier: Option<String>,
}

/// A single rate-limit window inside the usage response.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UsageWindow {
    /// Actual field name from the API. Alias handles older `percent_used` shape.
    #[serde(alias = "percent_used")]
    pub utilization: Option<f64>,
    /// Actual field name from the API. Alias handles older `reset_at` shape.
    #[serde(alias = "reset_at", default)]
    pub resets_at: Option<String>,
}

/// OAuth credentials as stored by Claude Code at `~/.claude/.credentials.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCredentials {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// Provider implementation for Claude Code OAuth quota monitoring.
pub struct ClaudeProvider;

impl ClaudeProvider {
    pub fn new() -> Self {
        Self
    }

    /// Path to the Claude Code credentials file: `~/.claude/.credentials.json`.
    fn credentials_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".claude").join(".credentials.json"))
    }

    /// Read and parse credentials — tries file first, then macOS Keychain.
    ///
    /// Keychain JSON shape: `{"claudeAiOauth":{"accessToken":"...","refreshToken":"..."}}`
    /// File JSON shape:     `{"access_token":"...","refresh_token":"..."}`
    fn read_credentials() -> anyhow::Result<ClaudeCredentials> {
        // Try file first.
        if let Some(path) = Self::credentials_path() {
            if path.exists() {
                let json = std::fs::read_to_string(&path).with_context(|| {
                    format!("cannot read Claude credentials: {}", path.display())
                })?;
                return serde_json::from_str(&json).with_context(|| {
                    format!("malformed Claude credentials at {}", path.display())
                });
            }
        }
        // Fall back to macOS Keychain.
        #[cfg(target_os = "macos")]
        {
            if let Ok(raw) = Self::read_keychain_credentials() {
                return Ok(raw);
            }
        }
        anyhow::bail!("no Claude credentials found (checked file and Keychain)")
    }

    /// Extract credentials from macOS Keychain entry `Claude Code-credentials`.
    ///
    /// The Keychain stores JSON as `{"claudeAiOauth":{"accessToken":"...","refreshToken":"..."}}`.
    #[cfg(target_os = "macos")]
    fn read_keychain_credentials() -> anyhow::Result<ClaudeCredentials> {
        let output = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .output()
            .context("failed to run `security` CLI")?;
        anyhow::ensure!(output.status.success(), "Keychain lookup failed");
        let raw = String::from_utf8(output.stdout).context("Keychain output not UTF-8")?;
        let raw = raw.trim();
        // Try flat shape first (access_token), then wrapped claudeAiOauth shape.
        if let Ok(c) = serde_json::from_str::<ClaudeCredentials>(raw) {
            return Ok(c);
        }
        // Wrapped shape: {"claudeAiOauth": {"accessToken": "...", "refreshToken": "...", "expiresAt": <epoch_ms>}}
        // expiresAt is epoch-milliseconds (u64), not an ISO string.
        #[derive(serde::Deserialize)]
        struct KeychainInner {
            #[serde(rename = "accessToken")]
            access_token: String,
            #[serde(rename = "refreshToken", default)]
            refresh_token: Option<String>,
            /// Epoch milliseconds — convert to ISO 8601 for the rest of the provider code.
            #[serde(rename = "expiresAt", default)]
            expires_at_ms: Option<u64>,
        }
        #[derive(serde::Deserialize)]
        struct KeychainWrapper {
            #[serde(rename = "claudeAiOauth")]
            claude_ai_oauth: KeychainInner,
        }
        let w: KeychainWrapper =
            serde_json::from_str(raw).context("unrecognised Keychain JSON shape")?;
        // Convert epoch_ms → ISO 8601 string so the expiry check in fetch() works.
        let expires_at = w.claude_ai_oauth.expires_at_ms.and_then(|ms| {
            let secs = (ms / 1000) as i64;
            chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
        });
        Ok(ClaudeCredentials {
            access_token: w.claude_ai_oauth.access_token,
            refresh_token: w.claude_ai_oauth.refresh_token,
            expires_at,
        })
    }

    /// Check if Claude Code credentials exist on disk.
    fn has_credentials_file() -> bool {
        Self::credentials_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// On macOS, check if credentials are stored in Keychain via
    /// `security find-generic-password -s 'Claude Code-credentials'`.
    #[cfg(target_os = "macos")]
    fn has_keychain_entry() -> bool {
        std::process::Command::new("security")
            .args(["find-generic-password", "-s", "Claude Code-credentials"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "macos"))]
    fn has_keychain_entry() -> bool {
        false
    }

    /// Parse a JSON response body from the Claude OAuth usage endpoint into a
    /// `ProviderSnapshot`. Separated from `fetch()` so it can be unit-tested
    /// without making network calls.
    fn parse_usage_response(body: &str) -> anyhow::Result<ProviderSnapshot> {
        let usage: UsageResponse = match serde_json::from_str(body) {
            Ok(u) => u,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "claude".into(),
                    display_name: "Claude Code".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("5h".into()),
                    secondary_label: Some("7d".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "oauth".into(),
                    error: Some(format!("failed to parse usage JSON: {e}")),
                    loaded_models: None,
                });
            }
        };

        // Map five_hour.utilization → primary_pct (clamped to 0-100).
        let primary_pct = usage
            .five_hour
            .as_ref()
            .and_then(|w| w.utilization)
            .map(|p| p.round().clamp(0.0, 100.0) as u8);

        // Map seven_day.utilization → secondary_pct (clamped to 0-100).
        let secondary_pct = usage
            .seven_day
            .as_ref()
            .and_then(|w| w.utilization)
            .map(|p| p.round().clamp(0.0, 100.0) as u8);

        // Parse five_hour.resets_at if present.
        let reset_at: Option<DateTime<Utc>> = usage
            .five_hour
            .as_ref()
            .and_then(|w| w.resets_at.as_deref())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        Ok(ProviderSnapshot {
            provider: "claude".into(),
            display_name: "Claude Code".into(),
            primary_pct,
            secondary_pct,
            primary_label: Some("5h".into()),
            secondary_label: Some("7d".into()),
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
impl Provider for ClaudeProvider {
    fn name(&self) -> &str {
        "claude"
    }

    /// Returns `true` if credentials file exists on disk, or if the macOS
    /// Keychain contains a `Claude Code-credentials` entry.
    fn detect(&self) -> bool {
        Self::has_credentials_file() || Self::has_keychain_entry()
    }

    /// Fetch current quota/usage from the Claude OAuth API.
    ///
    /// Reads credentials, calls the usage endpoint, and maps the response
    /// to a `ProviderSnapshot`. On failure, returns a snapshot with the
    /// `error` field set rather than propagating.
    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot> {
        let creds = match Self::read_credentials() {
            Ok(c) => c,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "claude".into(),
                    display_name: "Claude Code".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: None,
                    secondary_label: None,
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "oauth".into(),
                    error: Some(format!("credentials unreadable: {e}")),
                    loaded_models: None,
                });
            }
        };

        // Check if token is expired (best-effort, expires_at is optional).
        if let Some(ref expires_at) = creds.expires_at {
            if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(expires_at) {
                if exp < chrono::Utc::now() {
                    return Ok(ProviderSnapshot {
                        provider: "claude".into(),
                        display_name: "Claude Code".into(),
                        primary_pct: None,
                        secondary_pct: None,
                        primary_label: None,
                        secondary_label: None,
                        tokens_used: None,
                        cost_usd: None,
                        reset_at: None,
                        source: "oauth".into(),
                        error: Some("token expired".into()),
                        loaded_models: None,
                    });
                }
            }
        }

        // Fetch usage data from the Claude OAuth API.
        let client = reqwest::Client::new();
        let resp = match client
            .get(USAGE_URL)
            .header("Authorization", format!("Bearer {}", creds.access_token))
            .header("anthropic-beta", BETA_HEADER)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(ProviderSnapshot {
                    provider: "claude".into(),
                    display_name: "Claude Code".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("5h".into()),
                    secondary_label: Some("7d".into()),
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
            return Ok(ProviderSnapshot {
                provider: "claude".into(),
                display_name: "Claude Code".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: None,
                secondary_label: None,
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "oauth".into(),
                error: Some(format!("token rejected (HTTP {status})")),
                loaded_models: None,
            });
        }

        if !status.is_success() {
            return Ok(ProviderSnapshot {
                provider: "claude".into(),
                display_name: "Claude Code".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("5h".into()),
                secondary_label: Some("7d".into()),
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
                    provider: "claude".into(),
                    display_name: "Claude Code".into(),
                    primary_pct: None,
                    secondary_pct: None,
                    primary_label: Some("5h".into()),
                    secondary_label: Some("7d".into()),
                    tokens_used: None,
                    cost_usd: None,
                    reset_at: None,
                    source: "oauth".into(),
                    error: Some(format!("failed to read response body: {e}")),
                    loaded_models: None,
                });
            }
        };

        Self::parse_usage_response(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_missing_creds() {
        // Set HOME to a temp dir so neither credentials file nor Keychain
        // will resolve (Keychain check is a no-op in test context on non-macOS,
        // and even on macOS won't find the item under a fake home).
        let td = tempfile::tempdir().unwrap();
        let saved = std::env::var("HOME").ok();
        std::env::set_var("HOME", td.path());

        let provider = ClaudeProvider::new();
        assert!(
            !provider.detect(),
            "detect() should return false when credentials are missing"
        );

        // Restore original HOME.
        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    #[test]
    fn parse_valid_credentials() {
        let json = r#"{
            "access_token": "test-token-123",
            "refresh_token": "refresh-456",
            "expires_at": "2026-12-31T23:59:59Z"
        }"#;
        let creds: ClaudeCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.access_token, "test-token-123");
        assert_eq!(creds.refresh_token, Some("refresh-456".to_string()));
        assert_eq!(creds.expires_at, Some("2026-12-31T23:59:59Z".to_string()));
    }

    #[test]
    fn parse_minimal_credentials() {
        let json = r#"{"access_token": "tok"}"#;
        let creds: ClaudeCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.access_token, "tok");
        assert!(creds.refresh_token.is_none());
        assert!(creds.expires_at.is_none());
    }

    #[test]
    fn parse_credentials_missing_token_fails() {
        let json = r#"{"refresh_token": "ref"}"#;
        let result: Result<ClaudeCredentials, _> = serde_json::from_str(json);
        assert!(result.is_err(), "should fail without access_token");
    }

    /// Tests `parse_usage_response` with a realistic mock JSON response,
    /// verifying correct mapping to `ProviderSnapshot` fields — no network calls.
    #[test]
    fn test_claude_snapshot_parse() {
        let json = r#"{
            "five_hour": {
                "percent_used": 57.3,
                "reset_at": "2026-03-07T18:00:00Z"
            },
            "seven_day": {
                "percent_used": 38.9
            },
            "seven_day_sonnet": {
                "percent_used": 22.1
            },
            "seven_day_opus": {
                "percent_used": 5.0
            },
            "rate_limit_tier": "Max"
        }"#;

        let snap = ClaudeProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "claude");
        assert_eq!(snap.display_name, "Claude Code");
        assert_eq!(snap.source, "oauth");
        assert!(snap.error.is_none(), "no error expected: {:?}", snap.error);

        // five_hour.percent_used (57.3) → rounds to 57
        assert_eq!(snap.primary_pct, Some(57));
        assert_eq!(snap.primary_label.as_deref(), Some("5h"));

        // seven_day.percent_used (38.9) → rounds to 39
        assert_eq!(snap.secondary_pct, Some(39));
        assert_eq!(snap.secondary_label.as_deref(), Some("7d"));

        // five_hour.reset_at parsed as DateTime<Utc>
        let reset = snap.reset_at.expect("reset_at should be Some");
        assert_eq!(
            reset,
            "2026-03-07T18:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn test_claude_snapshot_parse_empty_response() {
        // Minimal valid JSON with no windows — all percentages should be None.
        let json = r#"{}"#;
        let snap = ClaudeProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "claude");
        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
        assert_eq!(snap.reset_at, None);
    }

    #[test]
    fn test_claude_snapshot_parse_malformed_json() {
        let json = "not json at all";
        let snap = ClaudeProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.provider, "claude");
        assert!(
            snap.error.is_some(),
            "malformed JSON should produce error field"
        );
        assert!(snap.error.unwrap().contains("failed to parse usage JSON"));
    }

    #[test]
    fn test_claude_snapshot_parse_clamping() {
        // percent_used > 100 should clamp to 100.
        let json = r#"{
            "five_hour": { "percent_used": 150.0 },
            "seven_day": { "percent_used": -5.0 }
        }"#;
        let snap = ClaudeProvider::parse_usage_response(json).unwrap();

        assert_eq!(snap.primary_pct, Some(100));
        assert_eq!(snap.secondary_pct, Some(0));
    }
}
