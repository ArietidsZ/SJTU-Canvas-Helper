//! Real loopback protocol admission and session isolation. Application tools,
//! accounts, Canvas and model providers are never invoked.
use super::*;
use serde_json::{json, Value};

const DEADLINE: Duration = Duration::from_secs(2);

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().timeout(DEADLINE).build().unwrap()
}

#[tokio::test]
async fn non_initialize_without_session_does_not_allocate_session_handles() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let client = client();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    assert!(runtime.sessions.sessions.read().await.is_empty());
    for message in [
        json!({"jsonrpc":"2.0", "id":1, "method":"ping"}),
        json!({"jsonrpc":"2.0", "id":2, "method":"tools/list"}),
        json!({"jsonrpc":"2.0", "method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0", "method":"notifications/cancelled", "params":{"requestId":1}}),
    ] {
        let response = client.post(&url)
            .header("accept", "application/json, text/event-stream")
            .json(&message).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!response.headers().contains_key("mcp-session-id"));
        // Repeat the request to model a misconfigured client reconnect loop.
        for _ in 0..3 {
            assert_eq!(client.post(&url)
                .header("accept", "application/json, text/event-stream")
                .json(&message).send().await.unwrap().status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
        }
        tokio::task::yield_now().await;
        assert!(runtime.sessions.sessions.read().await.is_empty(),
            "rejected request retained a session: {message}");
    }
    assert!(runtime.is_running());
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

struct Session {
    client: reqwest::Client,
    url: String,
    id: String,
}

fn rpc(text: &str) -> Value {
    text.lines().filter_map(|line| line.strip_prefix("data:"))
        .find_map(|line| serde_json::from_str(line.trim()).ok())
        .or_else(|| serde_json::from_str(text).ok()).expect("JSON-RPC response")
}

impl Session {
    async fn initialize(port: u16) -> Self {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{port}/mcp");
        let response = timeout(DEADLINE, client.post(&url)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
                "protocolVersion":"2025-03-26", "capabilities":{},
                "clientInfo":{"name":"session-admission-test", "version":"1"}
            }})).send()).await.unwrap().unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let id = response.headers()["mcp-session-id"].to_str().unwrap().to_owned();
        let body = timeout(DEADLINE, response.text()).await.unwrap().unwrap();
        assert_eq!(rpc(&body)["id"], 1);
        let session = Self { client, url, id };
        assert_eq!(session.post(json!({"jsonrpc":"2.0", "method":"notifications/initialized"})).await.status(),
            reqwest::StatusCode::ACCEPTED);
        session
    }

    fn request(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        self.client.request(method, &self.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", &self.id)
            .header("mcp-protocol-version", "2025-03-26")
    }

    async fn post(&self, body: Value) -> reqwest::Response {
        timeout(DEADLINE, self.request(reqwest::Method::POST).json(&body).send()).await.unwrap().unwrap()
    }

    async fn ping(&self) {
        let response = self.post(json!({"jsonrpc":"2.0", "id":2, "method":"ping"})).await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = timeout(DEADLINE, response.text()).await.unwrap().unwrap();
        assert_eq!(rpc(&body)["id"], 2);
        assert!(rpc(&body)["result"].is_object());
    }

    async fn stream(&self) -> reqwest::Response {
        let response = timeout(DEADLINE, self.request(reqwest::Method::GET).send()).await.unwrap().unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        response
    }
}

async fn assert_stream_open(response: &mut reqwest::Response) {
    for _ in 0..4 {
        match timeout(Duration::from_millis(40), response.chunk()).await {
            Err(_) => return,
            Ok(Ok(Some(_))) => {},
            result => panic!("SSE stream unexpectedly closed: {result:?}"),
        }
    }
    panic!("unexpected continuous idle-stream data");
}

async fn assert_stream_ended(mut response: reqwest::Response) {
    timeout(DEADLINE, async {
        let mut bytes = 0;
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => { bytes += chunk.len(); assert!(bytes < 16_384); },
                Ok(None) => return,
                Err(error) if !error.is_timeout() => return,
                Err(error) => panic!("client timeout is not server closure: {error}"),
            }
        }
    }).await.expect("deleted session SSE must end");
}

