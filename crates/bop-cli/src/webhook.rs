use anyhow::{anyhow, Context};
use bop_core::config::{
    merge_configs, read_config_file, WebhookConfig, WebhookEvent, WebhookFormat,
};
use bop_core::{Meta, RunRecord};
use chrono::Utc;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct WebhookEventData {
    pub event: WebhookEvent,
    pub card_id: String,
    pub state: String,
    pub provider: Option<String>,
    pub cost_usd: Option<f64>,
    pub tokens: Option<u64>,
    pub duration_secs: Option<u64>,
    pub timestamp: String,
}

impl WebhookEventData {
    pub fn from_meta(event: WebhookEvent, meta: Option<&Meta>) -> Self {
        let card_id = meta
            .map(|m| m.id.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let latest_run = meta.and_then(|m| m.runs.last());

        let provider = latest_run
            .and_then(|run| {
                if run.provider.trim().is_empty() {
                    None
                } else {
                    Some(run.provider.clone())
                }
            })
            .or_else(|| meta.and_then(|m| m.provider_chain.first().cloned()));

        let tokens = latest_run.and_then(total_tokens);
        let cost_usd = latest_run.and_then(|run| run.cost_usd);
        let duration_secs = latest_run.and_then(|run| run.duration_s).or_else(|| {
            meta.and_then(|m| {
                m.stages
                    .get(&m.stage)
                    .and_then(|stage_record| stage_record.duration_s)
            })
        });

        Self {
            event,
            card_id,
            state: event.as_str().to_string(),
            provider,
            cost_usd,
            tokens,
            duration_secs,
            timestamp: Utc::now().to_rfc3339(),
        }
    }

    pub fn test_payload() -> Self {
        Self {
            event: WebhookEvent::Done,
            card_id: "bop/webhook-test".to_string(),
            state: "done".to_string(),
            provider: Some("bop".to_string()),
            cost_usd: Some(0.0),
            tokens: Some(42),
            duration_secs: Some(1),
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebhookSendResult {
    pub url: String,
    pub status: Option<StatusCode>,
    pub error: Option<String>,
}

impl WebhookSendResult {
    pub fn status_label(&self) -> String {
        match (self.status, self.error.as_ref()) {
            (Some(status), None) => {
                let reason = status.canonical_reason().unwrap_or("UNKNOWN");
                format!("{} {}", status.as_u16(), reason)
            }
            (_, Some(err)) => format!("FAILED ({err})"),
            _ => "FAILED (unknown error)".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct WebhookClient {
    http: reqwest::Client,
    webhooks: Arc<Vec<WebhookConfig>>,
    default_log_path: PathBuf,
}

impl WebhookClient {
    pub fn from_cards_dir(cards_dir: &Path) -> anyhow::Result<Self> {
        let webhooks = load_webhooks(cards_dir)?;
        Ok(Self {
            http: reqwest::Client::new(),
            webhooks: Arc::new(webhooks),
            default_log_path: cards_dir.join("logs").join("webhook.log"),
        })
    }

    #[cfg(test)]
    pub fn from_webhooks_for_test(webhooks: Vec<WebhookConfig>, log_path: PathBuf) -> Self {
        Self {
            http: reqwest::Client::new(),
            webhooks: Arc::new(webhooks),
            default_log_path: log_path,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.webhooks.is_empty()
    }

    pub fn emit_transition(&self, event: WebhookEvent, meta: Option<&Meta>, card_dir: &Path) {
        if self.webhooks.is_empty() {
            return;
        }

        let payload = WebhookEventData::from_meta(event, meta);
        let log_path = card_dir.join("logs").join("webhook.log");
        let client = self.clone();
        tokio::spawn(async move {
            let _ = client
                .send_matching_with_log(&payload, &log_path, false)
                .await;
        });
    }

    pub async fn send_test(&self) -> Vec<WebhookSendResult> {
        if self.webhooks.is_empty() {
            return Vec::new();
        }

        let payload = WebhookEventData::test_payload();
        self.send_matching_with_log(&payload, &self.default_log_path, true)
            .await
    }

    async fn send_matching_with_log(
        &self,
        event: &WebhookEventData,
        log_path: &Path,
        include_all: bool,
    ) -> Vec<WebhookSendResult> {
        let mut results = Vec::new();
        for hook in self.webhooks.iter() {
            if !include_all && !hook.on.is_empty() && !hook.on.contains(&event.event) {
                continue;
            }

            let result = self.send_one(hook, event).await;
            if let Some(err) = result.error.as_ref() {
                let line = format!(
                    "{} url={} event={} error={}",
                    Utc::now().to_rfc3339(),
                    hook.url,
                    event.event.as_str(),
                    err
                );
                let _ = append_webhook_log(log_path, &line);
            }
            results.push(result);
        }
        results
    }

    async fn send_one(&self, hook: &WebhookConfig, event: &WebhookEventData) -> WebhookSendResult {
        let payload = render_payload(hook.format.clone(), event);
        match post_with_retry(&self.http, &hook.url, payload).await {
            Ok(status) => WebhookSendResult {
                url: hook.url.clone(),
                status: Some(status),
                error: None,
            },
            Err(err) => WebhookSendResult {
                url: hook.url.clone(),
                status: None,
                error: Some(err.to_string()),
            },
        }
    }
}

fn append_webhook_log(path: &Path, line: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create webhook log dir {}", parent.display()))?;
    }
    crate::util::append_log_line(path, line)
}

fn total_tokens(run: &RunRecord) -> Option<u64> {
    match (run.prompt_tokens, run.completion_tokens) {
        (Some(prompt), Some(completion)) => Some(prompt.saturating_add(completion)),
        (Some(prompt), None) => Some(prompt),
        (None, Some(completion)) => Some(completion),
        (None, None) => None,
    }
}

pub fn load_webhooks(cards_dir: &Path) -> anyhow::Result<Vec<WebhookConfig>> {
    let base = bop_core::load_config().unwrap_or_default();
    let cards_local_path = cards_dir.join(".bop").join("config.json");

    let merged = if cards_local_path.exists() {
        let local = read_config_file(&cards_local_path)
            .with_context(|| format!("cards-local config error: {}", cards_local_path.display()))?;
        merge_configs(base, local)
    } else {
        base
    };

    Ok(merged.webhooks.unwrap_or_default())
}

async fn post_with_retry(
    client: &reqwest::Client,
    url: &str,
    payload: Value,
) -> anyhow::Result<StatusCode> {
    match post_once(client, url, payload.clone()).await {
        Ok(status) => Ok(status),
        Err(err) if is_retryable_network_error(&err) => {
            tokio::time::sleep(Duration::from_millis(250)).await;
            post_once(client, url, payload)
                .await
                .with_context(|| format!("retry failed after network error: {err}"))
        }
        Err(err) => Err(anyhow!(err)).with_context(|| "webhook post failed"),
    }
}

async fn post_once(
    client: &reqwest::Client,
    url: &str,
    payload: Value,
) -> anyhow::Result<StatusCode> {
    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("request failed for {url}"))?;

    let status = response.status();
    if status.is_success() {
        Ok(status)
    } else {
        Err(anyhow!("http status {}", status.as_u16()))
            .with_context(|| format!("webhook responded non-success: {status}"))
    }
}

fn is_retryable_network_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .any(|err| err.is_timeout() || err.is_connect() || err.is_request() || err.is_body())
}

fn render_payload(format: WebhookFormat, event: &WebhookEventData) -> Value {
    match format {
        WebhookFormat::Json => render_json_payload(event),
        WebhookFormat::Slack => render_slack_payload(event),
    }
}

fn render_json_payload(event: &WebhookEventData) -> Value {
    json!({
        "event": event.event.as_str(),
        "card_id": event.card_id,
        "state": event.state,
        "provider": event.provider,
        "cost_usd": event.cost_usd,
        "tokens": event.tokens,
        "duration_secs": event.duration_secs,
        "timestamp": event.timestamp,
    })
}

fn render_slack_payload(event: &WebhookEventData) -> Value {
    let icon = event_icon(event.event);
    let verb = event_verb(event.event);
    let provider = event
        .provider
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let cost = event
        .cost_usd
        .map(|value| format!("${value:.2}"))
        .unwrap_or_else(|| "$0.00".to_string());
    let tokens = event
        .tokens
        .map(format_tokens)
        .map(|v| format!("{v} tokens"))
        .unwrap_or_else(|| "n/a tokens".to_string());
    let duration = event
        .duration_secs
        .map(format_duration)
        .unwrap_or_else(|| "n/a".to_string());

    let text = format!(
        "{} {} {} in {} ({})",
        icon, event.card_id, event.state, duration, cost
    );
    let details = format!(
        "{} *{}* {}\n>Provider: {}  •  {}  •  {}",
        icon, event.card_id, verb, provider, cost, tokens
    );

    json!({
        "text": text,
        "blocks": [
            {
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": details
                }
            }
        ]
    })
}

fn event_icon(event: WebhookEvent) -> &'static str {
    match event {
        WebhookEvent::Done | WebhookEvent::Merged => "✓",
        WebhookEvent::Failed => "✗",
        WebhookEvent::Running => "▶",
        WebhookEvent::Pending => "…",
    }
}

fn event_verb(event: WebhookEvent) -> &'static str {
    match event {
        WebhookEvent::Done => "completed",
        WebhookEvent::Failed => "failed",
        WebhookEvent::Running => "started",
        WebhookEvent::Merged => "merged",
        WebhookEvent::Pending => "queued",
    }
}

fn format_tokens(tokens: u64) -> String {
    let digits = tokens.to_string();
    let mut with_commas = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().rev().enumerate() {
        if index != 0 && index % 3 == 0 {
            with_commas.push(',');
        }
        with_commas.push(ch);
    }
    with_commas.chars().rev().collect()
}

fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub async fn cmd_webhook_test(cards_dir: &Path) -> anyhow::Result<()> {
    let client = WebhookClient::from_cards_dir(cards_dir)?;
    if !client.enabled() {
        println!(
            "No webhooks configured in {}",
            cards_dir.join(".bop").join("config.json").display()
        );
        return Ok(());
    }

    for result in client.send_test().await {
        println!(
            "Sending test to {} -> {}",
            result.url,
            result.status_label()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode as AxumStatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use chrono::Utc;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    struct TestState {
        tx: mpsc::UnboundedSender<Value>,
    }

    async fn hook_receiver(
        State(state): State<TestState>,
        Json(payload): Json<Value>,
    ) -> AxumStatusCode {
        let _ = state.tx.send(payload);
        AxumStatusCode::OK
    }

    async fn spawn_mock_server() -> Option<(
        String,
        mpsc::UnboundedReceiver<Value>,
        tokio::task::JoinHandle<()>,
    )> {
        let (tx, rx) = mpsc::unbounded_channel();
        let app = Router::new()
            .route("/hook", post(hook_receiver))
            .with_state(TestState { tx });

        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(_) => return None,
        };
        let addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(_) => return None,
        };
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Some((format!("http://{addr}/hook"), rx, handle))
    }

    fn sample_meta() -> Meta {
        let mut meta = Meta {
            id: "team-arch/spec-041".to_string(),
            stage: "implement".to_string(),
            created: Utc::now(),
            ..Default::default()
        };
        meta.runs.push(RunRecord {
            run_id: "run-1".to_string(),
            stage: "implement".to_string(),
            provider: "codex".to_string(),
            model: "gpt-5-codex".to_string(),
            adapter: "adapters/codex.nu".to_string(),
            started_at: Utc::now().to_rfc3339(),
            ended_at: Some(Utc::now().to_rfc3339()),
            outcome: "success".to_string(),
            prompt_tokens: Some(10000),
            completion_tokens: Some(4200),
            cost_usd: Some(0.18),
            duration_s: Some(291),
            note: None,
        });
        meta
    }

    #[tokio::test]
    async fn send_matching_honors_on_filter_and_json_payload() {
        let Some((url, mut rx, handle)) = spawn_mock_server().await else {
            return;
        };

        let webhooks = vec![
            WebhookConfig {
                url: url.clone(),
                on: vec![WebhookEvent::Done],
                format: WebhookFormat::Json,
            },
            WebhookConfig {
                url,
                on: vec![WebhookEvent::Failed],
                format: WebhookFormat::Json,
            },
        ];

        let td = tempfile::tempdir().unwrap();
        let client = WebhookClient::from_webhooks_for_test(webhooks, td.path().join("webhook.log"));
        let event = WebhookEventData::from_meta(WebhookEvent::Done, Some(&sample_meta()));

        let results = client
            .send_matching_with_log(&event, &td.path().join("webhook.log"), false)
            .await;

        assert_eq!(results.len(), 1);
        assert!(results[0].status.unwrap().is_success());

        let payload = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(payload["event"], "done");
        assert_eq!(payload["card_id"], "team-arch/spec-041");
        assert_eq!(payload["state"], "done");
        assert_eq!(payload["provider"], "codex");
        assert_eq!(payload["cost_usd"], 0.18);
        assert_eq!(payload["tokens"], 14200);
        assert_eq!(payload["duration_secs"], 291);
        assert!(payload["timestamp"].as_str().is_some());

        handle.abort();
    }

    #[tokio::test]
    async fn slack_payload_posts_block_kit_shape() {
        let Some((url, mut rx, handle)) = spawn_mock_server().await else {
            return;
        };
        let webhooks = vec![WebhookConfig {
            url,
            on: vec![WebhookEvent::Merged],
            format: WebhookFormat::Slack,
        }];

        let td = tempfile::tempdir().unwrap();
        let client = WebhookClient::from_webhooks_for_test(webhooks, td.path().join("webhook.log"));
        let event = WebhookEventData::from_meta(WebhookEvent::Merged, Some(&sample_meta()));

        let results = client
            .send_matching_with_log(&event, &td.path().join("webhook.log"), false)
            .await;
        assert_eq!(results.len(), 1);
        assert!(results[0].status.unwrap().is_success());

        let payload = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert!(payload["text"].as_str().is_some());
        assert_eq!(payload["blocks"][0]["type"], "section");
        assert_eq!(payload["blocks"][0]["text"]["type"], "mrkdwn");
        assert!(payload["blocks"][0]["text"]["text"].as_str().is_some());

        handle.abort();
    }
}
