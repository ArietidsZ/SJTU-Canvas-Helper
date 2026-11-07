//! Account-isolated tests: all configuration is under a fresh temporary root,
//! listeners are loopback only, and no Canvas/LLM tool is invoked.
use super::*;
use crate::mcp::{cli::HeadlessOptions, headless::run_with_shutdown};
use tokio::net::TcpListener;

fn app(root: &Path, port: u16) -> App {
    let store = ConfigStore::open(root).unwrap();
    let directory = root.to_str().unwrap();
    let (info, mut config) = App::load_startup_configuration(directory).unwrap();
    config.mcp_port = port;
    fs::write(App::get_config_path_in(directory, &info.current_account), serde_json::to_vec(&config).unwrap()).unwrap();
    App {
        config_store: Some(store),
        client: Arc::new(Client::new_without_proxy("http://127.0.0.1", "", "", "", None)),
        canvas_session: Default::default(),
        login_attempt: Default::default(),
        current_account: RwLock::new(info.current_account),
        config: RwLock::new(config),
        config_owner: tokio::sync::Mutex::new(Some("headless-test-owner".into())),
        handle: Default::default(),
        mcp_handle: Default::default(),
        mcp_headless: Default::default(),
        cache: Default::default(),
    }
}

async fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

#[tokio::test]
async fn headless_options_do_not_mutate_config_or_owner_and_can_only_be_set_once() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), 3100);
    let path = App::get_config_path_in(root.path().to_str().unwrap(), &Account::Default);
    let before = fs::read(&path).unwrap();
    assert_eq!(app.configure_headless_mcp(HeadlessOptions { port: Some(3210) }).await.unwrap(), 3210);
    assert_eq!(app.config.read().await.mcp_port, 3100);
    assert!(!app.config.read().await.mcp_enabled);
    assert_eq!(app.config_owner.lock().await.as_deref(), Some("headless-test-owner"));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(app.configure_headless_mcp(HeadlessOptions::default()).await.is_err());
}

#[tokio::test]
async fn headless_requires_valid_owner_and_store() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), 3100);
    *app.config_owner.lock().await = None;
    assert!(matches!(app.configure_headless_mcp(HeadlessOptions::default()).await, Err(AppError::StaleConfigOwner)));
    assert!(matches!(app.start_mcp().await, Err(AppError::StaleConfigOwner)));
    assert!(app.mcp_headless.get().is_none());
    *app.config_owner.lock().await = Some("headless-test-owner".into());
    app.config_store.as_ref().unwrap().poison();
    assert!(matches!(app.configure_headless_mcp(HeadlessOptions::default()).await, Err(AppError::PersistenceUncertain)));
    assert!(app.mcp_headless.get().is_none());
    assert!(!app.mcp_is_running().await);
}

#[tokio::test]
async fn zero_saved_port_fails_closed_but_explicit_valid_override_is_allowed() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), 0);
    assert!(app.configure_headless_mcp(HeadlessOptions::default()).await.is_err());
    assert!(app.mcp_headless.get().is_none());
    assert_eq!(app.configure_headless_mcp(HeadlessOptions { port: Some(3210) }).await.unwrap(), 3210);
    assert_eq!(app.config.read().await.mcp_port, 0);
}

#[tokio::test]
async fn headless_policy_cannot_be_applied_to_a_running_gui_listener() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), unused_port().await);
    app.start_mcp().await.unwrap();
    assert!(app.configure_headless_mcp(HeadlessOptions::default()).await.is_err());
    assert!(app.mcp_headless.get().is_none());
    assert!(app.mcp_is_running().await);
    *app.handle.write().await = Some(tokio::spawn(std::future::pending()));
    assert!(run_with_shutdown(&app, HeadlessOptions::default(), async { Ok(()) }).await.is_err());
    assert!(app.mcp_is_running().await, "rejected invocation must not stop an existing listener");
    assert!(app.handle.read().await.is_some(), "rejected invocation must not stop an existing proxy");
    app.stop_mcp().await.unwrap();
    app.stop_proxy().await;
}

#[tokio::test]
async fn headless_shutdown_success_and_signal_failure_both_release_listener_and_proxy() {
    for fail in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let port = unused_port().await;
        let app = app(root.path(), port);
        *app.handle.write().await = Some(tokio::spawn(std::future::pending()));
        let result = run_with_shutdown(&app, HeadlessOptions::default(), async {
            assert!(app.mcp_is_running().await);
            if fail { Err(std::io::Error::other("signal stream failed")) } else { Ok(()) }
        }).await;
        assert_eq!(result.is_err(), fail);
        assert!(app.mcp_handle.lock().await.is_none());
        assert!(app.handle.read().await.is_none());
        assert!(TcpListener::bind(("127.0.0.1", port)).await.is_ok());
    }
}

