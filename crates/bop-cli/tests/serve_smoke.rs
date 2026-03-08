use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
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

fn bop_bin() -> PathBuf {
    repo_root().join("target").join("debug").join("bop")
}

fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind random port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn wait_for_server(port: u16, timeout: Duration) -> bool {
    let addr = format!("127.0.0.1:{}", port);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(&addr).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn send_http_request(port: u16, body: &str, token: &str) -> String {
    let addr = format!("127.0.0.1:{}", port);
    let request = format!(
        "POST /cards/new HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        addr,
        token,
        body.len(),
        body
    );
    let mut stream = TcpStream::connect(&addr).expect("connect to serve");
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
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

fn start_server(cards_dir: &str, port: u16, token: &str) -> ServerGuard {
    let child = Command::new(bop_bin())
        .args([
            "--cards-dir",
            cards_dir,
            "serve",
            "--port",
            &port.to_string(),
        ])
        .env("BOP_SERVE_TOKEN", token)
        .spawn()
        .expect("failed to start bop serve");

    ServerGuard { child }
}

#[test]
fn test_serve_smoke() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    // Initialize cards directory
    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    // Find a free port
    let port = find_free_port();

    // Give OS time to release the port
    std::thread::sleep(Duration::from_millis(200));

    // Start server with a known token
    let token = "smoke-test-token-12345";
    let _server = start_server(cards.to_str().unwrap(), port, token);

    // Wait for server to be ready
    assert!(
        wait_for_server(port, Duration::from_secs(10)),
        "server did not start within 10 seconds"
    );

    // Create a card via POST request
    let payload = serde_json::json!({
        "id": "smoke-test",
        "spec": "# Test\nSmoke test spec"
    })
    .to_string();

    let response = send_http_request(port, &payload, token);

    // Verify 201 CREATED response
    assert!(
        response.starts_with("HTTP/1.1 201"),
        "expected HTTP 201 response, got: {}",
        response
    );

    // Verify card exists in pending directory
    let pending = cards.join("pending");
    let found = std::fs::read_dir(&pending)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("smoke-test"));

    assert!(found, "card should be in pending/");
}

#[test]
fn test_serve_rejects_path_traversal() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let port = find_free_port();
    std::thread::sleep(Duration::from_millis(200));

    let token = "test-token";
    let _server = start_server(cards.to_str().unwrap(), port, token);

    assert!(
        wait_for_server(port, Duration::from_secs(10)),
        "server did not start"
    );

    // Test .. rejection (without path separators to test the .. check specifically)
    let payload = serde_json::json!({
        "id": "test..card",
        "spec": "# Malicious"
    })
    .to_string();

    let response = send_http_request(port, &payload, token);
    assert!(
        response.contains("400") && response.contains("'..'"),
        "should reject .. in id, got: {}",
        response
    );
}

#[test]
fn test_serve_rejects_url_encoding() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let port = find_free_port();
    std::thread::sleep(Duration::from_millis(200));

    let token = "test-token";
    let _server = start_server(cards.to_str().unwrap(), port, token);

    assert!(
        wait_for_server(port, Duration::from_secs(10)),
        "server did not start"
    );

    // Test % rejection
    let payload = serde_json::json!({
        "id": "test%2F%2Fmalicious",
        "spec": "# Test"
    })
    .to_string();

    let response = send_http_request(port, &payload, token);
    assert!(
        response.contains("400") && response.contains("'%'"),
        "should reject % in id"
    );
}

#[test]
fn test_serve_rejects_invalid_chars() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let port = find_free_port();
    std::thread::sleep(Duration::from_millis(200));

    let token = "test-token";
    let _server = start_server(cards.to_str().unwrap(), port, token);

    assert!(
        wait_for_server(port, Duration::from_secs(10)),
        "server did not start"
    );

    // Test special character rejection
    let payload = serde_json::json!({
        "id": "test@card#123",
        "spec": "# Test"
    })
    .to_string();

    let response = send_http_request(port, &payload, token);
    assert!(
        response.contains("400") && response.contains("alphanumeric"),
        "should reject special chars in id"
    );
}

#[test]
fn test_serve_accepts_valid_chars() {
    build_jc();

    let td = tempfile::tempdir().unwrap();
    let cards = td.path().join(".cards");

    let status = Command::new(bop_bin())
        .args(["--cards-dir", cards.to_str().unwrap(), "init"])
        .status()
        .unwrap();
    assert!(status.success());

    let port = find_free_port();
    std::thread::sleep(Duration::from_millis(200));

    let token = "test-token";
    let _server = start_server(cards.to_str().unwrap(), port, token);

    assert!(
        wait_for_server(port, Duration::from_secs(10)),
        "server did not start"
    );

    // Test that valid chars (alphanumeric, dash, underscore, dot) are accepted
    let payload = serde_json::json!({
        "id": "test-card_123.v2",
        "spec": "# Test"
    })
    .to_string();

    let response = send_http_request(port, &payload, token);
    assert!(
        response.starts_with("HTTP/1.1 201"),
        "should accept valid chars, got: {}",
        response
    );

    let pending = cards.join("pending");
    let found = std::fs::read_dir(&pending)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().contains("test-card_123"));

    assert!(found, "card with valid chars should be created");
}
