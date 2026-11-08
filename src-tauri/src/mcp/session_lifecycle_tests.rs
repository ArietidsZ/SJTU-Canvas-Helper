//! Real loopback sessions and SSE streams. No application tools, account data,
//! Canvas requests or model requests are used by these regressions.
use super::*;
use serde_json::{json, Value};
use tokio::time::sleep;

const DEADLINE: Duration = Duration::from_secs(2);
const PROTOCOL: &str = "2025-03-26";

struct Session {
    client: reqwest::Client,
    url: String,
    id: String,
}

fn rpc(text: &str) -> Value {
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|line| serde_json::from_str(line.trim()).ok())
        .or_else(|| serde_json::from_str(text).ok())
        .expect("JSON-RPC response")
}

impl Session {
    async fn initialize(port: u16) -> Self {
        // A whole-response timeout would itself close a healthy SSE stream.
        // Every operation below has its own bounded deadline instead.
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{port}/mcp");
        let response = timeout(DEADLINE, client.post(&url)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                "protocolVersion":PROTOCOL, "capabilities":{},
                "clientInfo":{"name":"session-lifecycle-test", "version":"1"}
            }}))
            .send()).await.unwrap().unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let id = response.headers()["mcp-session-id"].to_str().unwrap().to_owned();
        let body = timeout(DEADLINE, response.text()).await.unwrap().unwrap();
        assert_eq!(rpc(&body)["id"], 1);
        assert!(rpc(&body)["result"]["capabilities"]["tools"].is_object());
        let session = Self { client, url, id };
        let response = session.post(json!({
            "jsonrpc":"2.0", "method":"notifications/initialized"
        })).await;
        assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
        session
    }

    fn request(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        self.client.request(method, &self.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", &self.id)
            .header("mcp-protocol-version", PROTOCOL)
    }

    async fn post(&self, message: Value) -> reqwest::Response {
        timeout(DEADLINE, self.request(reqwest::Method::POST).json(&message).send())
            .await.unwrap().unwrap()
    }

    async fn call(&self, message: Value) -> Value {
        let response = self.post(message).await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        rpc(&timeout(DEADLINE, response.text()).await.unwrap().unwrap())
    }

    async fn stream(&self) -> reqwest::Response {
        let response = timeout(DEADLINE, self.request(reqwest::Method::GET).send())
            .await.unwrap().unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(response.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
        response
    }
}

async fn assert_stream_open(response: &mut reqwest::Response) {
    // Ignore an optional initial SSE comment. Prove the connection remains
    // pending before shutdown instead of accepting an already-closed stream.
    for _ in 0..4 {
        match timeout(Duration::from_millis(40), response.chunk()).await {
            Err(_) => return,
            Ok(Ok(Some(_))) => {}
            result => panic!("SSE stream was not open: {result:?}"),
        }
    }
    panic!("unexpected continuous data in idle SSE stream");
}

async fn assert_stream_ended(mut response: reqwest::Response) {
    timeout(DEADLINE, async {
        let mut bytes = 0;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    bytes += chunk.len();
                    assert!(bytes < 16_384, "unexpected idle-stream output");
                }
                Ok(None) => return,
                // Abrupt closure on runtime Drop is permitted. A request/body
                // timeout is not evidence that the server closed the stream.
                Err(error) if !error.is_timeout() => return,
                Err(error) => panic!("SSE timed out instead of closing: {error}"),
            }
        }
    }).await.expect("runtime must close an admitted SSE stream");
}

#[tokio::test]
async fn graceful_stop_closes_initialized_sse_and_restart_rejects_old_session() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let port = runtime.bound_port;
    let old = Session::initialize(port).await;
    let mut stream = old.stream().await;
    assert_stream_open(&mut stream).await;
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(runtime.root.is_none());
    assert_stream_ended(stream).await;

    // Switching policy here also verifies old GUI sessions cannot survive a
    // restart into headless mode and retain the GUI's sensitive tool routes.
    let mut replacement = start_headless_mcp_server(port).await.unwrap();
    let stale = old.post(json!({"jsonrpc":"2.0", "id":2, "method":"tools/list"})).await;
    assert_eq!(stale.status(), reqwest::StatusCode::NOT_FOUND);
    let fresh = Session::initialize(port).await;
    assert_ne!(fresh.id, old.id);
    let tools = fresh.call(json!({"jsonrpc":"2.0", "id":3, "method":"tools/list"})).await;
    let names: Vec<_> = tools["result"]["tools"].as_array().unwrap()
        .iter().map(|tool| tool["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"list_courses"));
    for name in ["chat", "explain_file", "test_token"] {
        assert!(!names.contains(&name), "headless discovery exposed {name}");
    }
    for (name, arguments) in [
        ("chat", json!({"prompt":"synthetic canary"})),
        ("explain_file", json!({"course_id":1, "file_id":2})),
        ("test_token", json!({"token":"synthetic-canary-not-a-credential"})),
    ] {
        let message = fresh.call(json!({"jsonrpc":"2.0", "id":4, "method":"tools/call",
            "params":{"name":name, "arguments":arguments}})).await;
        assert!(message["error"].is_object(), "headless invocation allowed {name}: {message}");
        assert!(!message.to_string().contains("synthetic"));
    }
    timeout(DEADLINE, replacement.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn dropping_runtime_closes_initialized_sse_and_releases_port() {
    let runtime = start_mcp_server(0).await.unwrap();
    let port = runtime.bound_port;
    let session = Session::initialize(port).await;
    let mut stream = session.stream().await;
    assert_stream_open(&mut stream).await;
    let cancelled = runtime.cancellation.clone();
    drop(runtime);
    assert!(cancelled.is_cancelled());
    assert_stream_ended(stream).await;
    timeout(DEADLINE, async {
        loop {
            match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => { drop(listener); return; }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => sleep(Duration::from_millis(5)).await,
                Err(error) => panic!("rebind failed: {error}"),
            }
        }
    }).await.expect("drop must release the listener");
}

#[tokio::test]
async fn dropped_sse_client_can_reconnect_and_use_the_same_session() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let session = Session::initialize(runtime.bound_port).await;
    let mut first = session.stream().await;
    assert_stream_open(&mut first).await;
    drop(first);
    // A disconnected SSE channel must not delete its initialized MCP session.
    let listed = session.call(json!({"jsonrpc":"2.0", "id":2, "method":"tools/list"})).await;
    assert!(listed["result"]["tools"].is_array());
    let mut second = session.stream().await;
    assert_stream_open(&mut second).await;
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert_stream_ended(second).await;
}

#[tokio::test]
async fn interrupted_stop_closes_sse_and_retains_state_for_retry() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let session = Session::initialize(runtime.bound_port).await;
    let mut stream = session.stream().await;
    assert_stream_open(&mut stream).await;
    let handlers = runtime.handlers.clone();
    // Explicit fixture: this lock models an uncooperative application handler.
    let held = handlers.read().await;
    assert!(timeout(Duration::from_millis(40), runtime.stop()).await.is_err());
    assert!(runtime.root.is_some());
    assert!(runtime.cancellation.is_cancelled());
    assert!(!runtime.is_running());
    assert_stream_ended(stream).await;
    drop(held);
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(runtime.root.is_none());
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}