#[tokio::test]
async fn headless_bind_failure_still_cleans_proxy_and_does_not_publish_a_listener() {
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), occupied.local_addr().unwrap().port());
    *app.handle.write().await = Some(tokio::spawn(std::future::pending()));
    assert!(run_with_shutdown(&app, HeadlessOptions::default(), std::future::pending()).await.is_err());
    assert!(app.mcp_handle.lock().await.is_none());
    assert!(app.handle.read().await.is_none());
}

#[tokio::test]
async fn headless_does_not_wait_forever_after_listener_exit() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), unused_port().await);
    let result = tokio::time::timeout(Duration::from_secs(2), run_with_shutdown(
        &app, HeadlessOptions::default(), async {
            app.stop_mcp().await.unwrap();
            std::future::pending().await
        },
    )).await.unwrap().unwrap_err();
    assert!(result.to_string().contains("listener exited unexpectedly"));
    assert!(app.mcp_handle.lock().await.is_none());
}

#[tokio::test]
async fn headless_override_survives_transactional_config_restart_without_being_persisted() {
    let root = tempfile::tempdir().unwrap();
    // Occupy the saved port: the headless override must win both at startup
    // and after a real owner-checked save/restart, without rewriting that port.
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let saved_port = occupied.local_addr().unwrap().port();
    let override_port = unused_port().await;
    let app = app(root.path(), saved_port);
    app.configure_headless_mcp(HeadlessOptions { port: Some(override_port) }).await.unwrap();
    app.start_mcp().await.unwrap();
    let config = app.config.read().await.clone();
    app.save_config(config, "headless-test-owner").await.unwrap();
    assert!(app.mcp_is_running().await);
    assert!(TcpListener::bind(("127.0.0.1", override_port)).await.is_err());
    let saved = App::read_config_from_file(&App::get_config_path_in(root.path().to_str().unwrap(), &Account::Default)).unwrap();
    assert_eq!(saved.mcp_port, saved_port);
    assert!(!saved.mcp_enabled);
    assert_eq!(app.mcp_headless.get(), Some(&HeadlessOptions { port: Some(override_port) }));
    app.stop_mcp().await.unwrap();
    assert!(TcpListener::bind(("127.0.0.1", override_port)).await.is_ok());
}

#[tokio::test]
async fn status_does_not_infer_liveness_from_saved_enabled_setting() {
    use crate::mcp::status::McpState;
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), 3100);
    app.config.write().await.mcp_enabled = true;
    let status = app.get_mcp_status("headless-test-owner").await.unwrap();
    assert_eq!(status.state, McpState::Stopped);
    assert!(status.configured_enabled);
    assert_eq!(status.configured_port, 3100);
    assert_eq!(status.listening_port, None);
    assert!(!status.headless);
    assert!(!app.mcp_is_running().await);
}

#[tokio::test]
async fn status_rejects_stale_missing_and_poisoned_owners() {
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), 3100);
    assert!(matches!(app.get_mcp_status("old-owner").await, Err(AppError::StaleConfigOwner)));
    *app.config_owner.lock().await = None;
    assert!(matches!(app.get_mcp_status("headless-test-owner").await, Err(AppError::StaleConfigOwner)));
    *app.config_owner.lock().await = Some("headless-test-owner".into());
    app.config_store.as_ref().unwrap().poison();
    assert!(matches!(app.get_mcp_status("headless-test-owner").await, Err(AppError::PersistenceUncertain)));
}

#[tokio::test]
async fn status_separates_headless_override_from_saved_port() {
    use crate::mcp::status::McpState;
    let root = tempfile::tempdir().unwrap();
    let app = app(root.path(), 3100);
    let port = unused_port().await;
    app.configure_headless_mcp(HeadlessOptions { port: Some(port) }).await.unwrap();
    app.start_mcp().await.unwrap();
    let status = app.get_mcp_status("headless-test-owner").await.unwrap();
    assert_eq!(status.state, McpState::Running);
    assert!(!status.configured_enabled);
    assert_eq!(status.configured_port, 3100);
    assert_eq!(status.listening_port, Some(port));
    assert!(status.headless);
    app.stop_mcp().await.unwrap();
    assert_eq!(app.get_mcp_status("headless-test-owner").await.unwrap().state, McpState::Stopped);
}
