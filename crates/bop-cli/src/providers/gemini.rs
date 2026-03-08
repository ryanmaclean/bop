use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

use super::{Provider, ProviderSnapshot};

/// Google OAuth2 token endpoint for refreshing access tokens.
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Google Cloud Code API endpoint for loading code assist project info.
const LOAD_CODE_ASSIST_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";

/// Google Cloud Code API endpoint for retrieving user quota.
const RETRIEVE_USER_QUOTA_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";

// ---------------------------------------------------------------------------
// Gemini OAuth credentials and client info
// ---------------------------------------------------------------------------

/// OAuth credentials as stored by Gemini CLI at `~/.gemini/oauth_creds.json`.
///
/// Shape: `{"access_token":"...","refresh_token":"...","expiry_date":N,"id_token":"..."}`
/// where `expiry_date` is epoch-seconds (or epoch-milliseconds — we handle both).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCredentials {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Token expiry as epoch timestamp. May be seconds or milliseconds.
    #[serde(default)]
    pub expiry_date: Option<f64>,
    #[serde(default)]
    pub id_token: Option<String>,
}

/// OAuth client ID and secret extracted from the Gemini CLI npm package.
#[derive(Debug, Clone)]
pub struct OAuthClientInfo {
    pub client_id: String,
    pub client_secret: String,
}

/// Response from the Google OAuth2 token refresh endpoint.
#[derive(Debug, Clone, Deserialize)]
struct TokenRefreshResponse {
    pub access_token: String,
    #[serde(default)]
    #[allow(dead_code)] // Exposed for future use (e.g. updating cached expiry).
    pub expires_in: Option<u64>,
}

// ---------------------------------------------------------------------------
// Gemini Cloud Code API response types
// ---------------------------------------------------------------------------

/// Response from `POST /v1internal:loadCodeAssist`.
///
/// We only need the `cloudaicompanionProject` field to pass to
/// `retrieveUserQuota`. The response may contain many other fields.
#[derive(Debug, Clone, Deserialize)]
struct LoadCodeAssistResponse {
    #[serde(rename = "cloudaicompanionProject", default)]
    pub cloudai_companion_project: Option<String>,
}

/// A single quota entry from the `retrieveUserQuota` response array.
///
/// Shape: `{"remainingFraction":0.62,"resetTime":"2026-03-08T...","modelId":"gemini-2.5-pro"}`
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct QuotaEntry {
    #[serde(rename = "remainingFraction", default)]
    pub remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime", default)]
    pub reset_time: Option<String>,
    #[serde(rename = "modelId", default)]
    pub model_id: Option<String>,
}

/// Top-level response from `POST /v1internal:retrieveUserQuota`.
///
/// The response is a JSON object with a `quotas` array field (or the array
/// may be at the top level). We handle both shapes.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum QuotaResponse {
    /// Object with a `quotas` array field.
    Object {
        #[serde(default)]
        quotas: Vec<QuotaEntry>,
    },
    /// Direct array of quota entries.
    Array(Vec<QuotaEntry>),
}

impl QuotaResponse {
    fn into_entries(self) -> Vec<QuotaEntry> {
        match self {
            QuotaResponse::Object { quotas } => quotas,
            QuotaResponse::Array(entries) => entries,
        }
    }
}

/// Provider implementation for Gemini CLI OAuth quota monitoring.
pub struct GeminiProvider;

#[allow(dead_code)] // Methods used by Provider::fetch() and tests; registered in subtask-3-1.
impl GeminiProvider {
    pub fn new() -> Self {
        Self
    }

