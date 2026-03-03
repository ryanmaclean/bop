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
}
