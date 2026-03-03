//! OpenLineage-compatible event emission for card state transitions.
//!
//! Events are collected in a `Vec<RunEvent>` during a loop iteration
//! and flushed once via `flush_events()`. See the `bop-on` skill for
//! the O(N) design rule: never add per-item I/O in a hot loop.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::Meta;

const PRODUCER: &str = "https://github.com/yourorg/bop";
const SCHEMA_URL: &str = "https://openlineage.io/spec/2-0-2/OpenLineage.json#/$defs/RunEvent";

// ── OpenLineage types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    Start,
    Running,
    Complete,
    Fail,
    Abort,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub event_type: EventType,
    pub event_time: DateTime<Utc>,
    pub run: Run,
    pub job: Job,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Value>,
    pub producer: String,
    pub schema_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facets: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub namespace: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facets: BTreeMap<String, Value>,
}

// ── Event construction ───────────────────────────────────────────────────────

/// Map a (from_state, to_state) pair to an OpenLineage EventType.
pub fn event_type_for(from: &str, to: &str) -> EventType {
    match (from, to) {
        ("pending", "running") => EventType::Start,
        ("running", "done") => EventType::Complete,
        ("running", "failed") => EventType::Fail,
        ("done", "merged") => EventType::Complete,
        ("done", "failed") => EventType::Fail,
        _ => EventType::Other,
    }
}

/// Build an OpenLineage RunEvent from a card's Meta and transition info.
pub fn build_run_event(
    event_type: EventType,
    meta: &Meta,
    from_state: &str,
    to_state: &str,
) -> RunEvent {
    build_run_event_with_dir(event_type, meta, from_state, to_state, None)
}

/// Build an OpenLineage RunEvent with optional visual facet for observers.
///
/// When `card_dir` is provided, a `bop_visual` facet is added containing
/// glyph, token, thumbnail path, cost, and health — everything an external
/// observer (e.g. a Dynamic Island notification app) needs for rich rendering.
pub fn build_run_event_with_dir(
    event_type: EventType,
    meta: &Meta,
    from_state: &str,
    to_state: &str,
    card_dir: Option<&std::path::Path>,
) -> RunEvent {
    let run_id = meta
        .runs
        .last()
        .map(|r| r.run_id.clone())
        .unwrap_or_else(|| meta.id.clone());

    let mut run_facets: BTreeMap<String, Value> = BTreeMap::new();

    // processingEngine facet
    if let Some(last_run) = meta.runs.last() {
        let engine = serde_json::json!({
            "_producer": PRODUCER,
            "_schemaURL": "https://openlineage.io/spec/facets/1-1-1/ProcessingEngineRunFacet.json",
            "version": "1.0",
            "name": last_run.provider,
            "openlineageAdapterVersion": last_run.adapter,
        });
        run_facets.insert("processing_engine".into(), engine);
    }

    // parent facet (from depends_on or stage_chain lineage)
    if !meta.depends_on.is_empty() {
        let parent = serde_json::json!({
            "_producer": PRODUCER,
            "_schemaURL": "https://openlineage.io/spec/facets/1-0-1/ParentRunFacet.json",
            "run": { "runId": meta.depends_on.first() },
            "job": { "namespace": "bop", "name": meta.depends_on.first() },
        });
        run_facets.insert("parent".into(), parent);
    }

    // bop-specific transition facet
    let previous_run_id: Option<&str> = meta.runs.iter().rev().nth(1).map(|r| r.run_id.as_str());
    let bop_facet = serde_json::json!({
        "_producer": PRODUCER,
        "fromState": from_state,
        "toState": to_state,
        "stage": meta.stage,
        "retryCount": meta.retry_count.unwrap_or(0),
        "failureReason": meta.failure_reason,
        "previousRunId": previous_run_id,
    });
    run_facets.insert("bop_transition".into(), bop_facet);

    // bop_visual facet — everything an observer needs for rich notifications
    if let Some(dir) = card_dir {
        let last_run = meta.runs.last();
        let tokens_used: Option<u64> =
            last_run.and_then(|r| match (r.prompt_tokens, r.completion_tokens) {
                (Some(p), Some(c)) => Some(p + c),
                (Some(p), None) => Some(p),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            });
        let visual = serde_json::json!({
            "_producer": PRODUCER,
            "glyph": meta.glyph,
            "token": meta.token,
            "thumbnailPath": dir.join("QuickLook/Thumbnail.png").to_string_lossy(),
            "cardDir": dir.to_string_lossy(),
            "tokensUsed": tokens_used,
            "costUsd": last_run.and_then(|r| r.cost_usd),
            "health": meta.validation_summary.as_ref().map(|s| s.badge()),
        });
        run_facets.insert("bop_visual".into(), visual);
    }

    // job facets
    let mut job_facets: BTreeMap<String, Value> = BTreeMap::new();

    if let Some(ref branch) = meta.worktree_branch {
        let source = serde_json::json!({
            "_producer": PRODUCER,
            "_schemaURL": "https://openlineage.io/spec/facets/1-0-1/SourceCodeLocationJobFacet.json",
            "type": "git",
            "url": "",
            "branch": branch,
        });
        job_facets.insert("sourceCodeLocation".into(), source);
    }

    RunEvent {
        event_type,
        event_time: Utc::now(),
        run: Run {
            run_id,
            facets: run_facets,
        },
        job: Job {
            namespace: "bop".into(),
            name: meta.id.clone(),
            facets: job_facets,
        },
        inputs: Vec::new(),
        outputs: Vec::new(),
        producer: PRODUCER.into(),
        schema_url: SCHEMA_URL.into(),
    }
}

