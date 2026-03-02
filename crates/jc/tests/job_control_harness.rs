use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_jc() {
    let cargo = env!("CARGO");
    let status = Command::new(cargo)
        .arg("build")
        .current_dir(repo_root())
        .status()
        .expect("cargo build failed to start");
    assert!(status.success());
}

fn jc_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("bop")
}

/// Create a minimal card directory in the given state with a meta.json.
fn make_card(cards: &Path, state: &str, id: &str) -> PathBuf {
    let dir = cards.join(state).join(format!("{}.jobcard", id));
    fs::create_dir_all(dir.join("logs")).unwrap();
    fs::create_dir_all(dir.join("output")).unwrap();
    fs::write(
        dir.join("meta.json"),
        format!(
            r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{{}},"acceptance_criteria":[]}}"#
        ),
    )
    .unwrap();
    fs::write(dir.join("spec.md"), format!("spec for {id}")).unwrap();
    dir
}

/// Create a card with a specific retry_count and failure_reason.
fn make_failed_card(cards: &Path, id: &str, retry_count: u32, failure_reason: &str) -> PathBuf {
    let dir = cards.join("failed").join(format!("{}.jobcard", id));
    fs::create_dir_all(dir.join("logs")).unwrap();
    fs::create_dir_all(dir.join("output")).unwrap();
    fs::write(
        dir.join("meta.json"),
        format!(
            r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{{}},"acceptance_criteria":[],"retry_count":{retry_count},"failure_reason":"{failure_reason}"}}"#
        ),
    )
    .unwrap();
    fs::write(dir.join("spec.md"), format!("spec for {id}")).unwrap();
    dir
}

/// Create a failed card whose stage metadata is stale (still marked running).
fn make_failed_card_with_running_stage(cards: &Path, id: &str) -> PathBuf {
    let dir = cards.join("failed").join(format!("{}.jobcard", id));
    fs::create_dir_all(dir.join("logs")).unwrap();
    fs::create_dir_all(dir.join("output")).unwrap();
    fs::write(
        dir.join("meta.json"),
        format!(
            r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{{"implement":{{"status":"running","agent":"adapters/mock.zsh","provider":"mock","duration_s":12,"started":"2026-03-01T00:00:00Z"}}}},"acceptance_criteria":[],"retry_count":1,"failure_reason":"killed"}}"#
        ),
    )
    .unwrap();
    fs::write(dir.join("spec.md"), format!("spec for {id}")).unwrap();
    dir
}

fn init_cards(cards: &Path) {
    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());
}

// ── meta set ──────────────────────────────────────────────────────────────────

#[test]
fn meta_set_updates_workflow_fields() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_card(&cards, "pending", "m1");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "meta",
            "set",
            "m1",
            "--workflow-mode",
            "ideation",
            "--step-index",
            "3",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let meta_raw =
        fs::read_to_string(cards.join("pending").join("m1.jobcard").join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(
        meta.get("workflow_mode").and_then(|v| v.as_str()),
        Some("ideation")
    );
    assert_eq!(meta.get("step_index").and_then(|v| v.as_u64()), Some(3));
}

#[test]
fn meta_set_clear_workflow_mode_also_clears_step_index() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_card(&cards, "pending", "m2");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "meta",
            "set",
            "m2",
            "--workflow-mode",
            "roadmap",
            "--step-index",
            "2",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "meta",
            "set",
            "m2",
            "--clear-workflow-mode",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let meta_raw =
        fs::read_to_string(cards.join("pending").join("m2.jobcard").join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert!(
        meta.get("workflow_mode").is_none()
            || meta
                .get("workflow_mode")
                .and_then(|v| v.as_null())
                .is_some()
    );
    assert!(
        meta.get("step_index").is_none()
            || meta.get("step_index").and_then(|v| v.as_null()).is_some()
    );
}

#[test]
fn meta_set_rejects_step_index_without_workflow_mode() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_card(&cards, "pending", "m3");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "meta",
            "set",
            "m3",
            "--step-index",
            "2",
        ])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "step_index without workflow_mode should fail validation"
    );
}

// ── retry ─────────────────────────────────────────────────────────────────────

#[test]
fn retry_moves_failed_card_to_pending() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_failed_card(&cards, "r1", 0, "some_error");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "retry", "r1"])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(
        cards.join("pending").join("r1.jobcard").exists(),
        "card should be in pending/"
    );
    assert!(
        !cards.join("failed").join("r1.jobcard").exists(),
        "card should not remain in failed/"
    );
}

#[test]
fn retry_increments_retry_count_and_clears_failure_reason() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_failed_card(&cards, "r2", 2, "transient_error");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "retry", "r2"])
        .status()
        .unwrap();
    assert!(status.success());

    let meta_raw =
        fs::read_to_string(cards.join("pending").join("r2.jobcard").join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();

    assert_eq!(
        meta.get("retry_count").and_then(|v| v.as_u64()),
        Some(3),
        "retry_count should be incremented"
    );
    assert!(
        meta.get("failure_reason").is_none()
            || meta
                .get("failure_reason")
                .and_then(|v| v.as_null())
                .is_some(),
        "failure_reason should be cleared"
    );
}

