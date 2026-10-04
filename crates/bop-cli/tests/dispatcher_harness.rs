use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Find a card by bare id in a state dir, handling glyph-prefixed names.
fn find_card_in(cards: &Path, state: &str, id: &str) -> PathBuf {
    let dir = cards.join(state);
    let suffix = format!("-{}.bop", id);
    let exact = format!("{}.bop", id);
    if dir.join(&exact).exists() {
        return dir.join(exact);
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_str().map(|n| n.ends_with(&suffix)).unwrap_or(false) {
                return dir.join(name);
            }
        }
    }
    dir.join(exact) // return non-existent path so assert gives useful message
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_jc() {
    // No-op: env!("CARGO_BIN_EXE_bop") below makes cargo test build the
    // `bop` binary itself before any test runs, exactly once, in the
    // correct (possibly redirected) CARGO_TARGET_DIR. Calling `cargo build`
    // again per-test raced concurrent test threads against the same
    // output binary (rename/unlink window), causing sporadic
    // "No such file or directory" failures when tests ran in parallel.
}

fn bop_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bop"))
}

fn mock_adapter() -> PathBuf {
    repo_root().join("adapters").join("mock.nu")
}

fn write_providers(cards: &Path) {
    let cmd = mock_adapter().to_str().unwrap().replace('\\', "\\\\");
    let json = format!(
        "{{\"providers\":{{\"mock\":{{\"command\":\"{}\",\"rate_limit_exit\":75}},\"mock2\":{{\"command\":\"{}\",\"rate_limit_exit\":75}}}}}}",
        cmd, cmd
    );
    fs::write(cards.join("providers.json"), json).unwrap();
}

fn write_template(cards: &Path, template: &str) {
    let tdir = cards.join("templates").join(format!("{}.bop", template));
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(tdir.join("meta.json"), "{\"id\":\"t\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"implement\",\"provider_chain\":[\"mock\",\"mock2\"],\"stages\":{},\"acceptance_criteria\":[]}").unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();
}

fn write_running_card_with_stale_lease(cards: &Path, id: &str) {
    let card = cards.join("running").join(format!("{id}.bop"));
    fs::create_dir_all(card.join("logs")).unwrap();
    fs::create_dir_all(card.join("output")).unwrap();
    fs::write(
        card.join("meta.json"),
        format!(
            r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{{"implement":{{"status":"running","agent":"adapters/mock.nu","provider":"mock"}}}},"acceptance_criteria":[],"retry_count":0}}"#
        ),
    )
    .unwrap();
    fs::write(card.join("spec.md"), "stale lease").unwrap();
    fs::write(
        card.join("logs").join("lease.json"),
        format!(
            r#"{{"run_id":"stale-run","pid":{},"pid_start_time":"2026-03-01T00:00:00Z","started_at":"2026-03-01T00:00:00Z","heartbeat_at":"2026-03-01T00:00:00Z","host":"test-host"}}"#,
            std::process::id()
        ),
    )
    .unwrap();
}

fn write_invalid_pending_card(cards: &Path, id: &str) {
    let card = cards.join("pending").join(format!("{id}.bop"));
    fs::create_dir_all(card.join("logs")).unwrap();
    fs::create_dir_all(card.join("output")).unwrap();
    fs::write(
        card.join("meta.json"),
        format!(
            r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"","provider_chain":["mock"],"stages":{{}},"acceptance_criteria":[]}}"#
        ),
    )
    .unwrap();
    fs::write(card.join("spec.md"), "invalid").unwrap();
    fs::write(card.join("prompt.md"), "{{spec}}\n").unwrap();
}

