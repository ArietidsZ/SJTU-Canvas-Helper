//! Deterministic admission interruption on real loopback HTTP. The write guard
//! delays only the application initialize handler; no account or tool is used.
use super::*;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::sleep;

const DEADLINE: Duration = Duration::from_secs(2);

// Observe cancellation of the actual HTTP future before releasing the handler
// gate. Otherwise the response may win the race with socket-close detection,
// in which case keeping the already-delivered session until idle expiry is valid.
struct HttpCompletion {
    sender: Option<tokio::sync::oneshot::Sender<bool>>,
    completed: bool,
}

impl Drop for HttpCompletion {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() { let _ = sender.send(self.completed); }
    }
}

async fn start_observed_server() -> (McpRuntime, tokio::sync::oneshot::Receiver<bool>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let bound_port = listener.local_addr().unwrap().port();
    let cancellation = CancellationToken::new();
    let handlers = Arc::new(RwLock::new(()));
    let sessions = Arc::new(LocalSessionManager::default());
    let (sender, observed) = tokio::sync::oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
    let router = mcp_router(bound_port, &cancellation, &handlers, &sessions)
        .layer(middleware::from_fn(move |request: axum::extract::Request, next: axum::middleware::Next| {
            let sender = sender.clone();
            async move {
                let mut probe = HttpCompletion { sender: sender.lock().unwrap().take(), completed: false };
                let response = next.run(request).await;
                probe.completed = true;
                response
            }
        }));
    let shutdown = cancellation.clone();
    let guard = cancellation.clone().drop_guard();
    let root = tokio::spawn(async move {
        let _guard = guard;
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await }).await
    });
    (McpRuntime { bound_port, cancellation, handlers, sessions, root: Some(root) }, observed)
}

fn initialize_body() -> Value {
    json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
        "protocolVersion":"2025-03-26", "capabilities":{},
        "clientInfo":{"name":"interrupted-admission-test", "version":"1"}
    }})
}

async fn blocked_initialize(runtime: &McpRuntime) -> TcpStream {
    let body = initialize_body().to_string();
    let mut connection = TcpStream::connect(("127.0.0.1", runtime.bound_port)).await.unwrap();
    connection.write_all(format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        runtime.bound_port, body.len(), body,
    ).as_bytes()).await.unwrap();
    timeout(DEADLINE, async {
        loop {
            if runtime.sessions.sessions.read().await.len() == 1 { return; }
            sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("initialize must reach session allocation before interruption");
    connection
}

async fn assert_no_sessions(runtime: &McpRuntime) {
    timeout(DEADLINE, async {
        loop {
            if runtime.sessions.sessions.read().await.is_empty() { return; }
            sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("interrupted initialize must release its unreachable session");
}

#[tokio::test]
async fn disconnected_initialize_releases_its_unreachable_session() {
    let (mut runtime, observed) = start_observed_server().await;
    let handlers = runtime.handlers.clone();
    let gate = handlers.write().await;
    let connection = blocked_initialize(&runtime).await;
    // A real client disappears before it can learn the allocated session ID.
    let socket = connection.into_std().unwrap();
    socket.shutdown(std::net::Shutdown::Both).unwrap();
    drop(socket);
    assert!(!timeout(DEADLINE, observed).await.unwrap().unwrap(),
        "the pending HTTP future must be cancelled, not completed");
    drop(gate);
    assert_no_sessions(&runtime).await;
    assert!(runtime.is_running());
    // Cancellation must be scoped to the abandoned handshake, not the listener.
    let client = reqwest::Client::builder().no_proxy().timeout(DEADLINE).build().unwrap();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    let response = client.post(&url).header("accept", "application/json, text/event-stream")
        .json(&initialize_body()).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let session_id = response.headers()["mcp-session-id"].to_str().unwrap().to_owned();
    let body = response.text().await.unwrap();
    assert!(body.lines().filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .any(|message| message["id"] == 1 && message["result"].is_object()));
    assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
    assert_eq!(client.delete(&url).header("mcp-session-id", session_id)
        .header("mcp-protocol-version", "2025-03-26").send().await.unwrap().status(),
        reqwest::StatusCode::ACCEPTED);
    assert_no_sessions(&runtime).await;
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_during_initialize_releases_the_pending_session() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let handlers = runtime.handlers.clone();
    let gate = handlers.write().await;
    let mut connection = blocked_initialize(&runtime).await;
    // stop() is deliberately interrupted while waiting for the held gate.
    assert!(timeout(Duration::from_millis(40), runtime.stop()).await.is_err());
    assert!(runtime.cancellation.is_cancelled());
    assert!(!runtime.is_running());
    drop(gate);
    assert_no_sessions(&runtime).await;
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(runtime.root.is_none());
    use tokio::io::AsyncReadExt;
    let mut response = String::new();
    timeout(DEADLINE, connection.read_to_string(&mut response)).await.unwrap().unwrap();
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", runtime.bound_port)).await.unwrap();
    drop(listener);
}
