use std::fs;
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
fn dashboard_falls_back_to_status_when_non_interactive() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let job1 = run_jc(&cards, &["new", "implement", "dash-job-1"]);
    assert!(job1.status.success());

    let job2 = run_jc(&cards, &["new", "implement", "dash-job-2"]);
    assert!(job2.status.success());

    fs::rename(
        cards.join("pending").join("dash-job-2.jobcard"),
        cards.join("running").join("dash-job-2.jobcard"),
    )
    .unwrap();

    let output = run_jc(&cards, &["dashboard"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pending\t1"),
        "expected pending count in fallback output, got: {}",
        stdout
    );
    assert!(
        stdout.contains("running\t1"),
        "expected running count in fallback output, got: {}",
        stdout
    );
}