// ── iCalendar VEVENT projection ─────────────────────────────────────────────

/// Clean an ISO-8601 timestamp into iCalendar `yyyymmddThhmmssZ` format.
fn ical_timestamp(iso: &str) -> String {
    let s = iso.replace(['-', ':'], "");
    let s = s.trim_end_matches('Z');
    // Strip timezone offset if present (e.g. +0000)
    let s = if let Some(pos) = s.find('+') {
        &s[..pos]
    } else {
        s
    };
    // Only strip trailing minus if it looks like a tz offset (after the T)
    let s = if let Some(pos) = s.rfind('-') {
        if pos > 8 {
            &s[..pos]
        } else {
            s
        }
    } else {
        s
    };
    // Truncate fractional seconds (everything after a dot)
    let s = if let Some(pos) = s.find('.') {
        &s[..pos]
    } else {
        s
    };
    format!("{}Z", s)
}

/// Write a `card.ics` VEVENT into the jobcard bundle directory.
///
/// Uses VEVENT (not VTODO) so Apple Calendar renders timeline bars.
/// Each card run becomes a time-span event: DTSTART → DTEND.
/// State is encoded in the SUMMARY prefix and X-BOP-STATE.
///
/// STATUS mapping in SUMMARY:
///   running  → "▶ card-id"
///   done     → "✓ card-id"
///   failed   → "✗ card-id"
///   merged   → "⤴ card-id"
///   pending  → "◇ card-id"
pub fn write_ics(card_dir: &Path, meta: &Meta, to_state: &str) {
    let state_prefix = match to_state {
        "running" => "▶",
        "done" => "✓",
        "failed" => "✗",
        "merged" => "⤴",
        _ => "◇",
    };

    // VEVENT STATUS: CONFIRMED for active/done, CANCELLED for failed
    let status = match to_state {
        "failed" => "CANCELLED",
        _ => "CONFIRMED",
    };

    // DTSTART: first run start, or card creation
    let dtstart = meta
        .runs
        .first()
        .filter(|r| !r.started_at.is_empty())
        .map(|r| ical_timestamp(&r.started_at))
        .unwrap_or_else(|| meta.created.format("%Y%m%dT%H%M%SZ").to_string());

    // DTEND: last run end, or now (for running/pending)
    let dtend = meta
        .runs
        .last()
        .and_then(|r| r.ended_at.as_ref())
        .map(|t| ical_timestamp(t))
        .unwrap_or_else(|| Utc::now().format("%Y%m%dT%H%M%SZ").to_string());

    // Summary: state prefix + glyph + card id
    let summary = match &meta.glyph {
        Some(g) => format!("{} {} {}", state_prefix, g, meta.id),
        None => format!("{} {}", state_prefix, meta.id),
    };

    // Description
    let provider = meta
        .runs
        .last()
        .map(|r| r.provider.as_str())
        .unwrap_or("unknown");
    let mut desc_parts = vec![
        format!("Stage: {}", meta.stage),
        format!("Provider: {}", provider),
        format!("State: {}", to_state),
    ];
    if let Some(ref reason) = meta.failure_reason {
        desc_parts.push(format!("Failure: {}", reason));
    }
    if let Some(last) = meta.runs.last() {
        if let (Some(p), Some(c)) = (last.prompt_tokens, last.completion_tokens) {
            desc_parts.push(format!("Tokens: {}", p + c));
        }
        if let Some(cost) = last.cost_usd {
            desc_parts.push(format!("Cost: ${:.2}", cost));
        }
    }
    let description = desc_parts.join("\\n");

    let categories = if meta.stage_chain.is_empty() {
        meta.stage.clone()
    } else {
        meta.stage_chain.join(",")
    };

    let sequence = meta.retry_count.unwrap_or(0);
    let priority = meta.priority.map(|p| p.clamp(0, 9) as u8).unwrap_or(0);

    let mut ics = String::with_capacity(512);
    ics.push_str("BEGIN:VCALENDAR\r\n");
    ics.push_str("VERSION:2.0\r\n");
    ics.push_str("PRODID:-//bop//jobcard//EN\r\n");
    ics.push_str("X-WR-CALNAME:bop agents\r\n");
    ics.push_str("BEGIN:VEVENT\r\n");
    ics.push_str(&format!("UID:{}@bop\r\n", meta.id));
    ics.push_str(&format!(
        "DTSTAMP:{}Z\r\n",
        Utc::now().format("%Y%m%dT%H%M%S")
    ));
    ics.push_str(&format!("DTSTART:{}\r\n", dtstart));
    ics.push_str(&format!("DTEND:{}\r\n", dtend));
    ics.push_str(&format!("SUMMARY:{}\r\n", summary));
    ics.push_str(&format!("DESCRIPTION:{}\r\n", description));
    ics.push_str(&format!("STATUS:{}\r\n", status));
    ics.push_str(&format!("PRIORITY:{}\r\n", priority));
    ics.push_str(&format!("SEQUENCE:{}\r\n", sequence));
    ics.push_str(&format!("CATEGORIES:{}\r\n", categories));
    ics.push_str("TRANSP:TRANSPARENT\r\n"); // don't block free/busy

    // Extended properties
    ics.push_str(&format!("X-BOP-STATE:{}\r\n", to_state));
    ics.push_str(&format!("X-BOP-STAGE:{}\r\n", meta.stage));
    if let Some(ref token) = meta.token {
        ics.push_str(&format!("X-BOP-TOKEN:{}\r\n", token));
    }
    if let Some(ref branch) = meta.worktree_branch {
        ics.push_str(&format!("X-BOP-BRANCH:{}\r\n", branch));
    }
    if let Some(ref reason) = meta.failure_reason {
        ics.push_str(&format!("X-BOP-FAILURE:{}\r\n", reason));
    }
    if let Some(last) = meta.runs.last() {
        if let Some(tokens) = last
            .prompt_tokens
            .zip(last.completion_tokens)
            .map(|(p, c)| p + c)
        {
            ics.push_str(&format!("X-BOP-TOKENS:{}\r\n", tokens));
        }
        if let Some(cost) = last.cost_usd {
            ics.push_str(&format!("X-BOP-COST:{:.4}\r\n", cost));
        }
    }
    for dep in &meta.depends_on {
        ics.push_str(&format!("RELATED-TO;RELTYPE=PARENT:{}@bop\r\n", dep));
    }

    ics.push_str("END:VEVENT\r\n");
    ics.push_str("END:VCALENDAR\r\n");

    let ics_path = card_dir.join("card.ics");
    let _ = fs::write(&ics_path, ics);
}

