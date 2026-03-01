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

/// Write a minimal meta.json into a card directory.
fn write_meta(card_dir: &Path, id: &str, branch: &str) {
    let meta = format!(
        r#"{{"id":"{id}","created":"2026-03-01T00:00:00Z","stage":"implement","provider_chain":[],"stages":{{}},"acceptance_criteria":[],"worktree_branch":"{branch}"}}"#,
        id = id,
        branch = branch
    );
    fs::write(card_dir.join("meta.json"), meta).unwrap();
}

#[test]
fn worktree_list_shows_cards_with_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // jc init
    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // Card in running/ with a worktree/ dir
    let running_card = cards.join("running").join("run-job.jobcard");
    fs::create_dir_all(running_card.join("worktree")).unwrap();
    write_meta(&running_card, "run-job", "job/run-job");

    // Card in done/ with a worktree/ dir
    let done_card = cards.join("done").join("done-job.jobcard");
    fs::create_dir_all(done_card.join("worktree")).unwrap();
    write_meta(&done_card, "done-job", "job/done-job");

    // Card in pending/ WITHOUT a worktree/ dir (should not appear)
    let pending_card = cards.join("pending").join("no-wt.jobcard");
    fs::create_dir_all(&pending_card).unwrap();
    write_meta(&pending_card, "no-wt", "job/no-wt");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "list"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "jc worktree list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("run-job"),
        "should show run-job:\n{}",
        stdout
    );
    assert!(
        stdout.contains("done-job"),
        "should show done-job:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("no-wt"),
        "should NOT show no-wt (no worktree/):\n{}",
        stdout
    );
    assert!(
        stdout.contains("job/run-job"),
        "should show branch:\n{}",
        stdout
    );
    assert!(stdout.contains("running"), "should show state:\n{}", stdout);
    assert!(
        stdout.contains("done"),
        "should show done state:\n{}",
        stdout
    );
}

#[test]
fn worktree_list_flags_orphaned_git_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init the temp dir as a git repo (needed for git worktree add)
    let s = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(td.path())
        .status()
        .unwrap();
    assert!(s.success());
    // Initial commit required for git worktree add
    fs::write(td.path().join("README"), "init").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(td.path())
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.email=a@b",
            "-c",
            "user.name=a",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(td.path())
        .status()
        .unwrap();

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // Create an orphaned git worktree: a branch with NO corresponding card
    let orphan_path = td.path().join("orphan-wt");
    Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            "job/orphan",
            orphan_path.to_str().unwrap(),
        ])
        .current_dir(td.path())
        .status()
        .unwrap();

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    // The orphan-wt worktree must appear and be flagged
    assert!(
        stdout.contains("orphan") || stdout.contains(orphan_path.to_str().unwrap()),
        "orphaned worktree should appear in list:\n{}",
        stdout
    );
    assert!(
        stdout.contains("orphaned"),
        "orphaned worktrees should be flagged:\n{}",
        stdout
    );
}

#[test]
fn worktree_create_makes_git_worktree_for_pending_card() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init git repo with initial commit
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(td.path())
            .status()
            .unwrap();
    };
    git(&["init", "-b", "main"]);
    fs::write(td.path().join("README"), "init").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=a@b",
        "-c",
        "user.name=a",
        "commit",
        "-m",
        "init",
    ]);

    // jc init
    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // Create a pending card manually
    let card_dir = cards.join("pending").join("create-job.jobcard");
    fs::create_dir_all(card_dir.join("logs")).unwrap();
    fs::create_dir_all(card_dir.join("output")).unwrap();
    write_meta(&card_dir, "create-job", "job/create-job");
    fs::write(card_dir.join("spec.md"), "").unwrap();
    fs::write(card_dir.join("prompt.md"), "").unwrap();

    // Run worktree create
    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "worktree",
            "create",
            "create-job",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "worktree create failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // worktree/ directory must exist inside the card
    let wt = card_dir.join("worktree");
    assert!(wt.exists(), "worktree/ dir should exist after create");

    // git should know about the new worktree
    let git_out = Command::new("git")
        .args(["worktree", "list"])
        .current_dir(td.path())
        .output()
        .unwrap();
    let git_list = String::from_utf8_lossy(&git_out.stdout);
    assert!(
        git_list.contains("create-job") || git_list.contains(wt.to_str().unwrap()),
        "git worktree list should show the new worktree:\n{}",
        git_list
    );
}

#[test]
fn worktree_create_fails_for_done_card() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init git repo
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(td.path())
            .status()
            .unwrap();
    };
    git(&["init", "-b", "main"]);
    fs::write(td.path().join("README"), "init").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=a@b",
        "-c",
        "user.name=a",
        "commit",
        "-m",
        "init",
    ]);

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // Card in done/ — create should refuse
    let card_dir = cards.join("done").join("done-job.jobcard");
    fs::create_dir_all(&card_dir).unwrap();
    write_meta(&card_dir, "done-job", "job/done-job");

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "worktree",
            "create",
            "done-job",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "worktree create should fail for done/ card"
    );
}

#[test]
fn worktree_clean_dry_run_does_not_remove() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // Card in done/ with a worktree/ dir
    let done_card = cards.join("done").join("done-clean.jobcard");
    let wt = done_card.join("worktree");
    fs::create_dir_all(&wt).unwrap();
    write_meta(&done_card, "done-clean", "job/done-clean");

    let out = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "worktree",
            "clean",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dry-run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("done-clean") || stdout.contains("would remove"),
        "dry-run should preview removal:\n{}",
        stdout
    );

    // Worktree must still exist after dry-run
    assert!(wt.exists(), "dry-run must NOT delete the worktree");
}

#[test]
fn worktree_clean_removes_done_and_merged_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // done/ card with worktree
    let done_card = cards.join("done").join("d1.jobcard");
    let done_wt = done_card.join("worktree");
    fs::create_dir_all(&done_wt).unwrap();
    write_meta(&done_card, "d1", "job/d1");

    // merged/ card with worktree
    let merged_card = cards.join("merged").join("m1.jobcard");
    let merged_wt = merged_card.join("worktree");
    fs::create_dir_all(&merged_wt).unwrap();
    write_meta(&merged_card, "m1", "job/m1");

    // pending/ card with worktree — must NOT be removed
    let pending_card = cards.join("pending").join("p1.jobcard");
    let pending_wt = pending_card.join("worktree");
    fs::create_dir_all(&pending_wt).unwrap();
    write_meta(&pending_card, "p1", "job/p1");

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "clean"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clean failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(!done_wt.exists(), "done worktree should be removed");
    assert!(!merged_wt.exists(), "merged worktree should be removed");
    assert!(pending_wt.exists(), "pending worktree must NOT be removed");
}

#[test]
fn worktree_clean_removes_orphaned_git_worktrees() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Init git repo
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(td.path())
            .status()
            .unwrap();
    };
    git(&["init", "-b", "main"]);
    fs::write(td.path().join("README"), "init").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=a@b",
        "-c",
        "user.name=a",
        "commit",
        "-m",
        "init",
    ]);

    let s = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(s.success());

    // Orphaned git worktree: no corresponding job card
    let orphan_path = td.path().join("orphan-wt");
    Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            "job/orphan",
            orphan_path.to_str().unwrap(),
        ])
        .current_dir(td.path())
        .status()
        .unwrap();

    assert!(
        orphan_path.exists(),
        "orphan worktree should exist before clean"
    );

    let out = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "worktree", "clean"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "clean failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        !orphan_path.exists(),
        "orphaned git worktree should be removed"
    );
}