    /// Path to the Gemini credentials file: `~/.gemini/oauth_creds.json`.
    fn credentials_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".gemini").join("oauth_creds.json"))
    }

    /// Check if the `gemini` binary is on PATH.
    fn binary_on_path() -> bool {
        std::process::Command::new("which")
            .arg("gemini")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if Gemini credentials exist on disk.
    fn has_credentials_file() -> bool {
        Self::credentials_path()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Read and parse credentials from `~/.gemini/oauth_creds.json`.
    fn read_credentials() -> anyhow::Result<GeminiCredentials> {
        let path = Self::credentials_path()
            .context("cannot determine home directory for Gemini credentials")?;
        if !path.exists() {
            anyhow::bail!("no Gemini credentials found at {}", path.display());
        }
        let json = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read Gemini credentials: {}", path.display()))?;
        serde_json::from_str(&json)
            .with_context(|| format!("malformed Gemini credentials at {}", path.display()))
    }

    /// Find the full path to the `gemini` binary via `which`.
    /// Returns `None` if the binary is not on PATH.
    fn find_gemini_binary() -> Option<PathBuf> {
        let output = std::process::Command::new("which")
            .arg("gemini")
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let path_str = String::from_utf8(output.stdout).ok()?;
        let path_str = path_str.trim();
        if path_str.is_empty() {
            return None;
        }
        // Resolve symlinks to get the real path.
        let path = PathBuf::from(path_str);
        std::fs::canonicalize(&path).ok().or(Some(path))
    }

    /// Extract a quoted value after a given variable name from JavaScript source.
    ///
    /// Looks for patterns like:
    /// - `OAUTH_CLIENT_ID = "value"` or `OAUTH_CLIENT_ID = 'value'`
    /// - Handles optional whitespace around `=`.
    ///
    /// Returns the value between quotes, or `None` if not found.
    fn extract_js_string_value(js_source: &str, var_name: &str) -> Option<String> {
        let idx = js_source.find(var_name)?;
        let after_name = &js_source[idx + var_name.len()..];

        // Skip whitespace and `=`.
        let after_eq = after_name.trim_start();
        let after_eq = after_eq.strip_prefix('=')?;
        let after_eq = after_eq.trim_start();

        // Find the opening quote and strip it.
        let (quote_char, after_quote) = if let Some(rest) = after_eq.strip_prefix('"') {
            ('"', rest)
        } else if let Some(rest) = after_eq.strip_prefix('\'') {
            ('\'', rest)
        } else {
            return None;
        };

        // Read until matching closing quote.
        let end_idx = after_quote.find(quote_char)?;
        let value = &after_quote[..end_idx];

        if value.is_empty() {
            return None;
        }

        Some(value.to_string())
    }

    /// Locate the `oauth2.js` file in the Gemini CLI npm package and extract
    /// the `OAUTH_CLIENT_ID` and `OAUTH_CLIENT_SECRET` values.
    ///
    /// Strategy: from the resolved binary path, walk up parent directories
    /// looking for `node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js`.
    fn extract_oauth_client_from_path(binary_path: &std::path::Path) -> Option<OAuthClientInfo> {
        // Walk up parent directories from the binary location.
        let mut dir = binary_path.parent()?;
        for _ in 0..10 {
            let oauth2_path = dir
                .join("node_modules")
                .join("@google")
                .join("gemini-cli-core")
                .join("dist")
                .join("src")
                .join("code_assist")
                .join("oauth2.js");
            if oauth2_path.exists() {
                let source = std::fs::read_to_string(&oauth2_path).ok()?;
                return Self::extract_oauth_client_from_source(&source);
            }
            // Also check lib/node_modules (global npm installs).
            let global_oauth2_path = dir
                .join("lib")
                .join("node_modules")
                .join("@google")
                .join("gemini-cli-core")
                .join("dist")
                .join("src")
                .join("code_assist")
                .join("oauth2.js");
            if global_oauth2_path.exists() {
                let source = std::fs::read_to_string(&global_oauth2_path).ok()?;
                return Self::extract_oauth_client_from_source(&source);
            }
            dir = dir.parent()?;
        }
        None
    }

    /// Extract OAuth client ID and secret from the contents of `oauth2.js`.
    fn extract_oauth_client_from_source(source: &str) -> Option<OAuthClientInfo> {
        let client_id = Self::extract_js_string_value(source, "OAUTH_CLIENT_ID")?;
        let client_secret = Self::extract_js_string_value(source, "OAUTH_CLIENT_SECRET")?;
        Some(OAuthClientInfo {
            client_id,
            client_secret,
        })
    }

    /// Find the gemini binary and extract OAuth client credentials from its
    /// npm package. Returns `None` if the binary is not found or the oauth2.js
    /// file cannot be located/parsed.
    fn extract_oauth_client() -> Option<OAuthClientInfo> {
        let binary_path = Self::find_gemini_binary()?;
        Self::extract_oauth_client_from_path(&binary_path)
    }

    /// Call `loadCodeAssist` to get the `cloudaicompanionProject` string.
    ///
    /// Returns `Ok(Some(project))` on success, `Ok(None)` if the project ID
    /// cannot be determined (network error, auth error, missing field).
    async fn load_code_assist(client: &reqwest::Client, access_token: &str) -> Option<String> {
        let body = serde_json::json!({
            "metadata": {
                "ideType": "GEMINI_CLI",
                "pluginType": "GEMINI"
            }
        });
        let resp = client
            .post(LOAD_CODE_ASSIST_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&body)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let parsed: LoadCodeAssistResponse = resp.json().await.ok()?;
        parsed.cloudai_companion_project.filter(|s| !s.is_empty())
    }

    /// Call `retrieveUserQuota` to get the quota entries for this user.
    ///
    /// `project` should be the `cloudaicompanionProject` from `loadCodeAssist`,
    /// or an empty string if that call failed.
    async fn retrieve_user_quota(
        client: &reqwest::Client,
        access_token: &str,
        project: &str,
    ) -> anyhow::Result<Vec<QuotaEntry>> {
        let body = serde_json::json!({
            "project": project
        });
        let resp = client
            .post(RETRIEVE_USER_QUOTA_URL)
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&body)
            .send()
            .await
            .context("failed to POST retrieveUserQuota")?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("retrieveUserQuota failed (HTTP {status}): {body_text}");
        }

        let quota_resp: QuotaResponse = resp
            .json()
            .await
            .context("failed to parse retrieveUserQuota response")?;

        Ok(quota_resp.into_entries())
    }

    /// Parse a list of quota entries into a `ProviderSnapshot`.
    ///
    /// Separated from `fetch()` so it can be unit-tested without network calls.
    ///
    /// Logic:
    /// - Pro models: `model_id` contains "pro" (case-insensitive)
    /// - Flash models: `model_id` contains "flash" (case-insensitive)
    /// - `primary_pct` = `(1 - min(remainingFraction for Pro models)) * 100`
    /// - `secondary_pct` = same for Flash models
    /// - `reset_at` = earliest `resetTime` across all entries
    fn parse_quota_response(entries: &[QuotaEntry]) -> ProviderSnapshot {
        let mut pro_min: Option<f64> = None;
        let mut flash_min: Option<f64> = None;
        let mut earliest_reset: Option<DateTime<Utc>> = None;

        for entry in entries {
            let model_lower = entry.model_id.as_deref().unwrap_or("").to_lowercase();

            let remaining = match entry.remaining_fraction {
                Some(f) => f,
                None => continue,
            };

            if model_lower.contains("pro") {
                pro_min = Some(match pro_min {
                    Some(current) => current.min(remaining),
                    None => remaining,
                });
            } else if model_lower.contains("flash") {
                flash_min = Some(match flash_min {
                    Some(current) => current.min(remaining),
                    None => remaining,
                });
            }

            // Track earliest reset time across all models.
            if let Some(ref reset_str) = entry.reset_time {
                if let Ok(dt) = DateTime::parse_from_rfc3339(reset_str) {
                    let dt_utc = dt.with_timezone(&Utc);
                    earliest_reset = Some(match earliest_reset {
                        Some(current) => current.min(dt_utc),
                        None => dt_utc,
                    });
                }
            }
        }

        // Convert remainingFraction to usage percentage: (1 - fraction) * 100
        let primary_pct = pro_min.map(|f| ((1.0 - f) * 100.0).round().clamp(0.0, 100.0) as u8);
        let secondary_pct = flash_min.map(|f| ((1.0 - f) * 100.0).round().clamp(0.0, 100.0) as u8);

        ProviderSnapshot {
            provider: "gemini".into(),
            display_name: "Gemini CLI".into(),
            primary_pct,
            secondary_pct,
            primary_label: Some("Pro".into()),
            secondary_label: Some("Flash".into()),
            tokens_used: None,
            cost_usd: None,
            reset_at: earliest_reset,
            source: "oauth".into(),
            error: None,
            loaded_models: None,
        }
    }

    /// Check whether the access token has expired based on `expiry_date`.
    ///
    /// `expiry_date` may be in seconds or milliseconds — if the value is
    /// unreasonably large (> year 3000 in seconds), we treat it as millis.
    fn is_token_expired(creds: &GeminiCredentials) -> bool {
        let expiry = match creds.expiry_date {
            Some(e) => e,
            None => return false, // No expiry info — assume valid.
        };

        let expiry_secs = if expiry > 32_503_680_000.0 {
            // Looks like milliseconds (> year 3000 in seconds).
            expiry / 1000.0
        } else {
            expiry
        };

        let now = Utc::now().timestamp() as f64;
        expiry_secs < now
    }

    /// Refresh the OAuth access token using the Google token endpoint.
    ///
    /// Requires the OAuth client ID/secret (from `extract_oauth_client()`) and
    /// the refresh token from credentials. Returns the new access token on
    /// success.
    async fn refresh_token_if_expired(
        creds: &GeminiCredentials,
        client_info: &OAuthClientInfo,
    ) -> anyhow::Result<String> {
        // If token is not expired, return the current one.
        if !Self::is_token_expired(creds) {
            return Ok(creds.access_token.clone());
        }

        let refresh_token = creds
            .refresh_token
            .as_deref()
            .context("token expired but no refresh_token available")?;

        let client = reqwest::Client::new();
        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", &client_info.client_id),
                ("client_secret", &client_info.client_secret),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .context("failed to POST token refresh request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("token refresh failed (HTTP {status}): {body}");
        }

        let token_resp: TokenRefreshResponse = resp
            .json()
            .await
            .context("failed to parse token refresh response")?;

        Ok(token_resp.access_token)
    }

    // -----------------------------------------------------------------------
    // PTY fallback helpers
    // -----------------------------------------------------------------------

    /// Extract a fraction `N/M` from a line, returning the usage percentage
    /// `(N * 100) / M`, clamped to 0–100.
    ///
    /// Looks for the first occurrence of a `digits/digits` pattern in the line.
    fn extract_fraction_pct(line: &str) -> Option<u8> {
        let slash_idx = line.find('/')?;

        // Walk backward from '/' to collect numerator digits.
        let before = &line[..slash_idx];
        let num_str: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if num_str.is_empty() {
            return None;
        }
        let used: u64 = num_str.parse().ok()?;

        // Walk forward from '/' to collect denominator digits.
        let after = &line[slash_idx + 1..];
        let den_str: String = after
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if den_str.is_empty() {
            return None;
        }
        let limit: u64 = den_str.parse().ok()?;

        if limit == 0 {
            return None;
        }

        let pct = ((used * 100) / limit).min(100) as u8;
        Some(pct)
    }

    /// Extract a direct percentage `NN%` from a line, returning the value
    /// clamped to 0–100.
    fn extract_direct_pct(line: &str) -> Option<u8> {
        let pct_idx = line.find('%')?;
        let before = &line[..pct_idx];
        let num_str: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if num_str.is_empty() {
            return None;
        }
        let pct: u32 = num_str.parse().ok()?;
        Some(pct.min(100) as u8)
    }

    /// Extract a usage percentage from a single line.
    ///
    /// Checks for:
    /// 1. Fraction pattern `N/M` → percentage = `(N * 100) / M`
    /// 2. Direct percentage `NN%`
    fn extract_line_pct(line: &str) -> Option<u8> {
        // Try fraction first (e.g., "42/50 requests")
        if let Some(pct) = Self::extract_fraction_pct(line) {
            return Some(pct);
        }
        // Try direct percentage (e.g., "76%")
        Self::extract_direct_pct(line)
    }

    /// Parse PTY screen output from `gemini /stats model` for model usage.
    ///
    /// Scans lines for model names ("pro" or "flash", case-insensitive) and
    /// extracts usage percentages from fractions (`N/M`) or direct
    /// percentages (`NN%`) on those lines.
    ///
    /// Returns a `ProviderSnapshot` with source `"pty"`. If no model data
    /// is found, the `error` field describes what was missing.
    fn parse_pty_output(output: &str) -> ProviderSnapshot {
        let mut pro_pct: Option<u8> = None;
        let mut flash_pct: Option<u8> = None;

        for line in output.lines() {
            let lower = line.to_lowercase();
            let is_pro = lower.contains("pro");
            let is_flash = lower.contains("flash");

            if !is_pro && !is_flash {
                continue;
            }

            if let Some(pct) = Self::extract_line_pct(line) {
                if is_pro && pro_pct.is_none() {
                    pro_pct = Some(pct);
                }
                if is_flash && flash_pct.is_none() {
                    flash_pct = Some(pct);
                }
            }
        }

        let error = if pro_pct.is_none() && flash_pct.is_none() {
            Some("no model usage data found in PTY output".into())
        } else {
            None
        };

        ProviderSnapshot {
            provider: "gemini".into(),
            display_name: "Gemini CLI".into(),
            primary_pct: pro_pct,
            secondary_pct: flash_pct,
            primary_label: Some("Pro".into()),
            secondary_label: Some("Flash".into()),
            tokens_used: None,
            cost_usd: None,
            reset_at: None,
            source: "pty".into(),
            error,
            loaded_models: None,
        }
    }

    /// Spawn `gemini` via `tokio::process`, write `/stats model\n` to stdin,
    /// wait up to 3 seconds, read stdout, and parse for model usage data.
    /// Returns a snapshot with source `"pty"`.
    ///
    /// Note: without a real PTY crate, `gemini` may detect non-TTY stdin and
    /// behave differently. This path is best-effort.
    async fn fetch_via_pty() -> anyhow::Result<ProviderSnapshot> {
        let pty_result = tokio::time::timeout(Duration::from_secs(3), Self::pty_exchange()).await;

        match pty_result {
            Ok(Ok(snap)) => Ok(snap),
            Ok(Err(e)) => Ok(ProviderSnapshot {
                provider: "gemini".into(),
                display_name: "Gemini CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("Pro".into()),
                secondary_label: Some("Flash".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "pty".into(),
                error: Some(format!("PTY exchange failed: {e}")),
                loaded_models: None,
            }),
            Err(_) => Ok(ProviderSnapshot {
                provider: "gemini".into(),
                display_name: "Gemini CLI".into(),
                primary_pct: None,
                secondary_pct: None,
                primary_label: Some("Pro".into()),
                secondary_label: Some("Flash".into()),
                tokens_used: None,
                cost_usd: None,
                reset_at: None,
                source: "pty".into(),
                error: Some("PTY timed out (3s)".into()),
                loaded_models: None,
            }),
        }
    }

    /// Inner async function for the PTY exchange with the gemini process.
    /// Separated from `fetch_via_pty` so the timeout wrapper stays clean.
    async fn pty_exchange() -> anyhow::Result<ProviderSnapshot> {
        let mut child = tokio::process::Command::new("gemini")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn `gemini` for PTY fallback")?;

        let stdin = child
            .stdin
            .take()
            .context("no stdin on gemini PTY process")?;
        let stdout = child
            .stdout
            .take()
            .context("no stdout on gemini PTY process")?;

        let mut writer = tokio::io::BufWriter::new(stdin);
        let mut reader = BufReader::new(stdout);

        // Send /stats model command.
        writer
            .write_all(b"/stats model\n")
            .await
            .context("write /stats model")?;
        writer.flush().await.context("flush /stats model")?;

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
}

#[async_trait::async_trait]
impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    /// Returns `true` if `~/.gemini/oauth_creds.json` exists OR `gemini`
    /// binary is on PATH.
    fn detect(&self) -> bool {
        Self::has_credentials_file() || Self::binary_on_path()
    }

    /// Fetch current quota/usage from the Gemini API, falling back to PTY
    /// if OAuth fails.
    ///
    /// Flow: read credentials → extract OAuth client → refresh token →
    /// call loadCodeAssist (best-effort) → call retrieveUserQuota → parse.
    /// If any OAuth step fails, falls back to PTY via `gemini /stats model`.
    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot> {
        let creds = match Self::read_credentials() {
            Ok(c) => c,
            Err(_) => {
                // No OAuth credentials — try PTY fallback.
                return Self::fetch_via_pty().await;
            }
        };

        // Extract OAuth client info from the gemini npm package.
        let client_info = match Self::extract_oauth_client() {
            Some(info) => info,
            None => {
                // Cannot locate OAuth client — try PTY fallback.
                return Self::fetch_via_pty().await;
            }
        };

        // Refresh token if expired.
        let access_token = match Self::refresh_token_if_expired(&creds, &client_info).await {
            Ok(t) => t,
            Err(_) => {
                // Token refresh failed — try PTY fallback.
                return Self::fetch_via_pty().await;
            }
        };

        let client = reqwest::Client::new();

        // Step 1: loadCodeAssist — best-effort to get cloudaicompanionProject.
        // If this fails, we pass an empty string to retrieveUserQuota.
        let project = Self::load_code_assist(&client, &access_token)
            .await
            .unwrap_or_default();

        // Step 2: retrieveUserQuota — fetch the actual quota entries.
        let entries = match Self::retrieve_user_quota(&client, &access_token, &project).await {
            Ok(e) => e,
            Err(_) => {
                // Quota API failed — try PTY fallback.
                return Self::fetch_via_pty().await;
            }
        };

        // Step 3: Parse the entries into a ProviderSnapshot.
        Ok(Self::parse_quota_response(&entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static HOME_ENV_LOCK: Mutex<()> = Mutex::new(());

    // -----------------------------------------------------------------------
    // Credential parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_valid_credentials() {
        let json = r#"{
            "access_token": "ya29.test-gemini-token",
            "refresh_token": "1//refresh-gemini-456",
            "expiry_date": 1741392000.0,
            "id_token": "eyJ.test.id_token"
        }"#;
        let creds: GeminiCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.access_token, "ya29.test-gemini-token");
        assert_eq!(
            creds.refresh_token,
            Some("1//refresh-gemini-456".to_string())
        );
        assert_eq!(creds.expiry_date, Some(1741392000.0));
        assert_eq!(creds.id_token, Some("eyJ.test.id_token".to_string()));
    }

    #[test]
    fn parse_minimal_credentials() {
        let json = r#"{"access_token": "tok"}"#;
        let creds: GeminiCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.access_token, "tok");
        assert!(creds.refresh_token.is_none());
        assert!(creds.expiry_date.is_none());
        assert!(creds.id_token.is_none());
    }

    #[test]
    fn parse_credentials_missing_token_fails() {
        let json = r#"{"refresh_token": "ref"}"#;
        let result: Result<GeminiCredentials, _> = serde_json::from_str(json);
        assert!(result.is_err(), "should fail without access_token");
    }

    #[test]
    fn parse_credentials_with_integer_expiry() {
        // expiry_date may be an integer (epoch seconds).
        let json = r#"{
            "access_token": "tok",
            "expiry_date": 1741392000
        }"#;
        let creds: GeminiCredentials = serde_json::from_str(json).unwrap();
        assert_eq!(creds.expiry_date, Some(1741392000.0));
    }

    #[test]
    fn parse_credentials_with_millis_expiry() {
        // expiry_date may be epoch-milliseconds.
        let json = r#"{
            "access_token": "tok",
            "expiry_date": 1741392000000
        }"#;
        let creds: GeminiCredentials = serde_json::from_str(json).unwrap();
        // Should parse as f64.
        assert!(creds.expiry_date.unwrap() > 1_000_000_000_000.0);
    }

    // -----------------------------------------------------------------------
    // Token expiry tests
    // -----------------------------------------------------------------------

    #[test]
    fn is_token_expired_with_future_date() {
        let future_epoch = (Utc::now().timestamp() + 3600) as f64;
        let creds = GeminiCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            expiry_date: Some(future_epoch),
            id_token: None,
        };
        assert!(
            !GeminiProvider::is_token_expired(&creds),
            "future expiry should not be expired"
        );
    }

    #[test]
    fn is_token_expired_with_past_date() {
        let past_epoch = (Utc::now().timestamp() - 3600) as f64;
        let creds = GeminiCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            expiry_date: Some(past_epoch),
            id_token: None,
        };
        assert!(
            GeminiProvider::is_token_expired(&creds),
            "past expiry should be expired"
        );
    }

    #[test]
    fn is_token_expired_with_millis() {
        // Epoch-milliseconds for a future date.
        let future_ms = ((Utc::now().timestamp() + 3600) * 1000) as f64;
        let creds = GeminiCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            expiry_date: Some(future_ms),
            id_token: None,
        };
        assert!(
            !GeminiProvider::is_token_expired(&creds),
            "future millis expiry should not be expired"
        );
    }

    #[test]
    fn is_token_expired_with_no_expiry() {
        let creds = GeminiCredentials {
            access_token: "tok".into(),
            refresh_token: None,
            expiry_date: None,
            id_token: None,
        };
        assert!(
            !GeminiProvider::is_token_expired(&creds),
            "no expiry date should assume not expired"
        );
    }

    // -----------------------------------------------------------------------
    // OAuth client extraction from JS source tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_client_id_from_js_double_quotes() {
        let js = r#"
