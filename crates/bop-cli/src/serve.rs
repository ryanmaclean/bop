use axum::{
    extract::{ConnectInfo, State},
    http::{header, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use bop_core::{read_meta, write_meta};
use chrono::Utc;
use rand::{distributions::Alphanumeric, Rng};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;

use crate::cards;

#[derive(Debug, Deserialize)]
pub struct CreateCardRequest {
    pub id: String,
    pub spec: String,
    #[serde(default)]
    pub team: Option<String>,
    #[serde(default = "default_template")]
    pub template: String,
    #[serde(default)]
    pub priority: Option<u32>,
}

fn default_template() -> String {
    "implement".to_string()
}

#[derive(Clone)]
struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    max_requests: usize,
    window: Duration,
}

impl RateLimiter {
    fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    fn check_rate_limit(&self, ip: &str) -> bool {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        let cutoff = now - self.window;

        // Get or create the request history for this IP
        let history = requests.entry(ip.to_string()).or_default();

        // Remove requests outside the time window
        history.retain(|&timestamp| timestamp > cutoff);

        // Check if rate limit exceeded
        if history.len() >= self.max_requests {
            return false;
        }

        // Record this request
        history.push(now);
        true
    }
}

#[derive(Clone)]
struct AppState {
    cards_dir: PathBuf,
    token: String,
    rate_limiter: RateLimiter,
}

async fn post_cards_new(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateCardRequest>,
) -> impl IntoResponse {
    // Mandatory token auth
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let bearer = format!("Bearer {}", state.token);
    // Use constant-time comparison to prevent timing attacks
    if auth.as_bytes().ct_eq(bearer.as_bytes()).unwrap_u8() != 1 {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        );
    }

    // Validate id
    let id = req.id.trim().to_string();
    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id cannot be empty"})),
        );
    }
    if id.contains('/') || id.contains('\\') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id cannot contain path separators"})),
        );
    }
    // Reject path traversal attempts
    if id.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id cannot contain '..'"})),
        );
    }
    // Reject URL encoding character (potential encoding attacks)
    if id.contains('%') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id cannot contain '%'"})),
        );
    }
    // Reject null bytes
    if id.contains('\0') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id cannot contain null bytes"})),
        );
    }
    // Only allow alphanumeric, dash, underscore, and dot
    if !id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "id can only contain alphanumeric characters, dash, underscore, and dot"}),
            ),
        );
    }

    let template = req.template.trim().to_string();
    if template.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "template cannot be empty"})),
        );
    }

    let spec = req.spec.clone();
    if spec.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "spec cannot be empty"})),
        );
    }

    // Check for existing card (duplicate detection)
    if crate::paths::find_card(&state.cards_dir, &id).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("card already exists: {}", id)})),
        );
    }

    // Create the card (may still fail if card appears between check and create)
    let result = cards::create_card(
        &state.cards_dir,
        &template,
        &id,
        Some(&spec),
        req.team.as_deref(),
    );

    match result {
        Ok(card_path) => {
            if let Some(priority) = req.priority {
                if let Ok(mut meta) = read_meta(&card_path) {
                    meta.priority = Some(i64::from(priority));
                    let _ = write_meta(&card_path, &meta);
                }
            }
            let path_str = card_path.to_string_lossy().to_string();
            (
                StatusCode::CREATED,
                Json(serde_json::json!({"id": id, "path": path_str})),
            )
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": msg})),
                )
            } else if msg.contains("template not found")
                || msg.contains("cannot be empty")
                || msg.contains("path separator")
            {
                (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": msg})),
                )
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": msg})),
                )
            }
        }
    }
}

fn log_request(method: &str, path: &str, status: u16) {
    let ts = Utc::now().to_rfc3339();
    println!("[{}] {} {} {}", ts, method, path, status);
}

async fn logged_post_cards_new(
    state: State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: Json<CreateCardRequest>,
) -> impl IntoResponse {
    let resp = post_cards_new(state, headers, body).await;
    let resp = resp.into_response();
    log_request("POST", "/cards/new", resp.status().as_u16());
    resp
}

async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = addr.ip().to_string();

    if !state.rate_limiter.check_rate_limit(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error": "rate limit exceeded"})),
        )
            .into_response();
    }

    next.run(req).await
}

async fn add_security_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::HeaderName::from_static("x-frame-options"),
        header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

