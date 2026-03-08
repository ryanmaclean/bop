use anyhow::Context;
use serde_json::Value;

use super::{emit, BridgeEvent, CardStage};

/// Connect to opencode's SSE bus and translate events to BridgeEvents.
///
/// GET `http://localhost:{port}/event`
/// Events:
/// - `session.status` (idle/busy/retry) -> `StageChange` or `AwaitingHuman`
/// - `message.part.delta` -> `ToolStart`
/// - `permission.updated` -> `AwaitingHuman`
pub async fn listen_opencode(port: u16) -> anyhow::Result<()> {
    let url = format!("http://localhost:{port}/event");
    let client = reqwest::Client::new();
    let mut resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .with_context(|| format!("failed to connect to opencode SSE at {url}"))?;

    anyhow::ensure!(
        resp.status().is_success(),
        "opencode SSE returned HTTP {}",
        resp.status()
    );

    let mut buffer = String::new();
    let mut event_name: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();

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
                dispatch_event(event_name.as_deref(), &data_lines.join("\n"));
                event_name = None;
                data_lines.clear();
                continue;
            }

            if line.starts_with(':') {
                continue;
            }
            if let Some(name) = line.strip_prefix("event:") {
                event_name = Some(name.trim().to_string());
                continue;
            }
            if let Some(data) = line.strip_prefix("data:") {
                data_lines.push(data.trim_start().to_string());
            }
        }
    }

    // Flush pending event if the stream ends without a final blank line.
    if event_name.is_some() || !data_lines.is_empty() {
        dispatch_event(event_name.as_deref(), &data_lines.join("\n"));
    }

    Ok(())
}

fn dispatch_event(event_name: Option<&str>, data: &str) {
    let Some(event_name) = event_name else {
        return;
    };
    let Some(event) = translate_sse_event(event_name, data) else {
        return;
    };
    let _ = emit(&event);
}

fn translate_sse_event(event_name: &str, data: &str) -> Option<BridgeEvent> {
    let payload = parse_payload(data);
    let session_id = extract_session_id(payload.as_ref()).unwrap_or_else(|| "unknown".to_string());
    let card_id = extract_card_id(payload.as_ref());

    match event_name {
        "session.status" => {
            let status = extract_status(payload.as_ref(), data).to_ascii_lowercase();
            match status.as_str() {
                "busy" => Some(BridgeEvent::StageChange {
                    cli: "opencode".to_string(),
                    session_id,
                    card_id,
                    stage: CardStage::InProgress,
                }),
                "idle" => Some(BridgeEvent::StageChange {
                    cli: "opencode".to_string(),
                    session_id,
                    card_id,
                    stage: CardStage::HumanReview,
                }),
                "retry" => Some(BridgeEvent::AwaitingHuman {
                    cli: "opencode".to_string(),
                    session_id,
                    reason: Some("retry".to_string()),
                }),
                _ => None,
            }
        }
        "message.part.delta" => Some(BridgeEvent::ToolStart {
            cli: "opencode".to_string(),
            session_id,
            tool: extract_tool(payload.as_ref())
                .unwrap_or_else(|| "message.part.delta".to_string()),
        }),
        "permission.updated" => Some(BridgeEvent::AwaitingHuman {
            cli: "opencode".to_string(),
            session_id,
            reason: extract_reason(payload.as_ref())
                .or_else(|| (!data.trim().is_empty()).then(|| data.trim().to_string())),
        }),
        _ => None,
    }
}

fn parse_payload(data: &str) -> Option<Value> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed).ok()
}

fn extract_status(payload: Option<&Value>, fallback: &str) -> String {
    payload
        .and_then(|p| {
            extract_string(
                p,
                &[
                    &["status"],
                    &["state"],
                    &["session", "status"],
                    &["payload", "status"],
                ],
            )
        })
        .unwrap_or_else(|| fallback.trim().to_string())
}

fn extract_session_id(payload: Option<&Value>) -> Option<String> {
    payload.and_then(|p| {
        extract_string(
            p,
            &[
                &["session_id"],
                &["sessionId"],
                &["session", "id"],
                &["payload", "session_id"],
                &["id"],
            ],
        )
    })
}

fn extract_card_id(payload: Option<&Value>) -> Option<String> {
    payload.and_then(|p| {
        extract_string(
            p,
            &[
                &["card_id"],
                &["cardId"],
                &["card", "id"],
                &["payload", "card_id"],
            ],
        )
    })
}

fn extract_tool(payload: Option<&Value>) -> Option<String> {
    payload.and_then(|p| {
        extract_string(
            p,
            &[
                &["tool"],
                &["tool_name"],
                &["toolName"],
                &["name"],
                &["part", "tool"],
                &["part", "name"],
                &["payload", "tool"],
            ],
        )
    })
}

fn extract_reason(payload: Option<&Value>) -> Option<String> {
    payload.and_then(|p| {
        extract_string(
            p,
            &[
                &["reason"],
                &["message"],
                &["status"],
                &["permission", "status"],
                &["action"],
            ],
        )
    })
}

fn extract_string(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value_at_path(value, path))
        .and_then(|v| match v {
            Value::String(s) => Some(s.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_status_busy_maps_to_in_progress() {
        let event = translate_sse_event(
            "session.status",
            r#"{"session_id":"s1","card_id":"c1","status":"busy"}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            BridgeEvent::StageChange {
                cli: "opencode".to_string(),
                session_id: "s1".to_string(),
                card_id: Some("c1".to_string()),
                stage: CardStage::InProgress,
            }
        );
    }

    #[test]
    fn session_status_retry_maps_to_awaiting_human() {
        let event =
            translate_sse_event("session.status", r#"{"session_id":"s2","status":"retry"}"#)
                .unwrap();
        assert_eq!(
            event,
            BridgeEvent::AwaitingHuman {
                cli: "opencode".to_string(),
                session_id: "s2".to_string(),
                reason: Some("retry".to_string()),
            }
        );
    }

    #[test]
    fn permission_updated_maps_to_awaiting_human_with_reason() {
        let event = translate_sse_event(
            "permission.updated",
            r#"{"session_id":"s3","reason":"tool-confirmation"}"#,
        )
        .unwrap();
        assert_eq!(
            event,
            BridgeEvent::AwaitingHuman {
                cli: "opencode".to_string(),
                session_id: "s3".to_string(),
                reason: Some("tool-confirmation".to_string()),
            }
        );
    }
}
