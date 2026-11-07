//! Actual listener-task state checks. No App, account or model is invoked.
use super::*;
use super::super::status::McpState;

#[tokio::test]
async fn status_reports_actual_ephemeral_port_and_cancelled_task() {
    // Current-thread scheduling keeps the listener task unpolled until await.
    let mut runtime = start_mcp_server(0).await.unwrap();
    let (state, port) = runtime.status();
    assert_eq!(state, McpState::Running);
    assert!(port.is_some_and(|port| port > 0));
    assert!(runtime.is_running());
    runtime.cancellation.cancel();
    assert_eq!(runtime.status(), (McpState::Stopping, None));
    assert!(!runtime.is_running());
    runtime.stop().await.unwrap();
    assert_eq!(runtime.status(), (McpState::Exited, None));
}

#[tokio::test]
async fn status_does_not_claim_aborted_listener_is_running() {
    let mut runtime = start_headless_mcp_server(0).await.unwrap();
    runtime.root.as_ref().unwrap().abort();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.root.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
    }).await.expect("aborted listener must finish");
    assert_eq!(runtime.status(), (McpState::Exited, None));
    assert!(!runtime.is_running());
    runtime.stop().await.unwrap();
}