#[test]
fn dispatcher_moves_success_to_done() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);

    write_template(&cards, "implement");

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "job1",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "0")
        .env("BOP_RUN_ID", "spoofed-parent-id")
        .env("MOCK_ECHO_BOP_RUN_ID", "1")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let card = find_card_in(&cards, "done", "job1");
    assert!(card.exists());
    let logs_webloc = fs::read_to_string(card.join("Logs.webloc")).unwrap();
    assert!(
        logs_webloc.contains("bop://card/job1/logs"),
        "done cards should link to static logs action"
    );

    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(card.join("meta.json")).unwrap()).unwrap();
    let run_record_id = meta["runs"][0]["run_id"].as_str().unwrap();
    assert!(!run_record_id.is_empty());
    assert_ne!(run_record_id, "spoofed-parent-id");
    let stdout = fs::read_to_string(card.join("logs/stdout.log")).unwrap();
    let exported_line = format!("BOP_RUN_ID={run_record_id}");
    assert!(
        stdout.lines().any(|line| line == exported_line.as_str()),
        "adapter did not receive persisted run identity: {stdout}"
    );
}

#[test]
fn dispatcher_rate_limit_requeues_to_pending() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);

    write_template(&cards, "implement");

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "job2",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "75")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let card = find_card_in(&cards, "pending", "job2");
    assert!(card.exists());
    let logs_webloc = fs::read_to_string(card.join("Logs.webloc")).unwrap();
    assert!(
        logs_webloc.contains("bop://card/job2/tail"),
        "non-done cards should link to live tail action"
    );
}

#[test]
fn dispatcher_rate_limit_sets_cooldown_and_rotates_chain() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "job3",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "75")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let meta_path = find_card_in(&cards, "pending", "job3").join("meta.json");
    let meta = fs::read_to_string(meta_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(v.get("retry_count").and_then(|x| x.as_u64()), Some(1));
    let chain = v
        .get("provider_chain")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(chain.len() >= 2);
    assert_eq!(chain[0].as_str(), Some("mock2"));
    assert_eq!(chain[1].as_str(), Some("mock"));

    let providers = fs::read_to_string(cards.join("providers.json")).unwrap();
    assert!(providers.contains("cooldown_until_epoch_s"));
}

#[test]
fn dispatcher_relative_adapter_path_works() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Use absolute paths in providers.json so the adapter shell script itself
    // is found, but pass a *relative* adapter path to the dispatcher CLI to
    // exercise the relative→absolute conversion in run_card.
    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "rel-job1",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    // Run the dispatcher from repo_root() so that "adapters/mock.nu" resolves.
    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "0")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            "adapters/mock.nu", // relative path
            "--once",
        ])
        .current_dir(repo_root())
        .status()
        .unwrap();
    assert!(
        status.success(),
        "dispatcher failed with relative adapter path"
    );

    // Card moved to done
    let card_dir = find_card_in(&cards, "done", "rel-job1");
    assert!(card_dir.exists(), "card should be in done/");

    // Logs were written
    assert!(
        card_dir.join("logs").join("stdout.log").exists(),
        "stdout.log missing"
    );
    assert!(
        card_dir.join("logs").join("stderr.log").exists(),
        "stderr.log missing"
    );
}

#[test]
fn dispatcher_qa_prefers_different_provider_than_implement() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);

    let tdir = cards.join("templates").join("qa.bop");
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(
        tdir.join("meta.json"),
        "{\"id\":\"t\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"qa\",\"provider_chain\":[\"mock\",\"mock2\"],\"stages\":{\"implement\":{\"status\":\"done\",\"provider\":\"mock\"}},\"acceptance_criteria\":[]}",
    )
    .unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "new", "qa", "job4"])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "0")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let meta_path = find_card_in(&cards, "done", "job4").join("meta.json");
    let meta = fs::read_to_string(meta_path).unwrap();
    assert!(meta.contains("\"qa\""));
    assert!(meta.contains("\"provider\": \"mock2\"") || meta.contains("\"provider\":\"mock2\""));
}

#[test]
fn dispatcher_reaps_stale_lease_without_dead_pid() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_running_card_with_stale_lease(&cards, "lease-stale");

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--max-workers",
            "0",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let card = cards.join("pending").join("lease-stale.bop");
    assert!(
        card.exists(),
        "stale lease card should be moved back to pending"
    );
    let meta = fs::read_to_string(card.join("meta.json")).unwrap();
    assert!(meta.contains("\"retry_count\": 1") || meta.contains("\"retry_count\":1"));
    assert!(
        meta.contains("\"status\": \"pending\"") || meta.contains("\"status\":\"pending\""),
        "running stage should normalize to pending after reaping"
    );
}