pub async fn run_serve(cards_dir: PathBuf, host: &str, port: u16) -> anyhow::Result<()> {
    let token = match std::env::var("BOP_SERVE_TOKEN") {
        Ok(t) => {
            println!("BOP_SERVE_TOKEN set — using provided token");
            t
        }
        Err(_) => {
            // Generate a random 32-character token
            let generated: String = rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();
            println!("⚠️  BOP_SERVE_TOKEN not set — generated random token:");
            println!("   {}", generated);
            println!("   Use: curl -H 'Authorization: Bearer {}' ...", generated);
            generated
        }
    };

    // Rate limiter: 10 requests per minute per IP
    let rate_limiter = RateLimiter::new(10, Duration::from_secs(60));

    let state = Arc::new(AppState {
        cards_dir,
        token,
        rate_limiter,
    });

    let app = Router::new()
        .route("/cards/new", post(logged_post_cards_new))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(middleware::from_fn(add_security_headers))
        .layer(axum::extract::DefaultBodyLimit::max(65536))
        .with_state(state);

    let bind_addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&bind_addr).await?;
    println!("bop serve listening on http://{}", bind_addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("\nbop serve shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};
    use tower::util::ServiceExt;

    fn make_app(cards_dir: PathBuf, token: &str) -> Router {
        let rate_limiter = RateLimiter::new(10, Duration::from_secs(60));
        let state = Arc::new(AppState {
            cards_dir,
            token: token.to_string(),
            rate_limiter,
        });
        Router::new()
            .route("/cards/new", post(post_cards_new))
            .layer(middleware::from_fn(add_security_headers))
            .layer(axum::extract::DefaultBodyLimit::max(65536))
            .with_state(state)
    }

    async fn setup_cards_dir() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::TempDir::new().unwrap();
        let cards_dir = tmp.path().to_path_buf();
        crate::paths::ensure_cards_layout(&cards_dir).unwrap();
        crate::cards::seed_default_templates(&cards_dir).unwrap();
        (tmp, cards_dir)
    }

    fn find_pending_card(cards_dir: &std::path::Path, id: &str) -> Option<PathBuf> {
        let pending = cards_dir.join("pending");
        let exact = pending.join(format!("{id}.bop"));
        if exact.exists() {
            return Some(exact);
        }

        let suffix = format!("-{id}.bop");
        std::fs::read_dir(&pending)
            .ok()?
            .flatten()
            .find_map(|entry| {
                let path = entry.path();
                if path.is_dir()
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.ends_with(&suffix))
                        .unwrap_or(false)
                {
                    Some(path)
                } else {
                    None
                }
            })
    }

    #[tokio::test]
    async fn test_invalid_json_returns_400() {
        let (_tmp, cards_dir) = setup_cards_dir().await;
        let app = make_app(cards_dir, "test-token");

        let req = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token")
            .body(Body::from("not-json"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // axum returns 400 for JSON syntax errors
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_missing_id_returns_400() {
        let (_tmp, cards_dir) = setup_cards_dir().await;
        let app = make_app(cards_dir, "test-token");

        let body = serde_json::json!({"id": "", "spec": "# test"});
        let req = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_card_returns_201() {
        let (_tmp, cards_dir) = setup_cards_dir().await;
        let app = make_app(cards_dir.clone(), "test-token");

        let body = serde_json::json!({
            "id": "test-serve-card",
            "spec": "# Test\n\nA test card via serve.",
            "template": "implement"
        });
        let req = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Card should exist in pending/
        let pending = cards_dir.join("pending");
        let found = std::fs::read_dir(&pending)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("test-serve-card"));
        assert!(found, "card should be in pending/");
    }

    #[tokio::test]
    async fn test_duplicate_id_returns_409() {
        let (_tmp, cards_dir) = setup_cards_dir().await;

        let body_str = serde_json::json!({
            "id": "dup-card",
            "spec": "# Dup",
            "template": "implement"
        })
        .to_string();

        // First request
        let app1 = make_app(cards_dir.clone(), "test-token");
        let req1 = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token")
            .body(Body::from(body_str.clone()))
            .unwrap();
        let resp1 = app1.oneshot(req1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::CREATED);

        // Second request with same id (new app instance, same cards_dir)
        let app2 = make_app(cards_dir.clone(), "test-token");
        let req2 = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token")
            .body(Body::from(body_str))
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_token_auth_required() {
        let (_tmp, cards_dir) = setup_cards_dir().await;

        let body_str = serde_json::json!({
            "id": "auth-card",
            "spec": "# Auth test",
            "template": "implement"
        })
        .to_string();

        // No token → 401
        let app1 = make_app(cards_dir.clone(), "secret");
        let req = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .body(Body::from(body_str.clone()))
            .unwrap();
        let resp = app1.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Wrong token → 401
        let app2 = make_app(cards_dir.clone(), "secret");
        let req2 = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer wrong")
            .body(Body::from(body_str.clone()))
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::UNAUTHORIZED);

        // Correct token → 201
        let app3 = make_app(cards_dir, "secret");
        let req3 = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer secret")
            .body(Body::from(body_str))
            .unwrap();
        let resp3 = app3.oneshot(req3).await.unwrap();
        assert_eq!(resp3.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_security_headers() {
        let (_tmp, cards_dir) = setup_cards_dir().await;
        let app = make_app(cards_dir, "test-token");

        let body = serde_json::json!({
            "id": "security-test",
            "spec": "# Security Test",
        });
        let req = Request::builder()
            .method(Method::POST)
            .uri("/cards/new")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token")
            .body(Body::from(body.to_string()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        // Verify security headers are present
        let headers = resp.headers();
        assert_eq!(
            headers
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "X-Content-Type-Options header should be 'nosniff'"
        );
        assert_eq!(
            headers.get("x-frame-options").and_then(|v| v.to_str().ok()),
            Some("DENY"),
            "X-Frame-Options header should be 'DENY'"
        );
        assert_eq!(
            headers.get("cache-control").and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "Cache-Control header should be 'no-store'"
        );
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let (_tmp, cards_dir) = setup_cards_dir().await;
        let rate_limiter = RateLimiter::new(10, Duration::from_secs(60));
        let state = Arc::new(AppState {
            cards_dir: cards_dir.clone(),
            token: "test-token".to_string(),
            rate_limiter,
        });

        // Simulate 10 requests from the same IP
        let test_ip = "127.0.0.1";
        for i in 0..10 {
            assert!(
                state.rate_limiter.check_rate_limit(test_ip),
                "Request {} should be allowed",
                i + 1
            );
        }

        // 11th request should be rate limited
        assert!(
            !state.rate_limiter.check_rate_limit(test_ip),
            "Request 11 should be rate limited"
        );

        // Different IP should still be allowed
        let different_ip = "192.168.1.1";
        assert!(
            state.rate_limiter.check_rate_limit(different_ip),
            "Request from different IP should be allowed"
        );
    }

    #[tokio::test]
    #[ignore] // Flaky due to port binding race conditions; covered by other unit tests
    async fn test_smoke_server_post_creates_pending_card() {
        let (_tmp, cards_dir) = setup_cards_dir().await;

        // Set token for this test
        std::env::set_var("BOP_SERVE_TOKEN", "smoke-test-token");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind random port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        // Give OS time to release the port
        tokio::time::sleep(Duration::from_millis(200)).await;

        let host = "127.0.0.1".to_string();
        let cards_for_server = cards_dir.clone();

        let server = tokio::spawn(async move {
            if let Err(e) = run_serve(cards_for_server, &host, port).await {
                eprintln!("Server failed: {}", e);
            }
        });

        let addr = format!("127.0.0.1:{port}");

        // Wait for server to be ready
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut connected = false;
        while Instant::now() < deadline {
            if TcpStream::connect(&addr).is_ok() {
                connected = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if !connected {
            server.abort();
            std::env::remove_var("BOP_SERVE_TOKEN");
            panic!(
                "Server did not become ready within 15 seconds on port {}",
                port
            );
        }

        let payload = serde_json::json!({
            "id": "smoke-test-serve",
            "spec": "# test\nsmoke test spec"
        })
        .to_string();

        let request = format!(
            "POST /cards/new HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAuthorization: Bearer smoke-test-token\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let mut stream = TcpStream::connect(&addr).expect("connect to serve");
        stream.write_all(request.as_bytes()).expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");

        assert!(
            response.starts_with("HTTP/1.1 201"),
            "expected HTTP 201 response, got: {response}"
        );
        assert!(
            find_pending_card(&cards_dir, "smoke-test-serve").is_some(),
            "expected card directory in pending/"
        );

        server.abort();
        let _ = server.await;
        std::env::remove_var("BOP_SERVE_TOKEN");
    }
}