#[tokio::test]
async fn deleting_one_session_releases_only_its_handle_and_stream() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let first = Session::initialize(runtime.bound_port).await;
    let second = Session::initialize(runtime.bound_port).await;
    assert_ne!(first.id, second.id);
    assert_eq!(runtime.sessions.sessions.read().await.len(), 2);
    let mut first_stream = first.stream().await;
    let mut second_stream = second.stream().await;
    assert_stream_open(&mut first_stream).await;
    assert_stream_open(&mut second_stream).await;
    let response = timeout(DEADLINE, first.request(reqwest::Method::DELETE).send()).await.unwrap().unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    assert_stream_ended(first_stream).await;
    assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
    assert!(!runtime.cancellation.is_cancelled());
    assert!(runtime.is_running());
    second.ping().await;
    assert_stream_open(&mut second_stream).await;
    // A terminated session cannot be reused or transparently reconstructed.
    for method in [reqwest::Method::GET, reqwest::Method::POST] {
        let mut request = first.request(method.clone());
        if method == reqwest::Method::POST {
            request = request.json(&json!({"jsonrpc":"2.0", "id":3, "method":"ping"}));
        }
        assert_eq!(timeout(DEADLINE, request.send()).await.unwrap().unwrap().status(), reqwest::StatusCode::NOT_FOUND);
    }
    // Repeated DELETE is harmless and must not delete any other session.
    assert_eq!(timeout(DEADLINE, first.request(reqwest::Method::DELETE).send()).await.unwrap().unwrap().status(),
        reqwest::StatusCode::ACCEPTED);
    assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
    second.ping().await;
    let replacement = Session::initialize(runtime.bound_port).await;
    assert_ne!(replacement.id, first.id);
    assert_ne!(replacement.id, second.id);
    assert_eq!(runtime.sessions.sessions.read().await.len(), 2);
    replacement.ping().await;
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert_stream_ended(second_stream).await;
    assert!(runtime.sessions.sessions.read().await.is_empty());
}

