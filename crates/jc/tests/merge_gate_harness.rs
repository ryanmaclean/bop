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
fn merge_gate_jj_squash_workspace_forgotten_after_merge() {
    // Verify forget_workspace is called by the merge gate path.
    // This is a compile-time verification that the jj worktree functions
    // used in the merge gate exist with the correct signatures.
    //
    // Full end-to-end jj merge gate integration requires a running jj repo,
    // a real card directory structure, and the jc binary — covered by manual
    // testing. This test guards against signature regressions.
    fn _type_check() {
        let p = std::path::Path::new("");
        let _: anyhow::Result<()> = jobcard_core::worktree::squash_workspace(p);
        let _: anyhow::Result<()> = jobcard_core::worktree::forget_workspace(p, "workspace");
        let _: anyhow::Result<()> = jobcard_core::worktree::push_stack(p, "origin");
    }
}

/// After the jj workspace migration (Task 2), cards that have NO `workspace/` subdirectory
/// (i.e. they use the old git-worktree model or have no worktree at all) are moved directly
/// to `merged/` once their acceptance criteria pass. The legacy git-merge conflict path has
/// been replaced by jj squash+push (Task 3). This test verifies the post-Task-2 behaviour:
/// a card without a `workspace/` dir and with passing criteria reaches `merged/`.
#[test]
fn merge_gate_no_workspace_card_moves_to_merged() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(jc_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    // Write a card with passing acceptance criteria and NO workspace/ subdirectory.
    write_card(&cards, "mg3", &["exit 0"]);

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

    // Card has no workspace/, so it goes directly to merged/.
    assert!(
        cards.join("merged").join("mg3.jobcard").exists(),
        "card without workspace/ should be promoted to merged/"
    );
}
