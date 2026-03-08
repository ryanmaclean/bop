use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

use super::{Provider, ProviderSnapshot};

const OPENCODE_HEALTH_URL: &str = "http://localhost:4096/health";
const OPENCODE_SESSION_URL: &str = "http://localhost:4096/session";
const OPENCODE_EVENT_URL: &str = "http://localhost:4096/event";
const DETECT_TIMEOUT_MS: u64 = 500;
const FETCH_TIMEOUT_S: u64 = 5;
const SSE_FALLBACK_POLL_S: u64 = 60;

#[derive(Debug, Clone, Deserialize)]
struct HealthResponse {
    #[serde(default)]
    healthy: bool,
}

pub struct OpenCodeProvider;

impl OpenCodeProvider {
    pub fn new() -> Self {
        Self
    }

    fn base_snapshot() -> ProviderSnapshot {
        ProviderSnapshot {
            provider: "opencode".into(),
            display_name: "opencode".into(),
            primary_pct: None,
            secondary_pct: None,
            primary_label: None,
            secondary_label: None,
            tokens_used: None,
            cost_usd: None,
            reset_at: None,
            source: "http".into(),
            error: None,
            loaded_models: None,
        }
    }

    fn error_snapshot(error: impl Into<String>) -> ProviderSnapshot {
        let mut snap = Self::base_snapshot();
        snap.error = Some(error.into());
        snap
    }

    fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
        let mut current = value;
        for key in path {
            current = current.get(*key)?;
        }
        Some(current)
    }

    fn to_u64(value: &Value) -> Option<u64> {
        match value {
            Value::Number(n) => n
                .as_u64()
                .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok()))
                .or_else(|| {
                    n.as_f64()
                        .and_then(|v| if v >= 0.0 { Some(v as u64) } else { None })
                }),
            Value::String(s) => s.trim().parse::<u64>().ok(),
            _ => None,
        }
    }

    fn to_f64(value: &Value) -> Option<f64> {
        match value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }

    fn parse_time_value(value: &Value) -> Option<i64> {
        match value {
            Value::String(s) => {
                if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                    Some(dt.with_timezone(&Utc).timestamp_millis())
                } else {
                    s.parse::<i64>().ok()
                }
            }
            Value::Number(n) => n
                .as_i64()
                .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok())),
            _ => None,
        }
    }

    fn select_latest_session_id(sessions: &[Value]) -> Option<String> {
        let time_paths: &[&[&str]] = &[
            &["updated_at"],
            &["updatedAt"],
            &["last_updated_at"],
            &["lastUpdatedAt"],
            &["created_at"],
            &["createdAt"],
        ];
        let id_paths: &[&[&str]] = &[
            &["id"],
            &["session_id"],
            &["sessionId"],
            &["metadata", "id"],
            &["metadata", "session_id"],
        ];

        let mut best_id: Option<String> = None;
        let mut best_time = i64::MIN;

        for (idx, session) in sessions.iter().enumerate() {
            let id = id_paths
                .iter()
                .find_map(|path| Self::value_at_path(session, path))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            let Some(id) = id else {
                continue;
            };

            let time = time_paths
                .iter()
                .find_map(|path| Self::value_at_path(session, path))
                .and_then(Self::parse_time_value)
                // Keep stable ordering when no timestamp is present.
                .unwrap_or(i64::MIN + idx as i64);

            if time >= best_time {
                best_time = time;
                best_id = Some(id);
            }
        }

        best_id
    }

    fn parse_sessions_response(body: &str) -> anyhow::Result<Option<String>> {
        let parsed: Value = serde_json::from_str(body).context("failed to parse /session JSON")?;

        let sessions: Vec<Value> = match &parsed {
            Value::Array(items) => items.clone(),
            Value::Object(map) => map
                .get("sessions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        if sessions.is_empty() {
            return Ok(None);
        }

        Ok(Self::select_latest_session_id(&sessions))
    }

    fn extract_tokens(value: &Value) -> Option<u64> {
        let direct_paths: &[&[&str]] = &[
            &["total_tokens"],
            &["totalTokens"],
            &["tokens"],
            &["tokens_used"],
            &["tokensUsed"],
            &["usage", "total_tokens"],
            &["usage", "totalTokens"],
            &["usage", "tokens"],
            &["metadata", "total_tokens"],
            &["metadata", "totalTokens"],
            &["metadata", "tokens"],
            &["metadata", "tokens_used"],
            &["stats", "total_tokens"],
        ];

        for path in direct_paths {
            if let Some(raw) = Self::value_at_path(value, path) {
                if let Some(tokens) = Self::to_u64(raw) {
                    return Some(tokens);
                }
            }
        }

        let input = [
            &["usage", "input_tokens"][..],
            &["usage", "inputTokens"],
            &["metadata", "input_tokens"],
            &["metadata", "inputTokens"],
        ]
        .iter()
        .find_map(|path| Self::value_at_path(value, path))
        .and_then(Self::to_u64);

        let output = [
            &["usage", "output_tokens"][..],
            &["usage", "outputTokens"],
            &["metadata", "output_tokens"],
            &["metadata", "outputTokens"],
        ]
        .iter()
        .find_map(|path| Self::value_at_path(value, path))
        .and_then(Self::to_u64);

        match (input, output) {
            (Some(i), Some(o)) => Some(i.saturating_add(o)),
            _ => None,
        }
    }

    fn extract_cost(value: &Value) -> Option<f64> {
        let paths: &[&[&str]] = &[
            &["cost_usd"],
            &["costUsd"],
            &["total_cost_usd"],
            &["totalCostUsd"],
            &["cost"],
            &["usage", "cost_usd"],
            &["usage", "costUsd"],
            &["usage", "total_cost_usd"],
            &["usage", "totalCostUsd"],
            &["metadata", "cost_usd"],
            &["metadata", "costUsd"],
            &["metadata", "total_cost_usd"],
            &["metadata", "totalCostUsd"],
        ];

        for path in paths {
            if let Some(raw) = Self::value_at_path(value, path) {
                if let Some(cost) = Self::to_f64(raw) {
                    return Some(cost);
                }
            }
        }

        None
    }

    fn parse_session_detail(body: &str) -> anyhow::Result<(Option<u64>, Option<f64>)> {
        let parsed: Value =
            serde_json::from_str(body).context("failed to parse /session/:id JSON")?;

        let roots = [
            Some(&parsed),
            parsed.get("session"),
            parsed.get("data"),
            parsed.get("data").and_then(|v| v.get("session")),
        ];

        let mut tokens = None;
        let mut cost = None;

        for root in roots.into_iter().flatten() {
            if tokens.is_none() {
                tokens = Self::extract_tokens(root);
            }
            if cost.is_none() {
                cost = Self::extract_cost(root);
            }
            if tokens.is_some() && cost.is_some() {
                break;
            }
        }

        Ok((tokens, cost))
    }

    async fn stream_sse_updates(
        &self,
        tx: &tokio::sync::mpsc::UnboundedSender<ProviderSnapshot>,
    ) -> anyhow::Result<()> {
        let client = reqwest::Client::new();
        let mut resp = client
            .get(OPENCODE_EVENT_URL)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .context("failed to connect to opencode SSE")?;

        anyhow::ensure!(
            resp.status().is_success(),
            "opencode SSE returned HTTP {}",
            resp.status()
        );

        let mut buffer = String::new();
        let mut event_name: Option<String> = None;

        while let Some(chunk) = resp.chunk().await.context("failed to read SSE chunk")? {
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(pos) = buffer.find('\n') {
                let mut line = buffer[..pos].to_string();
                buffer.drain(..=pos);

                if line.ends_with('\r') {
                    line.pop();
                }

                if line.is_empty() {
                    if event_name
                        .as_deref()
                        .map(|event| event.starts_with("session."))
                        .unwrap_or(false)
                    {
                        if let Ok(snapshot) = self.fetch().await {
                            let _ = tx.send(snapshot);
                        }
                    }
                    event_name = None;
                    continue;
                }

                if let Some(name) = line.strip_prefix("event:") {
                    event_name = Some(name.trim().to_string());
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Provider for OpenCodeProvider {
    fn name(&self) -> &str {
        "opencode"
    }

    fn detect(&self) -> bool {
        super::run_detect_async(async {
            let client = reqwest::Client::new();
            let req = client.get(OPENCODE_HEALTH_URL).send();

            let response =
                match tokio::time::timeout(Duration::from_millis(DETECT_TIMEOUT_MS), req).await {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(_)) | Err(_) => return false,
                };

            if !response.status().is_success() {
                return false;
            }

            match response.json::<HealthResponse>().await {
                Ok(health) => health.healthy,
                Err(_) => false,
            }
        })
    }

    async fn fetch(&self) -> anyhow::Result<ProviderSnapshot> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(FETCH_TIMEOUT_S))
            .build()
            .context("failed to build HTTP client")?;

        let list_resp = client
            .get(OPENCODE_SESSION_URL)
            .send()
            .await
            .context("failed to GET /session")?;

        if !list_resp.status().is_success() {
            return Ok(Self::error_snapshot(format!(
                "GET /session failed with HTTP {}",
                list_resp.status()
            )));
        }

        let list_body = list_resp
            .text()
            .await
            .context("failed to read /session response")?;

        let latest_session_id = match Self::parse_sessions_response(&list_body) {
            Ok(session_id) => session_id,
            Err(e) => return Ok(Self::error_snapshot(e.to_string())),
        };

        let Some(session_id) = latest_session_id else {
            return Ok(Self::base_snapshot());
        };

        let detail_resp = client
            .get(format!("{OPENCODE_SESSION_URL}/{session_id}"))
            .send()
            .await
            .with_context(|| format!("failed to GET /session/{session_id}"))?;

        if !detail_resp.status().is_success() {
            return Ok(Self::error_snapshot(format!(
                "GET /session/{session_id} failed with HTTP {}",
                detail_resp.status()
            )));
        }

        let detail_body = detail_resp
            .text()
            .await
            .with_context(|| format!("failed to read /session/{session_id} response"))?;

        let (tokens_used, cost_usd) = match Self::parse_session_detail(&detail_body) {
            Ok(parsed) => parsed,
            Err(e) => return Ok(Self::error_snapshot(e.to_string())),
        };

        let mut snapshot = Self::base_snapshot();
        snapshot.tokens_used = tokens_used;
        snapshot.cost_usd = cost_usd;
        Ok(snapshot)
    }
}

#[allow(dead_code)] // retained for optional SSE integrations
pub fn spawn_watch_task(
    tx: tokio::sync::mpsc::UnboundedSender<ProviderSnapshot>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let provider = OpenCodeProvider::new();

        loop {
            let _ = provider.stream_sse_updates(&tx).await;

            loop {
                tokio::time::sleep(Duration::from_secs(SSE_FALLBACK_POLL_S)).await;

                if let Ok(snapshot) = provider.fetch().await {
                    let _ = tx.send(snapshot);
                }

                if provider.stream_sse_updates(&tx).await.is_ok() {
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sessions_response_with_sessions() {
        let body = r#"{
            "sessions": [
                {"id": "sess-old", "updated_at": "2026-03-07T11:00:00Z"},
                {"id": "sess-new", "updated_at": "2026-03-07T12:00:00Z"}
            ]
        }"#;

        let session_id = OpenCodeProvider::parse_sessions_response(body)
            .expect("parse sessions")
            .expect("session id");

        assert_eq!(session_id, "sess-new");
    }

    #[test]
    fn parse_sessions_response_with_empty_sessions() {
        let session_id =
            OpenCodeProvider::parse_sessions_response(r#"{"sessions": []}"#).expect("parse");
        assert_eq!(session_id, None);
    }

    #[test]
    fn parse_session_detail_with_tokens_and_cost() {
        let body = r#"{
            "session": {
                "metadata": {
                    "total_tokens": 12345,
                    "total_cost_usd": 0.42
                }
            }
        }"#;

        let (tokens, cost) = OpenCodeProvider::parse_session_detail(body).expect("parse detail");
        assert_eq!(tokens, Some(12345));
        assert_eq!(cost, Some(0.42));
    }

    #[test]
    fn parse_session_detail_without_tokens_or_cost() {
        let body = r#"{"session": {"metadata": {"title": "demo"}}}"#;
        let (tokens, cost) = OpenCodeProvider::parse_session_detail(body).expect("parse detail");
        assert_eq!(tokens, None);
        assert_eq!(cost, None);
    }

    #[test]
    fn parse_sessions_response_malformed_json() {
        let result = OpenCodeProvider::parse_sessions_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_session_detail_malformed_json() {
        let result = OpenCodeProvider::parse_session_detail("not json");
        assert!(result.is_err());
    }
}
