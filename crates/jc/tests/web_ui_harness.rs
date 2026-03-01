use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread::sleep;
use std::time::Duration;

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

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().unwrap().port()
}

struct ServerGuard {
    child: Child,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_server(port: u16) {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(250))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{}/openapi.json", port);
    for _ in 0..80 {
        if let Ok(resp) = client.get(&url).send() {
            if resp.status().is_success() {
                return;
            }
        }
        sleep(Duration::from_millis(100));
    }
    panic!("server did not become ready at {}", url);
}

fn start_server(cards: &Path, port: u16, ui: bool) -> ServerGuard {
    let mut cmd = Command::new(jc_bin());
    cmd.args([
        "--cards-dir",
        cards.to_str().unwrap(),
        "serve",
        "--port",
        &port.to_string(),
    ]);
    if ui {
        cmd.arg("--ui");
    }
    let child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn jc serve");
    wait_for_server(port);
    ServerGuard { child }
}

#[test]
fn serve_ui_exposes_dashboard_sse_details_and_providers() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let new_job = run_jc(&cards, &["new", "implement", "ui-job"]);
    assert!(new_job.status.success());
    fs::rename(
        cards.join("pending").join("ui-job.jobcard"),
        cards.join("done").join("ui-job.jobcard"),
    )
    .unwrap();

    let card = cards.join("done").join("ui-job.jobcard");
    fs::write(card.join("spec.md"), "UI test spec\n").unwrap();
    fs::create_dir_all(card.join("logs")).unwrap();
    fs::create_dir_all(card.join("output")).unwrap();
    fs::write(card.join("logs").join("stdout.log"), "stdout line\n").unwrap();
    fs::write(card.join("logs").join("stderr.log"), "stderr line\n").unwrap();
    fs::write(card.join("logs").join("qa.log"), "qa check ok\n").unwrap();
    fs::write(card.join("output").join("result.txt"), "artifact payload\n").unwrap();

    let port = free_port();
    let _server = start_server(&cards, port, true);
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::blocking::Client::new();

    let dashboard_resp = client.get(format!("{}/ui", base)).send().unwrap();
    assert_eq!(dashboard_resp.status(), reqwest::StatusCode::OK);
    let dashboard_html = dashboard_resp.text().unwrap();
    assert!(dashboard_html.contains("Job Dashboard"));
    assert!(dashboard_html.contains("/ui/jobs/ui-job"));
    assert!(dashboard_html.contains("EventSource(\"/ui/events\")"));

    let details_resp = client
        .get(format!("{}/ui/jobs/ui-job", base))
        .send()
        .unwrap();
    assert_eq!(details_resp.status(), reqwest::StatusCode::OK);
    let details_html = details_resp.text().unwrap();
    assert!(details_html.contains("<h2>Spec</h2>"));
    assert!(details_html.contains("UI test spec"));
    assert!(details_html.contains("<h2>Logs</h2>"));
    assert!(details_html.contains("stdout line"));
    assert!(details_html.contains("<h2>Output</h2>"));
    assert!(details_html.contains("artifact payload"));
    assert!(details_html.contains("<h2>Audit Trail</h2>"));

    let providers_resp = client.get(format!("{}/ui/providers", base)).send().unwrap();
    assert_eq!(providers_resp.status(), reqwest::StatusCode::OK);
    let providers_html = providers_resp.text().unwrap();
    assert!(providers_html.contains("Providers"));
    assert!(providers_html.contains("mock"));

    let sse_resp = client
        .get(format!("{}/ui/events?once=true", base))
        .send()
        .unwrap();
    assert_eq!(sse_resp.status(), reqwest::StatusCode::OK);
    let sse_body = sse_resp.text().unwrap();
    assert!(sse_body.contains("event: jobs"));
    assert!(sse_body.contains("\"id\":\"ui-job\""));
}
