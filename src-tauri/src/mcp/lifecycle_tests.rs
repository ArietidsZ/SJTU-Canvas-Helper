//! Runtime-boundary regressions. No App, account, Canvas, or AI tool is used.
//! Artificially held locks model uncooperative handlers/session cleanup; they
//! are not evidence that a real application operation was started or stopped.
use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{sleep, Instant},
};

const TEST_DEADLINE: Duration = Duration::from_secs(2);

// start_mcp_server intentionally returns no socket address. Reserve an ephemeral
// port, then let the actual production start function own it. Retry only the
// benign race where another process binds between releasing and starting.
async fn start() -> (McpRuntime, u16) {
    for _ in 0..8 {
        let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        match start_mcp_server(port).await {
            Ok(runtime) => return (runtime, port),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            Err(error) => panic!("production runtime start failed: {error}"),
        }
    }
    panic!("could not reserve an ephemeral loopback port");
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(TEST_DEADLINE)
        .build()
        .unwrap()
}

async fn assert_healthy(port: u16) {
    let response = client()
        .get(format!("http://127.0.0.1:{port}/health"))
        .send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), r#"{"status":"ok"}"#);
}

async fn wait_for_rebind(port: u16) {
    timeout(TEST_DEADLINE, async {
        loop {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => { drop(listener); return; }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    sleep(Duration::from_millis(5)).await;
                }
                Err(error) => panic!("rebind failed: {error}"),
            }
        }
    }).await.expect("runtime must release its listener");
}