const OAUTH_CLIENT_ID = "123456789.apps.googleusercontent.com";
const OAUTH_CLIENT_SECRET = "GOCSPX-abcdef123456";
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(info.is_some(), "should extract from double-quoted JS");
        let info = info.unwrap();
        assert_eq!(info.client_id, "123456789.apps.googleusercontent.com");
        assert_eq!(info.client_secret, "GOCSPX-abcdef123456");
    }

    #[test]
    fn extract_client_id_from_js_single_quotes() {
        let js = r#"
const OAUTH_CLIENT_ID = '987654321.apps.googleusercontent.com';
const OAUTH_CLIENT_SECRET = 'GOCSPX-xyz789';
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(info.is_some(), "should extract from single-quoted JS");
        let info = info.unwrap();
        assert_eq!(info.client_id, "987654321.apps.googleusercontent.com");
        assert_eq!(info.client_secret, "GOCSPX-xyz789");
    }

    #[test]
    fn extract_client_id_with_extra_whitespace() {
        let js = r#"
const OAUTH_CLIENT_ID   =   "spaced-id.apps.googleusercontent.com"  ;
const OAUTH_CLIENT_SECRET   =   "GOCSPX-spaced"  ;
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(
            info.is_some(),
            "should handle extra whitespace around = and value"
        );
        let info = info.unwrap();
        assert_eq!(info.client_id, "spaced-id.apps.googleusercontent.com");
        assert_eq!(info.client_secret, "GOCSPX-spaced");
    }

    #[test]
    fn extract_client_id_missing_id() {
        let js = r#"
const OAUTH_CLIENT_SECRET = "GOCSPX-only-secret";
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(info.is_none(), "should return None when ID is missing");
    }

    #[test]
    fn extract_client_id_missing_secret() {
        let js = r#"
const OAUTH_CLIENT_ID = "only-id.apps.googleusercontent.com";
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(info.is_none(), "should return None when secret is missing");
    }

    #[test]
    fn extract_client_id_empty_value() {
        let js = r#"
const OAUTH_CLIENT_ID = "";
const OAUTH_CLIENT_SECRET = "GOCSPX-secret";
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(info.is_none(), "should return None for empty client ID");
    }

    #[test]
    fn extract_client_id_no_quotes() {
        let js = r#"
const OAUTH_CLIENT_ID = some_variable;
const OAUTH_CLIENT_SECRET = another_variable;
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(
            info.is_none(),
            "should return None when values are not quoted"
        );
    }

    #[test]
    fn extract_js_string_value_basic() {
        assert_eq!(
            GeminiProvider::extract_js_string_value("FOO = \"bar\"", "FOO"),
            Some("bar".to_string())
        );
        assert_eq!(
            GeminiProvider::extract_js_string_value("FOO = 'baz'", "FOO"),
            Some("baz".to_string())
        );
    }

    #[test]
    fn extract_js_string_value_missing_var() {
        assert_eq!(
            GeminiProvider::extract_js_string_value("BAR = \"val\"", "FOO"),
            None
        );
    }

    #[test]
    fn extract_js_string_value_no_equals() {
        assert_eq!(
            GeminiProvider::extract_js_string_value("FOO: \"val\"", "FOO"),
            None
        );
    }

    // -----------------------------------------------------------------------
    // Realistic oauth2.js content test
    // -----------------------------------------------------------------------

    #[test]
    fn extract_from_realistic_oauth2_js() {
        // Simulates the structure of the actual oauth2.js file.
        let js = r#"
"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.refreshToken = exports.getOAuthToken = void 0;

const OAUTH_CLIENT_ID = "123456789012-abcdefghijklmnop.apps.googleusercontent.com";
const OAUTH_CLIENT_SECRET = "GOCSPX-AbCdEfGhIjKlMnOpQrStUvWxYz";
const SCOPES = ["openid", "email", "profile"];

async function getOAuthToken() {
    // ... implementation
}

async function refreshToken(credentials) {
    // ... implementation
}

exports.getOAuthToken = getOAuthToken;
exports.refreshToken = refreshToken;
"#;
        let info = GeminiProvider::extract_oauth_client_from_source(js);
        assert!(info.is_some(), "should parse realistic oauth2.js content");
        let info = info.unwrap();
        assert_eq!(
            info.client_id,
            "123456789012-abcdefghijklmnop.apps.googleusercontent.com"
        );
        assert_eq!(info.client_secret, "GOCSPX-AbCdEfGhIjKlMnOpQrStUvWxYz");
    }

    // -----------------------------------------------------------------------
    // detect() tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_missing_creds_and_binary() {
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        // Set HOME to a temp dir so credentials file won't resolve.
        let td = tempfile::tempdir().unwrap();
        let saved = std::env::var("HOME").ok();
        std::env::set_var("HOME", td.path());

        let provider = GeminiProvider::new();
        // detect() may still return true if `gemini` is on PATH.
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
        let _guard = HOME_ENV_LOCK.lock().unwrap();
        let td = tempfile::tempdir().unwrap();
        let gemini_dir = td.path().join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join("oauth_creds.json"),
            r#"{"access_token":"test"}"#,
        )
        .unwrap();

        let saved = std::env::var("HOME").ok();
        std::env::set_var("HOME", td.path());

        let provider = GeminiProvider::new();
        assert!(
            provider.detect(),
            "detect() should return true when oauth_creds.json exists"
        );

        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }

    // -----------------------------------------------------------------------
    // Token refresh response parsing test
    // -----------------------------------------------------------------------

    #[test]
    fn parse_token_refresh_response() {
        let json = r#"{
            "access_token": "ya29.new-refreshed-token",
            "expires_in": 3600,
            "scope": "openid email",
            "token_type": "Bearer"
        }"#;
        let resp: TokenRefreshResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token, "ya29.new-refreshed-token");
        assert_eq!(resp.expires_in, Some(3600));
    }

    // -----------------------------------------------------------------------
    // Quota response parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_quota_valid_pro_and_flash() {
        let entries = vec![
            QuotaEntry {
                remaining_fraction: Some(0.62),
                reset_time: Some("2026-03-08T18:00:00Z".into()),
                model_id: Some("gemini-2.5-pro".into()),
            },
            QuotaEntry {
                remaining_fraction: Some(0.45),
                reset_time: Some("2026-03-08T18:00:00Z".into()),
                model_id: Some("gemini-2.5-pro-exp-03-25".into()),
            },
            QuotaEntry {
                remaining_fraction: Some(0.80),
                reset_time: Some("2026-03-08T20:00:00Z".into()),
                model_id: Some("gemini-2.5-flash".into()),
            },
            QuotaEntry {
                remaining_fraction: Some(0.90),
                reset_time: Some("2026-03-08T20:00:00Z".into()),
                model_id: Some("gemini-2.0-flash".into()),
            },
        ];

        let snap = GeminiProvider::parse_quota_response(&entries);
        assert_eq!(snap.provider, "gemini");
        assert_eq!(snap.display_name, "Gemini CLI");
        assert_eq!(snap.source, "oauth");
        assert!(snap.error.is_none());

        // Pro: min(0.62, 0.45) = 0.45 → (1 - 0.45) * 100 = 55
        assert_eq!(snap.primary_pct, Some(55));
        assert_eq!(snap.primary_label.as_deref(), Some("Pro"));

        // Flash: min(0.80, 0.90) = 0.80 → (1 - 0.80) * 100 = 20
        assert_eq!(snap.secondary_pct, Some(20));
        assert_eq!(snap.secondary_label.as_deref(), Some("Flash"));

        // Earliest reset across all entries is 2026-03-08T18:00:00Z
        let reset = snap.reset_at.expect("reset_at should be Some");
        assert_eq!(
            reset,
            "2026-03-08T18:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn parse_quota_pro_only() {
        let entries = vec![QuotaEntry {
            remaining_fraction: Some(0.30),
            reset_time: Some("2026-03-09T12:00:00Z".into()),
            model_id: Some("gemini-2.5-pro".into()),
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        // Pro: (1 - 0.30) * 100 = 70
        assert_eq!(snap.primary_pct, Some(70));
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn parse_quota_flash_only() {
        let entries = vec![QuotaEntry {
            remaining_fraction: Some(0.50),
            reset_time: None,
            model_id: Some("gemini-2.0-flash-lite".into()),
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        assert_eq!(snap.primary_pct, None);
        // Flash: (1 - 0.50) * 100 = 50
        assert_eq!(snap.secondary_pct, Some(50));
        assert_eq!(snap.reset_at, None);
    }

    #[test]
    fn parse_quota_empty_entries() {
        let entries: Vec<QuotaEntry> = vec![];
        let snap = GeminiProvider::parse_quota_response(&entries);

        assert_eq!(snap.provider, "gemini");
        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
        assert_eq!(snap.reset_at, None);
    }

    #[test]
    fn parse_quota_no_remaining_fraction() {
        let entries = vec![QuotaEntry {
            remaining_fraction: None,
            reset_time: Some("2026-03-08T18:00:00Z".into()),
            model_id: Some("gemini-2.5-pro".into()),
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        // No remaining_fraction → no percentage
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn parse_quota_unknown_model_id() {
        // Models that don't contain "pro" or "flash" are ignored for pct.
        let entries = vec![QuotaEntry {
            remaining_fraction: Some(0.10),
            reset_time: Some("2026-03-08T18:00:00Z".into()),
            model_id: Some("gemini-2.0-ultra".into()),
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
        // Reset time should still be tracked
        assert!(snap.reset_at.is_some());
    }

    #[test]
    fn parse_quota_clamping() {
        // remainingFraction > 1.0 should clamp to 0%
        // remainingFraction < 0.0 should clamp to 100%
        let entries = vec![
            QuotaEntry {
                remaining_fraction: Some(1.5),
                reset_time: None,
                model_id: Some("gemini-2.5-pro".into()),
            },
            QuotaEntry {
                remaining_fraction: Some(-0.2),
                reset_time: None,
                model_id: Some("gemini-2.0-flash".into()),
            },
        ];

        let snap = GeminiProvider::parse_quota_response(&entries);
        // Pro: (1 - 1.5) * 100 = -50 → clamped to 0
        assert_eq!(snap.primary_pct, Some(0));
        // Flash: (1 - (-0.2)) * 100 = 120 → clamped to 100
        assert_eq!(snap.secondary_pct, Some(100));
    }

    #[test]
    fn parse_quota_case_insensitive_model_id() {
        let entries = vec![
            QuotaEntry {
                remaining_fraction: Some(0.75),
                reset_time: None,
                model_id: Some("Gemini-2.5-PRO".into()),
            },
            QuotaEntry {
                remaining_fraction: Some(0.60),
                reset_time: None,
                model_id: Some("GEMINI-2.0-FLASH".into()),
            },
        ];

        let snap = GeminiProvider::parse_quota_response(&entries);
        // Pro: (1 - 0.75) * 100 = 25
        assert_eq!(snap.primary_pct, Some(25));
        // Flash: (1 - 0.60) * 100 = 40
        assert_eq!(snap.secondary_pct, Some(40));
    }

    #[test]
    fn parse_quota_missing_model_id() {
        // Entry with no model_id — should be ignored for pro/flash classification.
        let entries = vec![QuotaEntry {
            remaining_fraction: Some(0.50),
            reset_time: None,
            model_id: None,
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn parse_quota_full_usage() {
        // remainingFraction = 0 means fully used → 100%
        let entries = vec![QuotaEntry {
            remaining_fraction: Some(0.0),
            reset_time: Some("2026-03-08T06:00:00Z".into()),
            model_id: Some("gemini-2.5-pro".into()),
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        assert_eq!(snap.primary_pct, Some(100));
    }

    #[test]
    fn parse_quota_no_usage() {
        // remainingFraction = 1.0 means nothing used → 0%
        let entries = vec![QuotaEntry {
            remaining_fraction: Some(1.0),
            reset_time: None,
            model_id: Some("gemini-2.5-pro".into()),
        }];

        let snap = GeminiProvider::parse_quota_response(&entries);
        assert_eq!(snap.primary_pct, Some(0));
    }

    // -----------------------------------------------------------------------
    // Quota response deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn deserialize_quota_response_object_shape() {
        let json = r#"{
            "quotas": [
                {
                    "remainingFraction": 0.62,
                    "resetTime": "2026-03-08T18:00:00Z",
                    "modelId": "gemini-2.5-pro"
                },
                {
                    "remainingFraction": 0.80,
                    "resetTime": "2026-03-08T20:00:00Z",
                    "modelId": "gemini-2.5-flash"
                }
            ]
        }"#;
        let resp: QuotaResponse = serde_json::from_str(json).unwrap();
        let entries = resp.into_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].model_id.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(entries[0].remaining_fraction, Some(0.62));
        assert_eq!(entries[1].model_id.as_deref(), Some("gemini-2.5-flash"));
    }

    #[test]
    fn deserialize_quota_response_array_shape() {
        let json = r#"[
            {
                "remainingFraction": 0.50,
                "resetTime": "2026-03-08T18:00:00Z",
                "modelId": "gemini-2.5-pro"
            }
        ]"#;
        let resp: QuotaResponse = serde_json::from_str(json).unwrap();
        let entries = resp.into_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].remaining_fraction, Some(0.50));
    }

    #[test]
    fn deserialize_load_code_assist_response() {
        let json = r#"{
            "cloudaicompanionProject": "my-project-123",
            "other_field": "ignored"
        }"#;
        let resp: LoadCodeAssistResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            resp.cloudai_companion_project,
            Some("my-project-123".to_string())
        );
    }

    #[test]
    fn deserialize_load_code_assist_response_missing_project() {
        let json = r#"{"other_field": "value"}"#;
        let resp: LoadCodeAssistResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.cloudai_companion_project, None);
    }

    // -----------------------------------------------------------------------
    // PTY parse_pty_output tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_pty_parse_fraction_pro_and_flash() {
        let output = concat!(
            "Model Usage:\n",
            "  gemini-2.5-pro: 42/50 requests\n",
            "  gemini-2.0-flash: 10/100 requests\n",
        );
        let snap = GeminiProvider::parse_pty_output(output);

        assert_eq!(snap.provider, "gemini");
        assert_eq!(snap.display_name, "Gemini CLI");
        assert_eq!(snap.source, "pty");
        assert!(snap.error.is_none(), "no error expected: {:?}", snap.error);

        // Pro: 42/50 = 84%
        assert_eq!(snap.primary_pct, Some(84));
        assert_eq!(snap.primary_label.as_deref(), Some("Pro"));

        // Flash: 10/100 = 10%
        assert_eq!(snap.secondary_pct, Some(10));
        assert_eq!(snap.secondary_label.as_deref(), Some("Flash"));
    }

    #[test]
    fn test_pty_parse_percentage_format() {
        let output = "gemini-2.5-pro: 76% used\ngemini-2.5-flash: 20% used\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(76));
        assert_eq!(snap.secondary_pct, Some(20));
    }

    #[test]
    fn test_pty_parse_pro_only() {
        let output = "gemini-2.5-pro 38/50\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        // 38/50 = 76%
        assert_eq!(snap.primary_pct, Some(76));
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_pty_parse_flash_only() {
        let output = "gemini-2.0-flash: 25/100\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, None);
        // 25/100 = 25%
        assert_eq!(snap.secondary_pct, Some(25));
    }

    #[test]
    fn test_pty_parse_no_model_data() {
        let output = "Gemini CLI v1.0\nReady.\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert_eq!(snap.source, "pty");
        assert!(snap.error.is_some());
        assert!(
            snap.error.unwrap().contains("no model usage data"),
            "error should mention missing model data"
        );
    }

    #[test]
    fn test_pty_parse_empty_output() {
        let snap = GeminiProvider::parse_pty_output("");

        assert_eq!(snap.source, "pty");
        assert!(snap.error.is_some());
        assert_eq!(snap.primary_pct, None);
        assert_eq!(snap.secondary_pct, None);
    }

    #[test]
    fn test_pty_parse_fraction_zero_limit() {
        // Denominator 0 should be skipped (no division by zero).
        let output = "gemini-2.5-pro: 0/0 requests\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_some());
        assert_eq!(snap.primary_pct, None);
    }

    #[test]
    fn test_pty_parse_full_usage() {
        let output = "gemini-2.5-pro: 50/50 requests\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        // 50/50 = 100%
        assert_eq!(snap.primary_pct, Some(100));
    }

    #[test]
    fn test_pty_parse_no_usage() {
        let output = "gemini-2.5-pro: 0/50 requests\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        // 0/50 = 0%
        assert_eq!(snap.primary_pct, Some(0));
    }

    #[test]
    fn test_pty_parse_case_insensitive_model() {
        let output = "GEMINI-2.5-PRO: 30/100\nGEMINI-2.0-FLASH: 5/50\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(30));
        assert_eq!(snap.secondary_pct, Some(10));
    }

    #[test]
    fn test_pty_parse_with_ansi_noise() {
        // Terminal output may have ANSI escape codes.
        let output = "\x1b[1mStats\x1b[0m\ngemini-2.5-pro: 42/50\ngemini-2.0-flash: 80%\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        assert_eq!(snap.primary_pct, Some(84));
        assert_eq!(snap.secondary_pct, Some(80));
    }

    #[test]
    fn test_pty_parse_takes_first_match_per_model() {
        // If multiple lines match "pro", take the first one.
        let output = "gemini-2.5-pro: 40/50\ngemini-2.5-pro-exp: 20/50\n";
        let snap = GeminiProvider::parse_pty_output(output);

        assert!(snap.error.is_none());
        // First pro line: 40/50 = 80%
        assert_eq!(snap.primary_pct, Some(80));
    }

    // -----------------------------------------------------------------------
    // PTY helper function tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_fraction_pct_basic() {
        assert_eq!(GeminiProvider::extract_fraction_pct("42/50"), Some(84));
        assert_eq!(GeminiProvider::extract_fraction_pct("0/100"), Some(0));
        assert_eq!(GeminiProvider::extract_fraction_pct("100/100"), Some(100));
    }

    #[test]
    fn test_extract_fraction_pct_with_text() {
        assert_eq!(
            GeminiProvider::extract_fraction_pct("requests: 42/50 used"),
            Some(84)
        );
    }

    #[test]
    fn test_extract_fraction_pct_no_slash() {
        assert_eq!(GeminiProvider::extract_fraction_pct("no slash here"), None);
    }

    #[test]
    fn test_extract_fraction_pct_zero_denominator() {
        assert_eq!(GeminiProvider::extract_fraction_pct("5/0"), None);
    }

    #[test]
    fn test_extract_direct_pct_basic() {
        assert_eq!(GeminiProvider::extract_direct_pct("76%"), Some(76));
        assert_eq!(GeminiProvider::extract_direct_pct("usage: 100%"), Some(100));
        assert_eq!(GeminiProvider::extract_direct_pct("0% used"), Some(0));
    }

    #[test]
    fn test_extract_direct_pct_clamps() {
        assert_eq!(GeminiProvider::extract_direct_pct("150%"), Some(100));
    }

    #[test]
    fn test_extract_direct_pct_no_percent() {
        assert_eq!(GeminiProvider::extract_direct_pct("no percentage"), None);
    }

    #[test]
    fn test_extract_line_pct_prefers_fraction() {
        // When both fraction and percentage are present, fraction wins.
        assert_eq!(GeminiProvider::extract_line_pct("42/50 (84%)"), Some(84));
    }

    #[test]
    fn test_extract_line_pct_falls_back_to_percentage() {
        assert_eq!(GeminiProvider::extract_line_pct("usage: 76%"), Some(76));
    }

    #[test]
    fn test_extract_line_pct_none() {
        assert_eq!(GeminiProvider::extract_line_pct("no numbers here"), None);
    }
}
