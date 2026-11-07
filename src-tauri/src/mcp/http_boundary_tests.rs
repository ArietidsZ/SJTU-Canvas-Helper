//! Loopback-only regressions using the production router and rmcp service.
//! These do not call Canvas, LLM tools, or initialize the application owner.
use std::sync::Arc;

use super::*;
use crate::mcp::lifecycle::mcp_router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::RwLock,
    task::JoinHandle,
    time::timeout,
};

struct HttpServer {
    port: u16,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl HttpServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = CancellationToken::new();
        let router = mcp_router(
            port,
            &cancel,
            &Arc::new(RwLock::new(())),
            &Arc::new(LocalSessionManager::default()),
        );
        // Deliberately do not stop the listener on cancellation: this proves
        // request admission on existing sockets independently of graceful close.
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { port, cancel, task }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

fn init_body() -> Value {
    json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
        "protocolVersion":"2025-03-26", "capabilities":{},
        "clientInfo":{"name":"http-boundary-test", "version":"1"}
    }})
}

fn rpc(text: &str) -> Value {
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|line| serde_json::from_str(line.trim()).ok())
        .or_else(|| serde_json::from_str(text).ok())
        .expect("JSON-RPC response")
}

#[tokio::test]
async fn initialize_accepts_loopback_hosts_origins_and_root_alias() {
    let server = HttpServer::start().await;
    let client = client();
    for (host, origin, path) in [
        ("127.0.0.1", None, "/mcp"),
        ("localhost", Some("localhost"), "/mcp"),
        ("127.0.0.1", Some("127.0.0.1"), "/"),
    ] {
        let mut request = client.post(server.url(path))
            .header("host", format!("{host}:{}", server.port))
            .header("accept", "application/json, text/event-stream")
            .json(&init_body());
        if let Some(origin) = origin {
            request = request.header("origin", format!("http://{origin}:{}", server.port));
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(response.headers().contains_key("mcp-session-id"));
        let message = rpc(&response.text().await.unwrap());
        assert_eq!(message["id"], 1);
        assert!(message["result"]["capabilities"]["tools"].is_object());
    }
}

#[tokio::test]
async fn initialize_rejects_untrusted_host_and_origin() {
    let server = HttpServer::start().await;
    let client = client();
    for (name, value) in [
        ("host", "evil.example"),
        ("host", "localhost:1"),
        ("host", "127.0.0.1:1"),
        ("origin", "https://evil.example"),
        ("origin", "null"),
        ("origin", "http://localhost:1"),
    ] {
        let response = client.post(server.url("/mcp"))
            .header(name, value)
            .header("accept", "application/json, text/event-stream")
            .json(&init_body())
            .send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN, "{name}: {value}");
    }
}

#[tokio::test]
async fn body_limit_allows_exactly_one_mib_and_rejects_one_more_byte() {
    let server = HttpServer::start().await;
    let client = client();
    for size in [MAX_REQUEST_BYTES, MAX_REQUEST_BYTES + 1] {
        let mut body = init_body().to_string();
        body.extend(std::iter::repeat_n(' ', size - body.len()));
        let response = client.post(server.url("/mcp"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(body).send().await.unwrap();
        assert_eq!(response.status(), if size == MAX_REQUEST_BYTES {
            reqwest::StatusCode::OK
        } else {
            reqwest::StatusCode::PAYLOAD_TOO_LARGE
        });
    }
}

async fn read_headers(connection: &mut TcpStream, deadline: Duration) -> Vec<u8> {
    timeout(deadline, async {
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            response.push(connection.read_u8().await.unwrap());
            assert!(response.len() < 16384);
        }
        response
    }).await.expect("HTTP response deadline")
}

fn assert_status(headers: &[u8], status: u16) {
    assert!(String::from_utf8_lossy(headers).starts_with(&format!("HTTP/1.1 {status} ")),
        "{}", String::from_utf8_lossy(headers));
}

#[tokio::test]
async fn chunked_body_without_content_length_is_bounded() {
    let server = HttpServer::start().await;
    let mut connection = TcpStream::connect(("127.0.0.1", server.port)).await.unwrap();
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        server.port, MAX_REQUEST_BYTES + 1,
    ).into_bytes();
    request.extend(vec![b' '; MAX_REQUEST_BYTES + 1]);
    request.extend_from_slice(b"\r\n0\r\n\r\n");
    timeout(Duration::from_secs(3), connection.write_all(&request)).await.unwrap().unwrap();
    assert_status(&read_headers(&mut connection, Duration::from_secs(3)).await, 413);
}

#[tokio::test]
async fn cancelled_token_rejects_reused_keep_alive_connection() {
    let server = HttpServer::start().await;
    let mut connection = TcpStream::connect(("127.0.0.1", server.port)).await.unwrap();
    connection.write_all(format!(
        "GET /health HTTP/1.1\r\nHost: localhost:{}\r\n\r\n", server.port,
    ).as_bytes()).await.unwrap();
    let headers = read_headers(&mut connection, Duration::from_secs(3)).await;
    assert_status(&headers, 200);
    let mut body = [0; 15];
    timeout(Duration::from_secs(3), connection.read_exact(&mut body)).await.unwrap().unwrap();
    assert_eq!(&body, br#"{"status":"ok"}"#);
    server.cancel.cancel();
    let body = init_body().to_string();
    connection.write_all(format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        server.port, body.len(),
    ).as_bytes()).await.unwrap();
    assert_status(&read_headers(&mut connection, Duration::from_secs(3)).await, 503);
    // Fresh requests are rejected as well; this is not a connection-close test.
    assert_eq!(client().get(server.url("/health")).send().await.unwrap().status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE);
}

async fn slow_body(server: &HttpServer) -> TcpStream {
    let mut connection = TcpStream::connect(("127.0.0.1", server.port)).await.unwrap();
    connection.write_all(format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\nContent-Length: 100\r\n\r\n{{", server.port,
    ).as_bytes()).await.unwrap();
    connection
}

#[tokio::test]
async fn cancellation_interrupts_body_read_before_its_deadline() {
    let server = HttpServer::start().await;
    let mut connection = slow_body(&server).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    server.cancel.cancel();
    assert_status(&read_headers(&mut connection, Duration::from_secs(1)).await, 503);
}

#[tokio::test]
async fn slow_post_body_times_out_after_ten_seconds() {
    let server = HttpServer::start().await;
    let mut connection = slow_body(&server).await;
    let started = tokio::time::Instant::now();
    assert_status(&read_headers(&mut connection, Duration::from_secs(12)).await, 408);
    assert!(started.elapsed() >= Duration::from_secs(9));
}
