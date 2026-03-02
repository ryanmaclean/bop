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

fn mock_adapter() -> PathBuf {
    repo_root().join("adapters").join("mock.zsh")
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
    let tdir = cards
        .join("templates")
        .join(format!("{}.jobcard", template));
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(tdir.join("meta.json"), "{\"id\":\"t\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"implement\",\"provider_chain\":[\"mock\",\"mock2\"],\"stages\":{},\"acceptance_criteria\":[]}").unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();
}

fn write_running_card_with_stale_lease(cards: &Path, id: &str) {
    let card = cards.join("running").join(format!("{id}.jobcard"));
    fs::create_dir_all(card.join("logs")).unwrap();
    fs::create_dir_all(card.join("output")).unwrap();
    fs::write(
        card.join("meta.json"),
        format!(
            r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{{"implement":{{"status":"running","agent":"adapters/mock.zsh","provider":"mock"}}}},"acceptance_criteria":[],"retry_count":0}}"#
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

#[test]
fn dispatcher_moves_success_to_done() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);

    write_template(&cards, "implement");

    let status = Command::new(jc_bin())
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

    let status = Command::new(jc_bin())
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

    assert!(cards.join("done").join("job1.jobcard").exists());
}

#[test]
fn dispatcher_rate_limit_requeues_to_pending() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);

    write_template(&cards, "implement");

    let status = Command::new(jc_bin())
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

    let status = Command::new(jc_bin())
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

    assert!(cards.join("pending").join("job2.jobcard").exists());
}

#[test]
fn dispatcher_rate_limit_sets_cooldown_and_rotates_chain() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");

    let status = Command::new(jc_bin())
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

    let status = Command::new(jc_bin())
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

    let meta_path = cards.join("pending").join("job3.jobcard").join("meta.json");
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
    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");

    let status = Command::new(jc_bin())
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

    // Run the dispatcher from repo_root() so that "adapters/mock.zsh" resolves.
    let status = Command::new(jc_bin())
        .env("MOCK_EXIT", "0")
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            "adapters/mock.zsh", // relative path
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
    let card_dir = cards.join("done").join("rel-job1.jobcard");
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

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);

    let tdir = cards.join("templates").join("qa.jobcard");
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(
        tdir.join("meta.json"),
        "{\"id\":\"t\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"qa\",\"provider_chain\":[\"mock\",\"mock2\"],\"stages\":{\"implement\":{\"status\":\"done\",\"provider\":\"mock\"}},\"acceptance_criteria\":[]}",
    )
    .unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "new", "qa", "job4"])
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(jc_bin())
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

    let meta_path = cards.join("done").join("job4.jobcard").join("meta.json");
    let meta = fs::read_to_string(meta_path).unwrap();
    assert!(meta.contains("\"qa\""));
    assert!(meta.contains("\"provider\": \"mock2\"") || meta.contains("\"provider\":\"mock2\""));
}

#[test]
fn dispatcher_reaps_stale_lease_without_dead_pid() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_running_card_with_stale_lease(&cards, "lease-stale");

    let status = Command::new(jc_bin())
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

    let card = cards.join("pending").join("lease-stale.jobcard");
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
fn dispatcher_fails_when_live_lock_exists() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
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

    let status = Command::new(jc_bin())
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

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_providers(&cards);
    write_template(&cards, "implement");
    let status = Command::new(jc_bin())
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

    let status = Command::new(jc_bin())
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
    assert!(cards.join("done").join("stale-lock-job.jobcard").exists());
}
