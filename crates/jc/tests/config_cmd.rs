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
        .expect("cargo build failed");
    assert!(status.success());
}

fn jc_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("jc")
}

fn config_get(config_path: &Path, key: &str) -> std::process::Output {
    Command::new(jc_bin())
        .args(["config", "get", key])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap()
}

fn config_set(config_path: &Path, key: &str, value: &str) -> std::process::Output {
    Command::new(jc_bin())
        .args(["config", "set", key, value])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap()
}

#[test]
fn config_set_and_get_max_concurrent() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_set(&config_path, "max_concurrent", "4");
    assert!(
        out.status.success(),
        "set failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = config_get(&config_path, "max_concurrent");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("4"),
        "expected '4' in output, got: {}",
        stdout
    );
}

#[test]
fn config_set_and_get_default_template() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_set(&config_path, "default_template", "qa");
    assert!(out.status.success());

    let out = config_get(&config_path, "default_template");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("qa"));
}

#[test]
fn config_set_and_get_provider_chain() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_set(&config_path, "default_provider_chain", "claude,codex");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = config_get(&config_path, "default_provider_chain");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("claude"), "got: {}", stdout);
    assert!(stdout.contains("codex"), "got: {}", stdout);
}

#[test]
fn config_get_missing_key_errors() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");

    let out = config_get(&config_path, "nonexistent_key");
    assert!(
        !out.status.success(),
        "expected non-zero exit for unknown key"
    );
}

#[test]
fn config_get_unset_value_prints_empty_or_unset() {
    build_jc();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.yaml");
    // Create empty config
    fs::write(&config_path, "").unwrap();

    let out = config_get(&config_path, "max_concurrent");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should print "(unset)" or empty line
    assert!(
        stdout.trim().is_empty() || stdout.contains("unset"),
        "got: {}",
        stdout
    );
}

#[test]
fn init_creates_global_config_with_defaults() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    // Point JOBCARD_CONFIG to a temp path so we don't touch real ~/.jobcard
    let config_path = td.path().join("config.yaml");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        config_path.exists(),
        "config file should be created by init"
    );
    let content = fs::read_to_string(&config_path).unwrap();
    // Should have sensible defaults
    assert!(
        content.contains("max_concurrent") || content.contains("default_template"),
        "config should contain default keys, got: {}",
        content
    );
}

#[test]
fn dispatcher_uses_config_max_concurrent() {
    build_jc();
    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");
    let config_path = td.path().join("config.yaml");

    // Write config with max_concurrent = 2
    fs::write(&config_path, "max_concurrent: 2\n").unwrap();

    // Init
    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success());

    // Write a template and create a card
    let tdir = cards.join("templates").join("implement.jobcard");
    fs::create_dir_all(tdir.join("logs")).unwrap();
    fs::create_dir_all(tdir.join("output")).unwrap();
    fs::write(
        tdir.join("meta.json"),
        r#"{"id":"t","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":["mock"],"stages":{},"acceptance_criteria":[]}"#,
    )
    .unwrap();
    fs::write(tdir.join("spec.md"), "").unwrap();
    fs::write(tdir.join("prompt.md"), "{{spec}}\n").unwrap();

    let mock_adapter = repo_root().join("adapters").join("mock.zsh");
    let mock_cmd = mock_adapter.to_str().unwrap();
    let providers_json = format!(
        r#"{{"providers":{{"mock":{{"command":"{}","rate_limit_exit":75}}}}}}"#,
        mock_cmd
    );
    fs::write(cards.join("providers.json"), providers_json).unwrap();

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "new",
            "implement",
            "cfg-job1",
        ])
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success());

    // Run dispatcher without --max-workers flag; it should pick up max_concurrent=2 from config
    let out = Command::new(jc_bin())
        .env("MOCK_EXIT", "0")
        .env("JOBCARD_CONFIG", config_path.to_str().unwrap())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "dispatcher",
            "--adapter",
            mock_cmd,
            "--once",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        cards.join("done").join("cfg-job1.jobcard").exists(),
        "card should be done"
    );
}
