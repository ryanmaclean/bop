use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(cmd: &mut Command) {
    let status = cmd.status().expect("failed to start command");
    assert!(status.success());
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

fn write_card(cards_dir: &PathBuf, id: &str, acceptance: &[&str]) {
    let card_dir = cards_dir.join("done").join(format!("{}.jobcard", id));
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();

    let criteria = acceptance
        .iter()
        .map(|c| format!("\"{}\"", c.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(",");

    let meta = format!(
        "{{\"id\":\"{}\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"qa\",\"provider_chain\":[],\"stages\":{{}},\"acceptance_criteria\":[{}]}}",
        id, criteria
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
    fs::write(card_dir.join("prompt.md"), "").unwrap();
    fs::write(card_dir.join("spec.md"), "").unwrap();
}

#[test]
fn merge_gate_moves_passing_card_to_merged() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_card(&cards, "mg1", &["exit 0"]);

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "merge-gate",
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    assert!(cards.join("merged").join("mg1.jobcard").exists());
}

#[test]
fn merge_gate_moves_failing_card_to_failed_and_writes_report() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    write_card(&cards, "mg2", &["exit 1"]);

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "merge-gate",
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let card_dir = cards.join("failed").join("mg2.jobcard");
    assert!(card_dir.exists());
    assert!(card_dir.join("output").join("qa_report.md").exists());
}

#[test]
fn merge_gate_merge_conflict_moves_card_to_failed_and_writes_conflicts() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let id = "mg3";
    let card_dir = cards.join("done").join(format!("{}.jobcard", id));
    let worktree = card_dir.join("worktree");
    fs::create_dir_all(worktree.join("logs")).unwrap();
    fs::create_dir_all(worktree.join("output")).unwrap();
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    fs::write(card_dir.join("prompt.md"), "").unwrap();
    fs::write(card_dir.join("spec.md"), "").unwrap();

    run(Command::new("git")
        .arg("init")
        .arg("-b")
        .arg("main")
        .current_dir(&worktree));

    fs::write(worktree.join("file.txt"), "line1\n").unwrap();
    run(Command::new("git")
        .arg("add")
        .arg("file.txt")
        .current_dir(&worktree));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(&worktree));

    run(Command::new("git")
        .arg("checkout")
        .arg("-b")
        .arg("job/mg3")
        .current_dir(&worktree));
    fs::write(worktree.join("file.txt"), "branch\n").unwrap();
    run(Command::new("git")
        .arg("add")
        .arg("file.txt")
        .current_dir(&worktree));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "branch",
        ])
        .current_dir(&worktree));

    run(Command::new("git")
        .arg("checkout")
        .arg("main")
        .current_dir(&worktree));
    fs::write(worktree.join("file.txt"), "main\n").unwrap();
    run(Command::new("git")
        .arg("add")
        .arg("file.txt")
        .current_dir(&worktree));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "main",
        ])
        .current_dir(&worktree));

    let meta = format!(
        "{{\"id\":\"{}\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"qa\",\"provider_chain\":[],\"stages\":{{}},\"acceptance_criteria\":[\"exit 0\"],\"worktree_branch\":\"job/mg3\"}}",
        id
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();

    let status = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "merge-gate",
            "--once",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let failed = cards.join("failed").join(format!("{}.jobcard", id));
    assert!(failed.exists());
    assert!(failed.join("output").join("conflicts.diff").exists());

    let meta = fs::read_to_string(failed.join("meta.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(
        v.get("failure_reason").and_then(|x| x.as_str()),
        Some("merge_conflict")
    );
}