#[test]
fn dispatcher_quarantines_invalid_pending_meta_to_failed() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let marker = td.path().join("adapter-ran");
    let marker_adapter = td.path().join("bad-meta-marker.nu");
    fs::write(
        &marker_adapter,
        "def main [workdir: string, prompt_file: string, stdout_log: string, stderr_log: string, ...rest] {\n    'ran' | save --force $env.BOP_TEST_MARKER\n}\n",
    )
    .unwrap();

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    fs::create_dir_all(cards.join(".bop")).unwrap();
    fs::write(cards.join(".bop").join("config.json"), r#"{"webhooks":[]}"#).unwrap();
    write_invalid_pending_card(&cards, "bad-meta");

    let status = Command::new(bop_bin())
        .env("BOP_TEST_MARKER", &marker)
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            marker_adapter.to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(
        !cards.join("pending").join("bad-meta.bop").exists(),
        "invalid card should leave pending"
    );
    let failed = cards.join("failed").join("bad-meta.bop");
    assert!(
        failed.exists(),
        "invalid card should be quarantined in failed/"
    );
    let rejected_log = fs::read_to_string(failed.join("logs").join("rejected.log")).unwrap();
    assert!(
        rejected_log.contains("invalid_meta"),
        "rejected marker should include invalid_meta reason"
    );
    assert!(!marker.exists(), "invalid metadata must not launch the adapter");
    for state in ["pending", "running", "done"] {
        assert!(
            !find_card_in(&cards, state, "bad-meta").exists(),
            "invalid card must not remain in {state}/"
        );
    }
    let failed_count = fs::read_dir(cards.join("failed"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("bop"))
        .count();
    assert_eq!(failed_count, 1, "invalid card must have one failed outcome");
}

#[test]
fn dispatcher_fails_when_live_lock_exists() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let lock_dir = cards.join(".locks").join("dispatcher.lock");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(
        lock_dir.join("owner.json"),
        format!(
            r#"{{"pid":{},"host":"test-host","started_at":"2026-03-01T00:00:00Z"}}"#,
            std::process::id()
        ),
    )
    .unwrap();

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "dispatcher should fail when a live lock is already held"
    );
}

#[test]
fn dispatcher_reclaims_stale_lock_and_runs() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");
    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "stale-lock-job",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let lock_dir = cards.join(".locks").join("dispatcher.lock");
    fs::create_dir_all(&lock_dir).unwrap();
    fs::write(
        lock_dir.join("owner.json"),
        r#"{"pid":999999,"host":"old-host","started_at":"2026-03-01T00:00:00Z"}"#,
    )
    .unwrap();

    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "0")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    assert!(find_card_in(&cards, "done", "stale-lock-job").exists());
}

#[test]
fn dispatcher_emits_lineage_events() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");

    // Enable lineage via hooks.toml
    fs::write(cards.join("hooks.toml"), "").unwrap();

    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "lineage-test",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(bop_bin())
        .env("MOCK_EXIT", "0")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_adapter().to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    // Card should be in done/
    assert!(find_card_in(&cards, "done", "lineage-test").exists());

    // events.jsonl should exist with START + COMPLETE
    let events_path = cards.join("events.jsonl");
    assert!(events_path.exists(), "events.jsonl should be created");

    let content = fs::read_to_string(&events_path).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        lines.len() >= 2,
        "expected at least 2 events (START + COMPLETE), got {}",
        lines.len()
    );

    // Verify we have a START and a COMPLETE
    let has_start = lines.iter().any(|l| l.contains("\"START\""));
    let has_complete = lines.iter().any(|l| l.contains("\"COMPLETE\""));
    assert!(has_start, "expected a START event");
    assert!(has_complete, "expected a COMPLETE event");

    // Verify events reference the right card
    assert!(
        lines.iter().all(|l| l.contains("lineage-test")),
        "all events should reference lineage-test card"
    );

    // Verify run_ids are present and non-empty
    let events: Vec<serde_json::Value> = lines
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    for ev in &events {
        let run_id = ev["run"]["runId"].as_str().unwrap_or_default();
        assert!(!run_id.is_empty(), "every event should have a run_id");
    }
    // COMPLETE event should have the dispatcher-generated run_id (not the card id fallback)
    let complete_run_id = events
        .iter()
        .find(|e| e["eventType"] == "COMPLETE")
        .and_then(|e| e["run"]["runId"].as_str())
        .unwrap();
    assert!(
        !complete_run_id.is_empty(),
        "COMPLETE event should have a run_id"
    );
}