#[tokio::test]
async fn start_stop_is_idempotent_and_releases_port_for_restart() {
    let (mut runtime, port) = start().await;
    assert!(runtime.is_running());
    assert_healthy(port).await;
    timeout(TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(!runtime.is_running());
    assert!(runtime.cancellation.is_cancelled());
    assert!(runtime.root.is_none());
    timeout(TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
    let mut replacement = start_mcp_server(port).await.unwrap();
    assert!(replacement.is_running());
    assert_healthy(port).await;
    timeout(TEST_DEADLINE, replacement.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn occupied_port_returns_bind_error_without_disrupting_live_runtime() {
    let (mut runtime, port) = start().await;
    let error = match start_mcp_server(port).await {
        Ok(_) => panic!("duplicate bind unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    assert!(runtime.is_running());
    assert_healthy(port).await;
    timeout(TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
}

#[tokio::test]
async fn drop_cancels_runtime_and_releases_active_listener() {
    let (runtime, port) = start().await;
    assert_healthy(port).await;
    let cancellation = runtime.cancellation.clone();
    drop(runtime);
    assert!(cancellation.is_cancelled());
    wait_for_rebind(port).await;
}

#[tokio::test]
async fn drop_before_listener_first_poll_releases_socket() {
    let (runtime, port) = start().await;
    // No await occurs between spawn in start and drop on this current-thread
    // executor, exercising cancellation before the listener future is polled.
    let cancellation = runtime.cancellation.clone();
    drop(runtime);
    assert!(cancellation.is_cancelled());
    wait_for_rebind(port).await;
}

#[tokio::test]
async fn listener_abort_before_first_poll_cancels_shared_token() {
    let (mut runtime, port) = start().await;
    let cancellation = runtime.cancellation.clone();
    runtime.root.as_ref().unwrap().abort();
    // Drop of the captured guard must work even if its task was never polled.
    timeout(TEST_DEADLINE, cancellation.cancelled()).await.unwrap();
    assert!(!runtime.is_running());
    timeout(TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(runtime.root.is_none());
    wait_for_rebind(port).await;
}

#[tokio::test]
async fn stop_waits_for_handler_quiescence_before_success() {
    let (mut runtime, _port) = start().await;
    let handlers = runtime.handlers.clone();
    let active_handler = handlers.read().await;
    let cancellation = runtime.cancellation.clone();
    let stop = runtime.stop();
    tokio::pin!(stop);
    assert!(timeout(Duration::from_millis(50), &mut stop).await.is_err());
    assert!(cancellation.is_cancelled());
    drop(active_handler);
    timeout(TEST_DEADLINE, &mut stop).await.unwrap().unwrap();
}

#[tokio::test]
async fn handler_timeout_retains_stopping_runtime_until_retry_can_drain() {
    let (mut runtime, port) = start().await;
    let handlers = runtime.handlers.clone();
    let sessions = runtime.sessions.clone();
    let root_id = runtime.root.as_ref().unwrap().id();
    let active_handler = handlers.read().await;
    let started = Instant::now();
    let error = timeout(STOP_TIMEOUT + TEST_DEADLINE, runtime.stop())
        .await.unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("configuration unchanged"));
    assert!(started.elapsed() >= STOP_TIMEOUT);
    assert!(runtime.cancellation.is_cancelled());
    assert!(!runtime.is_running());
    assert_eq!(runtime.root.as_ref().unwrap().id(), root_id);
    assert!(Arc::ptr_eq(&runtime.handlers, &handlers));
    assert!(Arc::ptr_eq(&runtime.sessions, &sessions));
    assert!(runtime.handlers.try_write().is_err());
    // This verifies McpRuntime retains its state. The App-managed slot and
    // account-generation protection need separate real-App tests.
    drop(active_handler);
    timeout(TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(runtime.root.is_none());
    wait_for_rebind(port).await;
}

#[tokio::test]
async fn stalled_session_cleanup_is_bounded_after_handlers_are_idle() {
    let (mut runtime, port) = start().await;
    let sessions = runtime.sessions.clone();
    let blocked_cleanup = sessions.sessions.write().await;
    let started = Instant::now();
    timeout(STOP_TIMEOUT + TEST_DEADLINE, runtime.stop())
        .await.expect("session cleanup must remain bounded").unwrap();
    assert!(started.elapsed() >= STOP_TIMEOUT);
    assert!(!runtime.is_running());
    assert!(runtime.root.is_none());
    drop(blocked_cleanup);
    wait_for_rebind(port).await;
}

#[tokio::test]
async fn stop_cancels_incomplete_http_body_before_body_deadline() {
    let (mut runtime, port) = start().await;
    assert_healthy(port).await;
    let mut connection = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    connection.write_all(format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: 100\r\n\r\n{{",
    ).as_bytes()).await.unwrap();
    // Give the socket read a chance to enter the body middleware. The separate
    // HTTP boundary suite tests exact 503 admission on a still-live listener.
    sleep(Duration::from_millis(30)).await;
    timeout(TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
    let mut response = Vec::new();
    match timeout(TEST_DEADLINE, connection.read_to_end(&mut response)).await.unwrap() {
        Ok(_) => assert!(response.is_empty() || response.starts_with(b"HTTP/1.1 503 "),
            "unexpected response: {}", String::from_utf8_lossy(&response)),
        // Abrupt closure is allowed during listener shutdown; the key boundary
        // is that the real stop completes without waiting for the 10s body timer.
        Err(error) => assert!(matches!(error.kind(), io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted | io::ErrorKind::UnexpectedEof)),
    }
    assert!(runtime.root.is_none());
    wait_for_rebind(port).await;
}

#[tokio::test]
async fn listener_not_cooperating_with_shutdown_is_aborted_within_bound() {
    let (mut runtime, port) = start().await;
    // Stop the actual listener, then substitute only the listener-task handle
    // to exercise the production timeout/abort fallback deterministically.
    runtime.root.as_ref().unwrap().abort();
    timeout(TEST_DEADLINE, runtime.cancellation.cancelled()).await.unwrap();
    runtime.root = Some(tokio::spawn(std::future::pending::<io::Result<()>>()));
    let started = Instant::now();
    timeout(STOP_TIMEOUT + TEST_DEADLINE, runtime.stop()).await.unwrap().unwrap();
    assert!(started.elapsed() >= STOP_TIMEOUT);
    assert!(runtime.root.is_none());
    assert!(!runtime.is_running());
    wait_for_rebind(port).await;
}