// ── Opt-in gate ──────────────────────────────────────────────────────────────

/// Check if lineage emission is enabled.
/// Returns true if `OPENLINEAGE_URL` is set OR `.cards/hooks.toml` exists.
/// Called once per loop iteration — not per card.
pub fn is_enabled(cards_dir: &Path) -> bool {
    std::env::var("OPENLINEAGE_URL").is_ok() || cards_dir.join("hooks.toml").exists()
}

// ── Batched flush ────────────────────────────────────────────────────────────

/// Append all events to `.cards/events.jsonl` in a single write.
/// If `OPENLINEAGE_URL` is set, fire-and-forget POST the batch.
pub fn flush_events(cards_dir: &Path, events: &[RunEvent]) {
    if events.is_empty() {
        return;
    }

    // Serialize all events into a single buffer
    let mut buf = String::new();
    for ev in events {
        if let Ok(line) = serde_json::to_string(ev) {
            buf.push_str(&line);
            buf.push('\n');
        }
    }

    // Single file append
    let events_path = cards_dir.join("events.jsonl");
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&events_path)
        .and_then(|mut f| f.write_all(buf.as_bytes()));

    // Fire-and-forget HTTP POST (one curl for entire batch)
    if let Ok(url) = std::env::var("OPENLINEAGE_URL") {
        let endpoint = format!("{}/api/v1/lineage", url);
        // POST each event as a separate request is the OL spec expectation,
        // but we batch into a single curl invocation with --next for efficiency.
        // For simplicity, post the entire JSONL as one body.
        let _ = std::process::Command::new("curl")
            .args([
                "-sS",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/x-ndjson",
                "-d",
                &buf,
                &endpoint,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    // Fire-and-forget Unix socket (for local observers like BopDeck)
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
        let socket_path = format!("/tmp/bop-deck-{}.sock", user);
        if let Ok(mut stream) = UnixStream::connect(&socket_path) {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));
            let _ = stream.write_all(buf.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Meta, RunRecord};

    fn test_meta() -> Meta {
        Meta {
            id: "test-card".into(),
            stage: "implement".into(),
            created: Utc::now(),
            runs: vec![RunRecord {
                run_id: "abc12345".into(),
                stage: "implement".into(),
                provider: "claude".into(),
                model: "claude-sonnet-4-6".into(),
                adapter: "adapters/claude.zsh".into(),
                started_at: "2026-03-02T00:00:00Z".into(),
                outcome: "success".into(),
                ..Default::default()
            }],
            worktree_branch: Some("job/test-card".into()),
            depends_on: vec!["parent-card".into()],
            ..Default::default()
        }
    }

    #[test]
    fn event_type_mapping() {
        assert_eq!(event_type_for("pending", "running"), EventType::Start);
        assert_eq!(event_type_for("running", "done"), EventType::Complete);
        assert_eq!(event_type_for("running", "failed"), EventType::Fail);
        assert_eq!(event_type_for("done", "merged"), EventType::Complete);
        assert_eq!(event_type_for("done", "failed"), EventType::Fail);
        assert_eq!(event_type_for("running", "pending"), EventType::Other);
        assert_eq!(event_type_for("drafts", "pending"), EventType::Other);
    }

    #[test]
    fn build_event_serializes_to_json() {
        let meta = test_meta();
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");

        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"eventType\":\"START\""));
        assert!(json.contains("\"runId\":\"abc12345\""));
        assert!(json.contains("\"namespace\":\"bop\""));
        assert!(json.contains("\"name\":\"test-card\""));
    }

    #[test]
    fn build_event_includes_processing_engine_facet() {
        let meta = test_meta();
        let ev = build_run_event(EventType::Complete, &meta, "running", "done");

        let facet = &ev.run.facets["processing_engine"];
        assert_eq!(facet["name"], "claude");
        assert_eq!(facet["openlineageAdapterVersion"], "adapters/claude.zsh");
    }

    #[test]
    fn build_event_includes_parent_facet() {
        let meta = test_meta();
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");

        let parent = &ev.run.facets["parent"];
        assert_eq!(parent["run"]["runId"], "parent-card");
    }

    #[test]
    fn build_event_includes_bop_transition_facet() {
        let meta = test_meta();
        let ev = build_run_event(EventType::Fail, &meta, "running", "failed");

        let bop = &ev.run.facets["bop_transition"];
        assert_eq!(bop["fromState"], "running");
        assert_eq!(bop["toState"], "failed");
        assert_eq!(bop["stage"], "implement");
    }

    #[test]
    fn build_event_includes_source_code_location_facet() {
        let meta = test_meta();
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");

        let source = &ev.job.facets["sourceCodeLocation"];
        assert_eq!(source["branch"], "job/test-card");
    }

    #[test]
    fn build_event_no_runs_uses_card_id() {
        let mut meta = test_meta();
        meta.runs.clear();
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");
        assert_eq!(ev.run.run_id, "test-card");
    }

    #[test]
    fn flush_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();
        let events = vec![
            build_run_event(EventType::Start, &meta, "pending", "running"),
            build_run_event(EventType::Complete, &meta, "running", "done"),
        ];

        flush_events(dir.path(), &events);

        let content = fs::read_to_string(dir.path().join("events.jsonl")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"START\""));
        assert!(lines[1].contains("\"COMPLETE\""));
    }

    #[test]
    fn flush_empty_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        flush_events(dir.path(), &[]);
        assert!(!dir.path().join("events.jsonl").exists());
    }

    #[test]
    fn is_enabled_respects_env() {
        let dir = tempfile::tempdir().unwrap();
        // Neither env var nor hooks.toml
        std::env::remove_var("OPENLINEAGE_URL");
        assert!(!is_enabled(dir.path()));
    }

    #[test]
    fn is_enabled_respects_hooks_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::env::remove_var("OPENLINEAGE_URL");
        fs::write(dir.path().join("hooks.toml"), "").unwrap();
        assert!(is_enabled(dir.path()));
    }

    #[test]
    fn build_event_includes_previous_run_id() {
        let mut meta = test_meta();
        // Add a second run so the first becomes "previous"
        meta.runs.push(RunRecord {
            run_id: "def67890".into(),
            stage: "implement".into(),
            provider: "claude".into(),
            model: "claude-sonnet-4-6".into(),
            adapter: "adapters/claude.zsh".into(),
            started_at: "2026-03-02T01:00:00Z".into(),
            outcome: "running".into(),
            ..Default::default()
        });
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");
        let bop = &ev.run.facets["bop_transition"];
        assert_eq!(bop["previousRunId"], "abc12345");
    }

    #[test]
    fn build_event_no_previous_run_on_first_attempt() {
        let meta = test_meta(); // single run
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");
        let bop = &ev.run.facets["bop_transition"];
        assert!(bop["previousRunId"].is_null());
    }

    #[test]
    fn run_event_roundtrips_json() {
        let meta = test_meta();
        let ev = build_run_event(EventType::Start, &meta, "pending", "running");
        let json = serde_json::to_string(&ev).unwrap();
        let parsed: RunEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, EventType::Start);
        assert_eq!(parsed.run.run_id, "abc12345");
        assert_eq!(parsed.job.name, "test-card");
    }

    // ── iCalendar VEVENT tests ───────────────────────────────────────────────

    #[test]
    fn write_ics_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();
        write_ics(dir.path(), &meta, "running");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("BEGIN:VCALENDAR"));
        assert!(ics.contains("BEGIN:VEVENT"));
        assert!(ics.contains("END:VEVENT"));
        assert!(ics.contains("END:VCALENDAR"));
        assert!(ics.contains("TRANSP:TRANSPARENT"));
    }

    #[test]
    fn ics_status_maps_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();

        // VEVENT uses CONFIRMED for active states, CANCELLED for failed
        write_ics(dir.path(), &meta, "pending");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("STATUS:CONFIRMED"));
        assert!(ics.contains("SUMMARY:◇"));

        write_ics(dir.path(), &meta, "running");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("STATUS:CONFIRMED"));
        assert!(ics.contains("SUMMARY:▶"));

        write_ics(dir.path(), &meta, "done");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("STATUS:CONFIRMED"));
        assert!(ics.contains("SUMMARY:✓"));

        write_ics(dir.path(), &meta, "failed");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("STATUS:CANCELLED"));
        assert!(ics.contains("SUMMARY:✗"));

        write_ics(dir.path(), &meta, "merged");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("STATUS:CONFIRMED"));
        assert!(ics.contains("SUMMARY:⤴"));
    }

    #[test]
    fn ics_includes_uid_and_summary() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();
        write_ics(dir.path(), &meta, "running");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("UID:test-card@bop"));
        assert!(ics.contains("SUMMARY:▶"));
        assert!(ics.contains("test-card"));
    }

    #[test]
    fn ics_includes_dtstart_and_dtend() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();
        write_ics(dir.path(), &meta, "done");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("DTSTART:20260302T000000Z"));
        assert!(ics.contains("DTEND:"));
    }

    #[test]
    fn ics_includes_related_to_for_depends() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();
        write_ics(dir.path(), &meta, "running");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("RELATED-TO;RELTYPE=PARENT:parent-card@bop"));
    }

    #[test]
    fn ics_includes_bop_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let meta = test_meta();
        write_ics(dir.path(), &meta, "running");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("X-BOP-STATE:running"));
        assert!(ics.contains("X-BOP-STAGE:implement"));
        assert!(ics.contains("X-BOP-BRANCH:job/test-card"));
    }

    #[test]
    fn ics_failure_includes_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut meta = test_meta();
        meta.failure_reason = Some("timeout".into());
        write_ics(dir.path(), &meta, "failed");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("STATUS:CANCELLED"));
        assert!(ics.contains("X-BOP-FAILURE:timeout"));
        assert!(ics.contains("Failure: timeout"));
    }

    #[test]
    fn ics_retry_sets_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let mut meta = test_meta();
        meta.retry_count = Some(3);
        write_ics(dir.path(), &meta, "running");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("SEQUENCE:3"));
    }

    #[test]
    fn ics_with_tokens_and_cost() {
        let dir = tempfile::tempdir().unwrap();
        let mut meta = test_meta();
        meta.runs[0].prompt_tokens = Some(1000);
        meta.runs[0].completion_tokens = Some(500);
        meta.runs[0].cost_usd = Some(0.42);
        write_ics(dir.path(), &meta, "done");
        let ics = fs::read_to_string(dir.path().join("card.ics")).unwrap();
        assert!(ics.contains("X-BOP-TOKENS:1500"));
        assert!(ics.contains("X-BOP-COST:0.42"));
        assert!(ics.contains("Tokens: 1500"));
    }

    #[test]
    fn ical_timestamp_cleans_iso() {
        assert_eq!(ical_timestamp("2026-03-02T00:00:00Z"), "20260302T000000Z");
        assert_eq!(
            ical_timestamp("2026-03-02T15:30:45+0000"),
            "20260302T153045Z"
        );
        assert_eq!(
            ical_timestamp("2026-03-02T15:30:45.123Z"),
            "20260302T153045Z"
        );
    }
}