// ── spec 036 / 052: the adapter follows the *selected* provider ─────────────

/// Adapter that records which provider entry actually ran, then succeeds.
fn write_marker_adapter(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(format!("marker-{name}.nu"));
    fs::write(
        &path,
        format!(
            "def main [workdir: string, prompt_file: string, stdout_log: string, stderr_log: string, ...rest] {{\n    \"ran:{name}\\n\" | save --append $stdout_log\n    exit 0\n}}\n"
        ),
    )
    .unwrap();
    path
}

fn write_template_with_chain(cards: &Path, template: &str, chain_json: &str) {
    let tdir = cards.join("templates").join(format!("{}.bop", template));
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(
        tdir.join("meta.json"),
        format!("{{\"id\":\"t\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"implement\",\"provider_chain\":{chain_json},\"stages\":{{}},\"acceptance_criteria\":[]}}"),
    )
    .unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();
}

/// init + template + one pending card; auto-select off so the test never
/// probes real provider quotas.
fn setup_routing_case(cards: &Path, chain_json: &str, providers_json: &str, id: &str) {
    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());
    fs::write(cards.join("providers.json"), providers_json).unwrap();
    fs::create_dir_all(cards.join(".bop")).unwrap();
    fs::write(
        cards.join(".bop").join("config.json"),
        r#"{"dispatch":{"auto_select_provider":false}}"#,
    )
    .unwrap();
    write_template_with_chain(cards, "implement", chain_json);
    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            id,
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

fn run_dispatch_once(cards: &Path, global_adapter: &Path) {
    let status = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            global_adapter.to_str().unwrap(),
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());
}

fn done_stdout(cards: &Path, id: &str) -> String {
    let card = find_card_in(cards, "done", id);
    assert!(card.exists(), "card {id} should be in done/");
    fs::read_to_string(card.join("logs").join("stdout.log")).unwrap_or_default()
}