#[tokio::test]
async fn unsupported_protocol_rejects_requests_without_poisoning_existing_session() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let session = Session::initialize(runtime.bound_port).await;
    for method in [reqwest::Method::GET, reqwest::Method::POST, reqwest::Method::DELETE] {
        let mut request = session.client.request(method.clone(), &session.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session.id)
            .header("mcp-protocol-version", "unsupported-test-version");
        if method == reqwest::Method::POST {
            request = request.json(&json!({"jsonrpc":"2.0", "id":3, "method":"ping"}));
        }
        assert_eq!(timeout(DEADLINE, request.send()).await.unwrap().unwrap().status(), reqwest::StatusCode::BAD_REQUEST);
        assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
        session.ping().await;
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn non_text_session_header_cannot_bypass_initialization_guard() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let response = client().post(format!("http://127.0.0.1:{}/mcp", runtime.bound_port))
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &b"\xff"[..])
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":"ping"})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(runtime.sessions.sessions.read().await.is_empty());
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn rejected_first_messages_do_not_disrupt_a_live_session() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let session = Session::initialize(runtime.bound_port).await;
    for body in [
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "{",
    ] {
        let response = client().post(&session.url)
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(body).send().await.unwrap();
        assert!(response.status().is_client_error());
        assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
        session.ping().await;
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn first_message_guard_preserves_http_negotiation_errors() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    for (accept, content_type, status) in [
        ("application/json", "application/json", reqwest::StatusCode::NOT_ACCEPTABLE),
        ("text/event-stream", "application/json", reqwest::StatusCode::NOT_ACCEPTABLE),
        ("application/json, text/event-stream", "text/plain", reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE),
    ] {
        let response = client().post(&url).header("accept", accept).header("content-type", content_type)
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).send().await.unwrap();
        assert_eq!(response.status(), status);
        assert!(runtime.sessions.sessions.read().await.is_empty());
    }
    let session = Session::initialize(runtime.bound_port).await;
    session.ping().await;
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_preserves_host_and_origin_rejections() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    for (name, value, status) in [
        ("host", "evil.example", reqwest::StatusCode::FORBIDDEN),
        ("host", "localhost:1", reqwest::StatusCode::FORBIDDEN),
        ("origin", "https://evil.example", reqwest::StatusCode::FORBIDDEN),
        ("origin", "null", reqwest::StatusCode::FORBIDDEN),
        ("origin", "not-an-origin", reqwest::StatusCode::BAD_REQUEST),
    ] {
        let response = client().post(&url)
            .header("accept", "application/json, text/event-stream")
            .header(name, value)
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"ping"}))
            .send().await.unwrap();
        assert_eq!(response.status(), status, "{name}: {value}");
        assert!(runtime.sessions.sessions.read().await.is_empty());
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_does_not_override_health_route() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let response = client().post(format!("http://127.0.0.1:{}/health", runtime.bound_port))
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":"ping"}))
        .send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()["allow"], "GET,HEAD");
    assert!(runtime.sessions.sessions.read().await.is_empty());
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_explains_how_to_initialize() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let response = client().post(format!("http://127.0.0.1:{}/mcp", runtime.bound_port))
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":"ping"}))
        .send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.text().await.unwrap(), "Unexpected message, expect initialize request");
    assert!(runtime.sessions.sessions.read().await.is_empty());
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_json_rpc_shapes_leave_no_sessions() {
    use rmcp::model::{ClientJsonRpcMessage, ClientRequest};
    let mut runtime = start_mcp_server(0).await.unwrap();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    for body in [
        "{", "null", "true", "1", "[]",
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
        r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"ping"} {}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"invalid request"}}"#,
        r#"{"jsonrpc":"2.0","id":"a","method":"custom/method","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"custom/notification","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    ] {
        // rmcp's actual reader/parser is the compatibility oracle, including
        // its custom request/notification fallbacks for incomplete messages.
        let parsed = serde_json::from_reader::<_, ClientJsonRpcMessage>(body.as_bytes());
        assert!(!matches!(&parsed, Ok(ClientJsonRpcMessage::Request(request))
            if matches!(request.request, ClientRequest::InitializeRequest(_))));
        let expected = if parsed.is_ok() { reqwest::StatusCode::UNPROCESSABLE_ENTITY }
            else { reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE };
        let response = client().post(&url)
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .body(body).send().await.unwrap();
        assert_eq!(response.status(), expected, "{body}");
        assert!(!response.headers().contains_key("mcp-session-id"));
        let text = response.text().await.unwrap();
        if parsed.is_ok() {
            assert_eq!(text, "Unexpected message, expect initialize request");
        } else {
            assert!(text.starts_with("fail to deserialize request body"), "{text}");
        }
        assert!(runtime.sessions.sessions.read().await.is_empty(), "{body}");
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_preserves_initialize_variants() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    let client = client();
    for (id, protocol, session_header) in [
        (json!(0), "2025-03-26", None),
        (json!(-1), "2025-06-18", None),
        (json!(""), "2025-11-25", None),
        (json!("init/string"), "unknown-negotiate-latest", None),
        (json!(i64::MAX), "2025-03-26", Some(&b"\xff"[..])),
    ] {
        let mut request = client.post(&url)
            .header("accept", "application/json, text/event-stream")
            // The transport ignores this header for initialization and
            // negotiates using the body. The guard must not add a new rule.
            .header("mcp-protocol-version", "unsupported-on-initialize")
            .json(&json!({"jsonrpc":"2.0", "id":id, "method":"initialize", "params":{
                "protocolVersion":protocol, "capabilities":{},
                "clientInfo":{"name":"admission-review", "version":"1"},
                "_meta":{"review":true}
            }}));
        if let Some(header) = session_header { request = request.header("mcp-session-id", header); }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{id}");
        let session_id = response.headers()["mcp-session-id"].to_str().unwrap().to_owned();
        let result = rpc(&response.text().await.unwrap());
        assert_eq!(result["id"], id);
        assert!(result["result"]["capabilities"].is_object());
        let session = Session { client: client.clone(), url: url.clone(), id: session_id };
        assert_eq!(session.post(json!({"jsonrpc":"2.0", "method":"notifications/initialized"})).await.status(),
            reqwest::StatusCode::ACCEPTED);
        session.ping().await;
        assert_eq!(session.request(reqwest::Method::DELETE).send().await.unwrap().status(),
            reqwest::StatusCode::ACCEPTED);
        assert!(runtime.sessions.sessions.read().await.is_empty());
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_preserves_session_routing_and_client_replies() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let session = Session::initialize(runtime.bound_port).await;
    for id in ["", "unknown-session"] {
        let response = client().post(&session.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", id)
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"ping"}))
            .send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        assert_eq!(response.text().await.unwrap(), "Not Found: Session not found");
        assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
    }
    for message in [
        json!({"jsonrpc":"2.0", "id":"unmatched", "result":{}}),
        json!({"jsonrpc":"2.0", "id":"unmatched", "error":{"code":-32600,"message":"test"}}),
        json!({"jsonrpc":"2.0", "method":"custom/notification", "params":{}}),
    ] {
        assert_eq!(session.post(message).await.status(), reqwest::StatusCode::ACCEPTED);
        assert_eq!(runtime.sessions.sessions.read().await.len(), 1);
        session.ping().await;
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_matches_transport_header_negotiation() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let url = format!("http://127.0.0.1:{}/mcp", runtime.bound_port);
    for (accepts, content_type, expected) in [
        (vec![], Some("application/json"), 406),
        (vec!["*/*"], Some("application/json"), 406),
        (vec!["APPLICATION/JSON, TEXT/EVENT-STREAM"], Some("application/json"), 406),
        (vec!["application/json", "text/event-stream"], Some("application/json"), 406),
        (vec!["application/json", "application/json, text/event-stream"], Some("application/json"), 406),
        (vec!["application/json, text/event-stream", "application/json"], Some("application/json"), 422),
        (vec!["application/json, text/event-stream"], None, 415),
        (vec!["application/json, text/event-stream"], Some("Application/JSON"), 415),
        (vec!["application/json, text/event-stream"], Some("application/json; charset=utf-8"), 422),
        // rmcp 1.7 uses substring/prefix checks, not a complete MIME parser.
        // Preserve its behavior; do not silently impose stricter negotiation.
        (vec!["application/json;q=0, text/event-stream;q=0"], Some("application/json"), 422),
        (vec!["application/json, text/event-stream"], Some("application/json-other"), 422),
    ] {
        let mut request = client().post(&url).body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        for accept in &accepts { request = request.header("accept", *accept); }
        if let Some(content_type) = content_type { request = request.header("content-type", content_type); }
        let response = request.send().await.unwrap();
        assert_eq!(response.status().as_u16(), expected, "{accepts:?} {content_type:?}");
        assert!(runtime.sessions.sessions.read().await.is_empty());
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn review_admission_covers_equivalent_loopback_authorities_and_fallback_paths() {
    let mut runtime = start_mcp_server(0).await.unwrap();
    let port = runtime.bound_port;
    for (host, origin, path) in [
        (format!("LOCALHOST:{port}"), Some(format!("HTTP://LOCALHOST:{port}")), "/mcp"),
        (format!("localhost:0{port}"), Some(format!("  http://localhost:{port}  ")), "/"),
        (format!("127.0.0.1:{port}"), None, "/another-fallback-path"),
        // rmcp's origin matcher ignores path/query. Mirror its existing
        // decision so these requests cannot bypass the no-allocation guard.
        (format!("localhost:{port}"), Some(format!("http://localhost:{port}/path?query")), "/mcp"),
    ] {
        let mut request = client().post(format!("http://127.0.0.1:{port}{path}"))
            .header("host", &host)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"ping"}));
        if let Some(origin) = &origin { request = request.header("origin", origin); }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY, "{host} {origin:?}");
        assert!(runtime.sessions.sessions.read().await.is_empty(), "{host} {origin:?}");
    }
    timeout(DEADLINE, runtime.stop()).await.unwrap().unwrap();
}
