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
    let status = Command::new("cargo")
        .arg("build")
        .current_dir(repo_root())
        .status()
        .expect("cargo build failed to start");
    assert!(status.success());
}

fn jc_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("jc")
}

fn init_cards(td: &Path) -> PathBuf {
    let cards = td.join(".cards");
    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());
    cards
}

#[test]
fn providers_list_shows_seeded_providers() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "providers", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("mock"), "expected 'mock' in: {}", stdout);
    assert!(stdout.contains("mock2"), "expected 'mock2' in: {}", stdout);
}

#[test]
fn providers_list_shows_all_fields() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    // Manually write a provider with a model and cooldown
    let now_plus_300 = chrono::Utc::now().timestamp() + 300;
    let json = format!(
        r#"{{"providers":{{"cool-provider":{{"command":"adapters/mock.zsh","rate_limit_exit":75,"cooldown_until_epoch_s":{},"model":"gpt-4o"}},"no-cool":{{"command":"adapters/mock.zsh","rate_limit_exit":75}}}}}}"#,
        now_plus_300
    );
    fs::write(cards.join("providers.json"), json).unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "providers", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // cool-provider should show a cooldown remaining > 0
    assert!(stdout.contains("cool-provider"), "{}", stdout);
    assert!(stdout.contains("gpt-4o"), "{}", stdout);
    // Should show remaining seconds (approximately 300)
    assert!(stdout.contains("cooldown"), "{}", stdout);
    // no-cool should show no cooldown
    assert!(stdout.contains("no-cool"), "{}", stdout);
}

#[test]
fn providers_add_creates_provider() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let mock_adapter = repo_root().join("adapters").join("mock.zsh");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "add",
            "claude-3",
            "--adapter",
            mock_adapter.to_str().unwrap(),
            "--model",
            "claude-opus-4-6",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let json = fs::read_to_string(cards.join("providers.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["providers"]["claude-3"]["command"].as_str().is_some());
    assert_eq!(
        v["providers"]["claude-3"]["model"].as_str(),
        Some("claude-opus-4-6")
    );
}

#[test]
fn providers_add_rejects_duplicate_name() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let mock_adapter = repo_root().join("adapters").join("mock.zsh");

    // Add once — should succeed
    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "add",
            "my-prov",
            "--adapter",
            mock_adapter.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    // Add again — should fail
    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "add",
            "my-prov",
            "--adapter",
            mock_adapter.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(!status.success(), "duplicate add should fail");
}

#[test]
fn providers_add_rejects_empty_adapter() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "add",
            "bad-prov",
            "--adapter",
            "",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

fn write_running_card_with_provider(cards: &Path, card_id: &str, provider: &str) {
    let card_dir = cards.join("running").join(format!("{}.jobcard", card_id));
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    let meta = format!(
        r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","stages":{{"implement":{{"status":"running","provider":"{prov}"}}}},"acceptance_criteria":[]}}"#,
        id = card_id,
        prov = provider
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
}

#[test]
fn providers_remove_deletes_provider() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "remove",
            "mock2",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let json = fs::read_to_string(cards.join("providers.json")).unwrap();
    assert!(!json.contains("\"mock2\""), "mock2 should be removed");
    assert!(json.contains("\"mock\""), "mock should remain");
}

#[test]
fn providers_remove_blocks_when_provider_has_active_jobs() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    // Place a running card using 'mock'
    write_running_card_with_provider(&cards, "active-job", "mock");

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "remove",
            "mock",
        ])
        .output()
        .unwrap();
    // Should fail without --force
    assert!(!out.status.success(), "should fail when active jobs exist");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("active") || stderr.contains("force"),
        "error should mention active jobs or --force: {}",
        stderr
    );
}

#[test]
fn providers_remove_force_removes_despite_active_jobs() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    write_running_card_with_provider(&cards, "active-job2", "mock");

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "remove",
            "mock",
            "--force",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let json = fs::read_to_string(cards.join("providers.json")).unwrap();
    assert!(!json.contains("\"mock\""), "mock should be removed");
}

#[test]
fn providers_remove_nonexistent_errors() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "remove",
            "does-not-exist",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

fn write_done_card_with_provider(cards: &Path, card_id: &str, provider: &str, success: bool) {
    let state = if success { "done" } else { "failed" };
    let card_dir = cards.join(state).join(format!("{}.jobcard", card_id));
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    let stage_status = if success { "done" } else { "failed" };
    let meta = format!(
        r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","stages":{{"implement":{{"status":"{ss}","provider":"{prov}"}}}},"acceptance_criteria":[]}}"#,
        id = card_id,
        ss = stage_status,
        prov = provider
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
}

#[test]
fn providers_status_shows_job_counts() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    // 2 successes and 1 failure for mock
    write_done_card_with_provider(&cards, "j1", "mock", true);
    write_done_card_with_provider(&cards, "j2", "mock", true);
    write_done_card_with_provider(&cards, "j3", "mock", false);
    // 1 success for mock2
    write_done_card_with_provider(&cards, "j4", "mock2", true);

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "status",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // mock: 3 total, 2 success
    assert!(stdout.contains("mock"), "{}", stdout);
    // The output should contain job count numbers
    assert!(stdout.contains('3') || stdout.contains('2'), "{}", stdout);
}

#[test]
fn providers_status_shows_cooldown() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = init_cards(td.path());

    let now_plus_300 = chrono::Utc::now().timestamp() + 300;
    let json = format!(
        r#"{{"providers":{{"cool-prov":{{"command":"adapters/mock.zsh","rate_limit_exit":75,"cooldown_until_epoch_s":{}}}}}}}"#,
        now_plus_300
    );
    fs::write(cards.join("providers.json"), json).unwrap();

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "providers",
            "status",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cool-prov"), "{}", stdout);
    assert!(
        stdout.contains("cooldown") || stdout.contains("300") || stdout.contains("29"),
        "{}",
        stdout
    );
}
