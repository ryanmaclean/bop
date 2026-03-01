use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn run_jc(cards: &Path, args: &[&str]) -> Output {
    Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn memory_cli_set_get_list_delete() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let set = run_jc(
        &cards,
        &[
            "memory",
            "set",
            "implement",
            "project_style",
            "prefer small diffs",
            "--ttl-seconds",
            "3600",
        ],
    );
    assert!(set.status.success());

    let get = run_jc(&cards, &["memory", "get", "implement", "project_style"]);
    assert!(get.status.success());
    let value = String::from_utf8_lossy(&get.stdout);
    assert_eq!(value.trim(), "prefer small diffs");

    let list = run_jc(&cards, &["memory", "list", "implement"]);
    assert!(list.status.success());
    let list_out = String::from_utf8_lossy(&list.stdout);
    assert!(list_out.contains("project_style"));
    assert!(list_out.contains("prefer small diffs"));

    let delete = run_jc(&cards, &["memory", "delete", "implement", "project_style"]);
    assert!(delete.status.success());

    let list_after = run_jc(&cards, &["memory", "list", "implement"]);
    assert!(list_after.status.success());
    assert_eq!(
        String::from_utf8_lossy(&list_after.stdout).trim(),
        "(empty)"
    );
}

#[test]
fn memory_list_prunes_expired_entries() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    fs::create_dir_all(cards.join("memory")).unwrap();
    fs::write(
        cards.join("memory").join("implement.json"),
        r#"{
  "entries": {
    "expired": {
      "value": "old",
      "updated_at": "2025-01-01T00:00:00Z",
      "expires_at": "2000-01-01T00:00:00Z"
    },
    "alive": {
      "value": "fresh",
      "updated_at": "2025-01-01T00:00:00Z",
      "expires_at": "2999-01-01T00:00:00Z"
    }
  }
}
"#,
    )
    .unwrap();

    let list = run_jc(&cards, &["memory", "list", "implement"]);
    assert!(list.status.success());
    let list_out = String::from_utf8_lossy(&list.stdout);
    assert!(!list_out.contains("expired"));
    assert!(list_out.contains("alive"));

    let updated = fs::read_to_string(cards.join("memory").join("implement.json")).unwrap();
    assert!(!updated.contains("\"expired\""));
    assert!(updated.contains("\"alive\""));
}

#[test]
fn dispatcher_injects_memory_and_merges_memory_output() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let set = run_jc(
        &cards,
        &[
            "memory",
            "set",
            "implement",
            "project_style",
            "use rustfmt before commit",
            "--ttl-seconds",
            "3600",
        ],
    );
    assert!(set.status.success());

    let new = run_jc(&cards, &["new", "implement", "job-memory"]);
    assert!(new.status.success());

    let adapter = td.path().join("memory_adapter.zsh");
    fs::write(
        &adapter,
        r#"#!/usr/bin/env zsh
set -euo pipefail
workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"; memory_out="${5:-${JOBCARD_MEMORY_OUT:-}}"
cat "$prompt_file" >> "$stdout_log"
echo '{"set":{"learned_fact":"always run cargo test first"}}' > "$memory_out"
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&adapter).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&adapter, perms).unwrap();

    let dispatch = run_jc(
        &cards,
        &[
            "dispatcher",
            "--adapter",
            adapter.to_str().unwrap(),
            "--once",
        ],
    );
    assert!(dispatch.status.success());

    let card_dir = cards.join("done").join("job-memory.jobcard");
    assert!(card_dir.exists());

    let stdout_log = fs::read_to_string(card_dir.join("logs").join("stdout.log")).unwrap();
    assert!(stdout_log.contains("project_style"));
    assert!(stdout_log.contains("use rustfmt before commit"));

    let memory_store = fs::read_to_string(cards.join("memory").join("implement.json")).unwrap();
    let store_json: serde_json::Value = serde_json::from_str(&memory_store).unwrap();
    assert_eq!(
        store_json["entries"]["learned_fact"]["value"].as_str(),
        Some("always run cargo test first")
    );
}

#[test]
fn dispatcher_merges_flat_memory_output_format() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let new = run_jc(&cards, &["new", "implement", "job-memory-flat"]);
    assert!(new.status.success());

    let adapter = td.path().join("memory_adapter_flat.zsh");
    fs::write(
        &adapter,
        r#"#!/usr/bin/env zsh
set -euo pipefail
workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"; memory_out="${5:-${JOBCARD_MEMORY_OUT:-}}"
cat "$prompt_file" >> "$stdout_log"
echo '{"flat_fact":"prefer deterministic tests"}' > "$memory_out"
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&adapter).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&adapter, perms).unwrap();

    let dispatch = run_jc(
        &cards,
        &[
            "dispatcher",
            "--adapter",
            adapter.to_str().unwrap(),
            "--once",
        ],
    );
    assert!(dispatch.status.success());

    let memory_store = fs::read_to_string(cards.join("memory").join("implement.json")).unwrap();
    let store_json: serde_json::Value = serde_json::from_str(&memory_store).unwrap();
    assert_eq!(
        store_json["entries"]["flat_fact"]["value"].as_str(),
        Some("prefer deterministic tests")
    );
}