/// This must be run explicitly on a permitted native build host with an
/// independently built Moth agent. It is source-only until that run succeeds.
#[test]
#[ignore = "requires a permitted native host and debug MOTH_AGENT_BIN"]
fn dispatcher_moth_receipt_matches_full_rendered_prompt() {
    let moth_bin = PathBuf::from(
        std::env::var_os("MOTH_AGENT_BIN").expect("MOTH_AGENT_BIN is required for this test"),
    );
    assert!(moth_bin.is_absolute() && moth_bin.is_file());
    let adapter = PathBuf::from(env!("CARGO_BIN_EXE_bop-moth-adapter"));
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let providers = serde_json::json!({
        "providers": {
            "moth": {
                "command": adapter.to_str().unwrap(),
                "rate_limit_exit": 75
            }
        }
    })
    .to_string();
    setup_routing_case(&cards, r#"["moth"]"#, &providers, "mothbridge");

    // All echoed bytes are synthetic fixture data; no user prompt or secret
    // enters the debug-only full-echo path or the test's failure messages.
    fs::write(cards.join("system_context.md"), "Synthetic bridge fixture only.\n").unwrap();
    let pending = find_card_in(&cards, "pending", "mothbridge");
    let spec = format!("BOP-MOTH-BEGIN 雪🙂\n{}\nBOP-MOTH-END\n", "x".repeat(240));
    fs::write(pending.join("spec.md"), &spec).unwrap();
    // Limit this native fixture's dispatcher wait through BOP's checksummed
    // metadata API. On timeout BOP kills only the adapter; Moth grandchild
    // cleanup remains a separate gate.
    let mut fixture_meta = bop_core::read_meta(&pending).unwrap();
    fixture_meta.timeout_seconds = Some(30);
    bop_core::write_meta(&pending, &fixture_meta).unwrap();
    let template = fs::read_to_string(pending.join("prompt.md")).unwrap();
    let pending_meta = bop_core::read_meta(&pending).unwrap();
    let ctx = bop_core::PromptContext::from_files(&pending, &pending_meta).unwrap();
    let expected = bop_core::render_prompt(&template, &ctx);
    assert!(expected.contains(&spec));
    assert!(expected.chars().count() > 80);

    let output = Command::new(bop_bin())
        .env("MOTH_AGENT_BIN", &moth_bin)
        .env("BOP_MOTH_TEST_MOCK", "1")
        .env("MOTH_TEST_ECHO_FULL_PROMPT", &expected)
        .env("BOP_RUN_ID", "spoofed-parent-id")
        .env("AGENT_RUN_ID", "inherited-agent-conflict")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            adapter.to_str().unwrap(),
            "--once",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "dispatcher failed without a native receipt");

    let card = find_card_in(&cards, "done", "mothbridge");
    assert!(card.exists(), "Moth card must finish in done/");
    let rendered = fs::read(card.join("prompt.md")).unwrap();
    assert!(rendered == expected.as_bytes(), "BOP rendered prompt differs from fixture");
    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(card.join("meta.json")).unwrap()).unwrap();
    let runs = meta["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    let run_id = runs[0]["run_id"].as_str().unwrap();
    assert_eq!(run_id.len(), 32);
    assert!(run_id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert!(run_id != "spoofed-parent-id", "BOP must override a spoofed parent ID");
    assert!(run_id != "inherited-agent-conflict", "BOP must override AGENT_RUN_ID");
    assert_eq!(runs[0]["outcome"].as_str(), Some("success"));

    let runlog_dir = card.join("logs").join("moth");
    let jsonl = fs::read(runlog_dir.join(format!("{run_id}.jsonl"))).unwrap();
    assert_eq!(jsonl.last(), Some(&b'\n'), "runlog must end at a JSONL record boundary");
    let events: Vec<serde_json::Value> = std::str::from_utf8(&jsonl)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(events.len() >= 3);
    for (seq, event) in events.iter().enumerate() {
        assert_eq!(event["seq"].as_u64(), Some(seq as u64));
        assert!(event["run_id"].as_str() == Some(run_id), "JSONL record has a different run ID");
    }
    assert_eq!(events[0]["kind"].as_str(), Some("start"));
    assert_eq!(events.last().unwrap()["kind"].as_str(), Some("done"));
    assert_eq!(
        events.iter().filter(|event| event["kind"].as_str() == Some("done")).count(),
        1
    );
    assert!(!runlog_dir.join("inherited-agent-conflict.jsonl").exists());
    assert!(!runlog_dir.join("spoofed-parent-id.jsonl").exists());

    let echoed = format!("[mock] received: {expected}\n");
    let deltas: Vec<_> = events
        .iter()
        .filter(|event| event["kind"].as_str() == Some("text_delta"))
        .collect();
    assert_eq!(deltas.len(), 1);
    assert!(
        deltas[0]["data"]["text"].as_str() == Some(echoed.as_str()),
        "Moth text_delta did not contain the complete rendered prompt"
    );
    let stdout = fs::read_to_string(card.join("logs").join("stdout.log")).unwrap();
    assert!(stdout == echoed, "Moth stdout did not contain the complete rendered prompt");
}

/// Native-only process-tree regression. The fixture is an original Rust binary,
/// built only with the explicit timeout-process-tree-fixture feature.
#[cfg(all(unix, feature = "timeout-process-tree-fixture"))]
#[test]
#[ignore = "requires a permitted Linux/FreeBSD native host and selected tool-license closure"]
fn dispatcher_timeout_reaps_adapter_and_stops_moth_grandchild() {
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    struct KillOnDrop(Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_bop-timeout-process-tree-fixture"));
    let adapter = PathBuf::from(env!("CARGO_BIN_EXE_bop-moth-adapter"));
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let providers = serde_json::json!({
        "providers": {"moth": {"command": adapter.to_str().unwrap(), "rate_limit_exit": 75}}
    }).to_string();
    setup_routing_case(&cards, r#"["moth"]"#, &providers, "timeout-tree");
    let pending = find_card_in(&cards, "pending", "timeout-tree");
    fs::write(pending.join("spec.md"), "Synthetic timeout tree prompt.\n").unwrap();
    let mut meta = bop_core::read_meta(&pending).unwrap();
    meta.timeout_seconds = Some(6);
    bop_core::write_meta(&pending, &meta).unwrap();

    let prompt_snapshot = td.path().join("received-prompt");
    let grandchild_pid_file = td.path().join("grandchild-pid");
    let grandchild_beat = td.path().join("grandchild-beat");
    let sentinel_pid_file = td.path().join("sentinel-pid");
    let sentinel_beat = td.path().join("sentinel-beat");
    let mut sentinel = KillOnDrop(Command::new(&fixture)
        .arg("grandchild")
        .env("BOP_TIMEOUT_TREE_PID_FILE", &sentinel_pid_file)
        .env("BOP_TIMEOUT_TREE_BEAT_FILE", &sentinel_beat)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().unwrap());
    let mut dispatcher = KillOnDrop(Command::new(bop_bin())
        .env("MOTH_AGENT_BIN", &fixture)
        .env("BOP_TIMEOUT_TREE_PROMPT_FILE", &prompt_snapshot)
        .env("BOP_TIMEOUT_TREE_PID_FILE", &grandchild_pid_file)
        .env("BOP_TIMEOUT_TREE_BEAT_FILE", &grandchild_beat)
        .args([
            "--cards-dir", cards.to_str().unwrap(),
            "dispatcher", "--adapter", adapter.to_str().unwrap(), "--once",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn().unwrap());

    let started_deadline = Instant::now() + Duration::from_secs(4);
    let adapter_pid_file = loop {
        // The card moves under its glyph-prefixed filename. Rediscover it
        // after the dispatcher transition rather than freezing fallback path.
        let running = find_card_in(&cards, "running", "timeout-tree");
        let pid_file = running.join("logs").join("pid");
        if grandchild_pid_file.exists() && pid_file.exists()
            && grandchild_beat.exists() && sentinel_beat.exists()
        {
            break pid_file;
        }
        assert!(Instant::now() < started_deadline, "fixture did not start before timeout");
        assert!(dispatcher.0.try_wait().unwrap().is_none(), "dispatcher exited before fixture started");
        std::thread::sleep(Duration::from_millis(20));
    };
    let adapter_pid: i32 = fs::read_to_string(&adapter_pid_file).unwrap().parse().unwrap();
    let grandchild_pid: i32 = fs::read_to_string(&grandchild_pid_file).unwrap().parse().unwrap();
    assert!(adapter_pid > 1 && grandchild_pid > 1);
    assert_eq!(unsafe { libc::getpgid(grandchild_pid) }, adapter_pid,
        "grandchild did not inherit the adapter-owned group");
    assert_ne!(unsafe { libc::getpgid(sentinel.0.id() as i32) }, adapter_pid,
        "unrelated sentinel joined adapter group");

    let finish_deadline = Instant::now() + Duration::from_secs(12);
    let status = loop {
        if let Some(status) = dispatcher.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < finish_deadline, "dispatcher exceeded test watchdog");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "dispatcher did not record timeout cleanly");
    let failed = find_card_in(&cards, "failed", "timeout-tree");
    assert!(failed.exists(), "timed-out card must be in failed/");
    let meta = bop_core::read_meta(&failed).unwrap();
    assert_eq!(meta.exit_code, Some(124));
    assert_eq!(meta.runs.len(), 1);
    assert_eq!(meta.runs[0].outcome, "timeout");
    assert!(fs::read(&prompt_snapshot).unwrap() == fs::read(failed.join("prompt.md")).unwrap(),
        "fake Moth did not receive the full rendered prompt");

    let stopped_at = fs::metadata(&grandchild_beat).unwrap().len();
    let sentinel_at = fs::metadata(&sentinel_beat).unwrap().len();
    assert!(stopped_at > 0 && sentinel_at > 0);
    std::thread::sleep(Duration::from_millis(750));
    assert_eq!(fs::metadata(&grandchild_beat).unwrap().len(), stopped_at,
        "grandchild kept running after dispatcher timeout");
    assert!(fs::metadata(&sentinel_beat).unwrap().len() > sentinel_at,
        "unrelated sentinel stopped after adapter group kill");
    assert_eq!(unsafe { libc::kill(adapter_pid, 0) }, -1,
        "direct adapter was not reaped");
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
}

#[test]
fn dispatcher_runs_adapter_of_selected_provider_not_chain_head() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let cold = write_marker_adapter(td.path(), "cold");
    let warm = write_marker_adapter(td.path(), "warm");
    let global = write_marker_adapter(td.path(), "global");
    let providers = format!(
        r#"{{"providers":{{"cold":{{"command":"{}","rate_limit_exit":75,"cooldown_until_epoch_s":4102444800}},"warm":{{"command":"{}","rate_limit_exit":75}}}}}}"#,
        cold.display(),
        warm.display()
    );
    setup_routing_case(&cards, r#"["cold","warm"]"#, &providers, "route1");
    run_dispatch_once(&cards, &global);

    let out = done_stdout(&cards, "route1");
    assert!(
        out.contains("ran:warm"),
        "selected provider's adapter must run: {out}"
    );
    assert!(
        !out.contains("ran:cold"),
        "cooled-down chain head must not run: {out}"
    );
}

#[test]
fn dispatcher_unknown_provider_uses_global_adapter() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let global = write_marker_adapter(td.path(), "global");
    let providers = format!(
        r#"{{"providers":{{"mock":{{"command":"{}","rate_limit_exit":75}}}}}}"#,
        mock_adapter().display()
    );
    setup_routing_case(&cards, r#"["grok"]"#, &providers, "route2");
    run_dispatch_once(&cards, &global);

    let out = done_stdout(&cards, "route2");
    assert!(
        out.contains("ran:global"),
        "unknown provider → global --adapter: {out}"
    );
}

#[test]
fn dispatcher_empty_chain_uses_global_adapter() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let global = write_marker_adapter(td.path(), "global");
    let providers = format!(
        r#"{{"providers":{{"mock":{{"command":"{0}","rate_limit_exit":75}},"mock2":{{"command":"{0}","rate_limit_exit":75}}}}}}"#,
        mock_adapter().display()
    );
    setup_routing_case(&cards, "[]", &providers, "route3");
    run_dispatch_once(&cards, &global);

    let out = done_stdout(&cards, "route3");
    assert!(
        out.contains("ran:global"),
        "empty chain → global --adapter: {out}"
    );
}

/// bop#9: with BOP_TRANSLOG=1 the dispatcher and `bop retry` shadow-write
/// immutable transition facts, and the replayed state matches the directory.
#[test]
fn dispatcher_shadow_translog_matches_directory_state() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let cards_s = cards.to_str().unwrap();

    let bop = |args: &[&str]| {
        Command::new(bop_bin())
            .env("BOP_TRANSLOG", "1")
            .env("MOCK_EXIT", "0")
            .args(["--cards-dir", cards_s])
            .args(args)
            .output()
            .unwrap()
    };

    assert!(bop(&["init"]).status.success());
    write_providers(&cards);
    write_template(&cards, "implement");
    assert!(bop(&["new", "implement", "tl1"]).status.success());
    let adapter = mock_adapter();
    assert!(bop(&[
        "dispatcher",
        "--adapter",
        adapter.to_str().unwrap(),
        "--once"
    ])
    .status
    .success());
    assert!(find_card_in(&cards, "done", "tl1").exists());

    let out = bop(&["translog", "verify", "tl1"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let rep: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(rep["schema"], "bop.translog.verify.v1");
    assert_eq!(rep["log_state"], "done");
    assert_eq!(rep["consistent"], true);

    // Operator retry keeps request identity and appends a Retry fact.
    assert!(bop(&["retry", "tl1"]).status.success());
    let out = bop(&["translog", "show", "tl1"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["view"]["state"], "pending");
    let ops: Vec<&str> = v["view"]["lineage"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["op"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["create", "claim", "complete", "retry"]);
    assert_eq!(v["torn_tail"], serde_json::Value::Null);

    let out = bop(&["translog", "verify", "--all"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