#[test]
fn retry_normalizes_stale_running_stage_to_pending() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_failed_card_with_running_stage(&cards, "r2b");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "retry", "r2b"])
        .status()
        .unwrap();
    assert!(status.success());

    let meta_raw =
        fs::read_to_string(cards.join("pending").join("r2b.jobcard").join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    let stage = meta.get("stages").and_then(|s| s.get("implement")).unwrap();

    assert_eq!(
        stage.get("status").and_then(|v| v.as_str()),
        Some("pending"),
        "retry should reset stale stage status to pending"
    );
    assert!(
        stage.get("agent").is_none() || stage.get("agent").and_then(|v| v.as_null()).is_some(),
        "retry should clear stale agent"
    );
    assert!(
        stage.get("provider").is_none()
            || stage.get("provider").and_then(|v| v.as_null()).is_some(),
        "retry should clear stale provider"
    );
    assert!(
        stage.get("duration_s").is_none()
            || stage.get("duration_s").and_then(|v| v.as_null()).is_some(),
        "retry should clear stale duration_s"
    );
}

#[test]
fn retry_fails_when_card_not_found() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "retry",
            "nosuchcard",
        ])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "retry should exit non-zero for missing card"
    );
}

#[test]
fn retry_fails_when_card_is_pending() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_card(&cards, "pending", "r3");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "retry", "r3"])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "retry of already-pending card should fail"
    );
}

#[test]
fn retry_fails_when_card_is_running() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_card(&cards, "running", "r4");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "retry", "r4"])
        .status()
        .unwrap();
    assert!(!status.success(), "retry of running card should fail");
}

// ── kill ──────────────────────────────────────────────────────────────────────

#[test]
fn kill_fails_when_card_not_running() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    make_card(&cards, "failed", "k1");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "kill", "k1"])
        .status()
        .unwrap();
    assert!(!status.success(), "kill of non-running card should fail");
}

#[test]
fn kill_fails_when_card_not_found() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "kill", "nosuchcard"])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "kill should exit non-zero for missing card"
    );
}

/// Spawn a real sleep process, write its PID to the card, then kill it.
#[test]
fn kill_sends_sigterm_and_moves_to_failed() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);

    // Create a running card backed by a real process we control
    let card_dir = make_card(&cards, "running", "k2");

    // Spawn a long-lived sleep so we have a real PID to kill
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("failed to spawn sleep");
    let pid = child.id();

    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::write(card_dir.join("logs").join("pid"), pid.to_string()).unwrap();

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "kill", "k2"])
        .status()
        .unwrap();
    assert!(status.success(), "kill should succeed");

    // Reap child after kill
    let _ = child.wait();

    assert!(
        cards.join("failed").join("k2.jobcard").exists(),
        "card should be in failed/"
    );
    assert!(
        !cards.join("running").join("k2.jobcard").exists(),
        "card should not remain in running/"
    );

    let meta_raw =
        fs::read_to_string(cards.join("failed").join("k2.jobcard").join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(
        meta.get("failure_reason").and_then(|v| v.as_str()),
        Some("killed"),
        "failure_reason should be 'killed'"
    );
}

#[test]
fn kill_handles_stale_pid_and_moves_to_failed() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);

    let card_dir = make_card(&cards, "running", "k3");

    // Create a stale PID by spawning then immediately terminating a child.
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("failed to spawn sleep");
    let pid = child.id();
    let _ = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    let _ = child.wait();

    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::write(card_dir.join("logs").join("pid"), pid.to_string()).unwrap();

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "kill", "k3"])
        .status()
        .unwrap();
    assert!(status.success(), "kill should succeed for stale pid");

    assert!(
        cards.join("failed").join("k3.jobcard").exists(),
        "card should be in failed/"
    );
    assert!(
        !cards.join("running").join("k3.jobcard").exists(),
        "card should not remain in running/"
    );

    let meta_raw =
        fs::read_to_string(cards.join("failed").join("k3.jobcard").join("meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
    assert_eq!(
        meta.get("failure_reason").and_then(|v| v.as_str()),
        Some("killed"),
        "failure_reason should be 'killed'"
    );
}

// ── logs ──────────────────────────────────────────────────────────────────────

#[test]
fn logs_prints_existing_stdout_and_stderr() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    let card_dir = make_card(&cards, "done", "l1");

    fs::write(card_dir.join("logs").join("stdout.log"), "hello stdout\n").unwrap();
    fs::write(card_dir.join("logs").join("stderr.log"), "hello stderr\n").unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "logs", "l1"])
        .output()
        .unwrap();

    assert!(out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("hello stdout"), "stdout content missing");
    assert!(combined.contains("hello stderr"), "stderr content missing");
}

#[test]
fn logs_fails_when_card_not_found() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "logs", "nosuchcard"])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "logs should exit non-zero for missing card"
    );
}

// ── inspect ───────────────────────────────────────────────────────────────────

#[test]
fn inspect_shows_meta_and_spec() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);
    let card_dir = make_card(&cards, "done", "i1");

    fs::write(card_dir.join("logs").join("stdout.log"), "output line 1\n").unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "inspect", "i1"])
        .output()
        .unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(stdout.contains("meta"), "should show meta section");
    assert!(
        stdout.contains("spec for i1"),
        "should show spec.md content"
    );
    assert!(stdout.contains("output line 1"), "should show log tail");
}

#[test]
fn inspect_fails_when_card_not_found() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    init_cards(&cards);

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "inspect",
            "nosuchcard",
        ])
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "inspect should exit non-zero for missing card"
    );
}
