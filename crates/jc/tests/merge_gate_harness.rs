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

    // 1. Create a temp dir that IS the git root (card dir must be inside the repo).
    let td = tempfile::tempdir().unwrap();
    let git_root = td.path();

    // 2. Init git repo in git_root with an initial commit so HEAD exists.
    run(Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(git_root));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ])
        .current_dir(git_root));

    // 3. Create conflict-inducing file on main and commit it.
    fs::write(git_root.join("shared.txt"), "main version\n").unwrap();
    run(Command::new("git")
        .args(["add", "shared.txt"])
        .current_dir(git_root));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "add shared",
        ])
        .current_dir(git_root));

    // 4. Create cards directory structure INSIDE the git repo so find_git_root works.
    let cards = git_root.join(".cards");
    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let id = "mg3";
    let done_dir = cards.join("done");
    let card_dir = done_dir.join(format!("{}.jobcard", id));
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    fs::write(card_dir.join("prompt.md"), "").unwrap();
    fs::write(card_dir.join("spec.md"), "").unwrap();

    // 5. Create a linked git worktree on branch jobs/mg3 at card_dir/worktree.
    let wt_path = card_dir.join("worktree");
    run(Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            "jobs/mg3",
            wt_path.to_str().unwrap(),
        ])
        .current_dir(git_root));

    // 6. Make a conflicting change in the worktree branch and commit it.
    fs::write(wt_path.join("shared.txt"), "branch version\n").unwrap();
    run(Command::new("git")
        .args(["add", "shared.txt"])
        .current_dir(&wt_path));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "branch change",
        ])
        .current_dir(&wt_path));

    // 7. Make a conflicting change on main AFTER the worktree was created from the same base.
    //    git_root IS on main, so edit shared.txt there directly.
    fs::write(git_root.join("shared.txt"), "different main version\n").unwrap();
    run(Command::new("git")
        .args(["add", "shared.txt"])
        .current_dir(git_root));
    run(Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "conflicting main",
        ])
        .current_dir(git_root));

    // 8. Write meta with acceptance criteria that pass so we reach the merge step.
    let meta = format!(
        "{{\"id\":\"{id}\",\"created\":\"2026-03-01T00:00:00Z\",\"stage\":\"qa\",\"provider_chain\":[],\"stages\":{{}},\"acceptance_criteria\":[\"exit 0\"],\"worktree_branch\":\"jobs/mg3\"}}",
        id = id
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();

    // 9. Run merge gate.
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

    // 10. Verify outcome: card is in failed/, conflicts.diff exists, failure_reason is set.
    let failed = cards.join("failed").join(format!("{}.jobcard", id));
    assert!(failed.exists(), "card should be in failed/");
    assert!(
        failed.join("output").join("conflicts.diff").exists(),
        "conflicts.diff should exist"
    );

    let meta_str = fs::read_to_string(failed.join("meta.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&meta_str).unwrap();
    assert_eq!(
        v.get("failure_reason").and_then(|x| x.as_str()),
        Some("merge_conflict")
    );
}
