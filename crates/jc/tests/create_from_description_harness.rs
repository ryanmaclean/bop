use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

fn mock_adapter() -> PathBuf {
    repo_root().join("adapters").join("mock.zsh")
}

fn run_jc(cards: &Path, args: &[&str]) -> Output {
    Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap()
}

fn write_providers(cards: &Path) {
    let cmd = mock_adapter().to_str().unwrap().replace('\\', "\\\\");
    let json = format!(
        "{{\"default_provider\":\"mock\",\"providers\":{{\"mock\":{{\"command\":\"{}\",\"rate_limit_exit\":75}},\"mock2\":{{\"command\":\"{}\",\"rate_limit_exit\":75}}}}}}",
        cmd, cmd
    );
    fs::write(cards.join("providers.json"), json).unwrap();
}

#[test]
fn create_from_description_generates_and_writes_with_yes() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());
    write_providers(&cards);

    let generated = r##"{"suggested_template":"implement","id":"dark-mode-toggle","spec_md":"# Dark Mode Toggle\n\nAdd a user-visible dark mode toggle in settings.","acceptance_criteria":["Settings page has a dark mode toggle.","Theme preference persists across app restarts."]}"##;
    let output = Command::new(jc_bin())
        .env("MOCK_STDOUT_TEXT", generated)
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "create",
            "--from-description",
            "Add a dark mode toggle to the settings page",
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let pending: Vec<_> = fs::read_dir(cards.join("pending"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(pending.len(), 1);

    let card_dir = pending[0].path();
    let spec = fs::read_to_string(card_dir.join("spec.md")).unwrap();
    assert!(spec.contains("Dark Mode Toggle"));

    let meta: Value =
        serde_json::from_str(&fs::read_to_string(card_dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["template_namespace"], "implement");
    assert_eq!(
        meta["acceptance_criteria"][0],
        "Settings page has a dark mode toggle."
    );
    assert_eq!(
        meta["acceptance_criteria"][1],
        "Theme preference persists across app restarts."
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Generated job card draft"));
    assert!(stdout.contains("provider: mock"));
}

#[test]
fn create_from_description_requires_confirmation_before_write() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());
    write_providers(&cards);

    let generated = r##"{"suggested_template":"implement","id":"dark-mode-toggle","spec_md":"Dark mode spec","acceptance_criteria":["criterion one"]}"##;
    let mut child = Command::new(jc_bin())
        .env("MOCK_STDOUT_TEXT", generated)
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "create",
            "--from-description",
            "Add a dark mode toggle to the settings page",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"n\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let pending_count = fs::read_dir(cards.join("pending")).unwrap().count();
    assert_eq!(
        pending_count, 0,
        "card should not be written without confirmation"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Write this draft to pending/? [y/N]:"));
    assert!(stdout.contains("aborted: draft was not written"));
}
