use serde_json::{json, Value};
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

fn start_server(cards: &Path, port: u16) -> ServerGuard {
    let child = Command::new(jc_bin())
        .args([
            "--cards-dir",
            cards.to_str().unwrap(),
            "serve",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn jc serve");
    wait_for_server(port);
    ServerGuard { child }
}

#[test]
fn rest_api_create_list_inspect_and_kill() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let port = free_port();
    let _server = start_server(&cards, port);
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::blocking::Client::new();

    let create_resp = client
        .post(format!("{}/jobs", base))
        .json(&json!({
            "template": "implement",
            "id": "api-job",
            "spec": "REST-created spec"
        }))
        .send()
        .unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::CREATED);
    let created: Value = create_resp.json().unwrap();
    assert_eq!(created["job"]["id"], "api-job");
    assert_eq!(created["job"]["state"], "pending");
    assert_eq!(
        fs::read_to_string(
            cards
                .join("pending")
                .join("api-job.jobcard")
                .join("spec.md")
        )
        .unwrap(),
        "REST-created spec"
    );

    let list_resp = client.get(format!("{}/jobs", base)).send().unwrap();
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list: Value = list_resp.json().unwrap();
    assert!(list.as_array().unwrap().iter().any(|job| {
        job.get("id").and_then(Value::as_str) == Some("api-job")
            && job.get("state").and_then(Value::as_str) == Some("pending")
    }));

    let inspect_resp = client.get(format!("{}/jobs/api-job", base)).send().unwrap();
    assert_eq!(inspect_resp.status(), reqwest::StatusCode::OK);
    let inspect: Value = inspect_resp.json().unwrap();
    assert_eq!(inspect["job"]["id"], "api-job");
    assert_eq!(inspect["spec"], "REST-created spec");

    let retry_resp = client
        .post(format!("{}/jobs/api-job/retry", base))
        .send()
        .unwrap();
    assert_eq!(retry_resp.status(), reqwest::StatusCode::CONFLICT);

    fs::rename(
        cards.join("pending").join("api-job.jobcard"),
        cards.join("running").join("api-job.jobcard"),
    )
    .unwrap();

    let mut sleep_child = Command::new("sleep").arg("60").spawn().unwrap();
    fs::write(
        cards
            .join("running")
            .join("api-job.jobcard")
            .join("logs")
            .join("pid"),
        sleep_child.id().to_string(),
    )
    .unwrap();

    let kill_resp = client
        .delete(format!("{}/jobs/api-job", base))
        .send()
        .unwrap();
    assert_eq!(kill_resp.status(), reqwest::StatusCode::OK);
    let killed: Value = kill_resp.json().unwrap();
    assert_eq!(killed["job"]["state"], "failed");

    let _ = sleep_child.wait();

    let failed_card = cards.join("failed").join("api-job.jobcard");
    assert!(failed_card.exists());
    let failed_meta: Value =
        serde_json::from_str(&fs::read_to_string(failed_card.join("meta.json")).unwrap()).unwrap();
    assert_eq!(failed_meta["failure_reason"], "killed");
}

#[test]
fn rest_api_serves_openapi_and_sse_logs() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let init = run_jc(&cards, &["init"]);
    assert!(init.status.success());

    let new_job = run_jc(&cards, &["new", "implement", "log-job"]);
    assert!(new_job.status.success());
    fs::rename(
        cards.join("pending").join("log-job.jobcard"),
        cards.join("done").join("log-job.jobcard"),
    )
    .unwrap();

    let card = cards.join("done").join("log-job.jobcard");
    fs::create_dir_all(card.join("logs")).unwrap();
    fs::write(card.join("logs").join("stdout.log"), "stdout line\n").unwrap();
    fs::write(card.join("logs").join("stderr.log"), "stderr line\n").unwrap();

    let port = free_port();
    let _server = start_server(&cards, port);
    let base = format!("http://127.0.0.1:{}", port);
    let client = reqwest::blocking::Client::new();

    let openapi_resp = client.get(format!("{}/openapi.json", base)).send().unwrap();
    assert_eq!(openapi_resp.status(), reqwest::StatusCode::OK);
    let openapi: Value = openapi_resp.json().unwrap();
    assert!(openapi["paths"].get("/jobs").is_some());
    assert!(openapi["paths"].get("/jobs/{id}/logs").is_some());

    let sse_resp = client
        .get(format!("{}/jobs/log-job/logs", base))
        .send()
        .unwrap();
    assert_eq!(sse_resp.status(), reqwest::StatusCode::OK);
    let sse_body = sse_resp.text().unwrap();
    assert!(sse_body.contains("event: stdout"));
    assert!(sse_body.contains("stdout line"));
    assert!(sse_body.contains("event: stderr"));
    assert!(sse_body.contains("stderr line"));
}
