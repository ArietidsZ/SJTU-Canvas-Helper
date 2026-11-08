//! Own the listener and the application handler lifetime independently of rmcp's
//! detached session tasks. Stopping does not revoke already-sent remote work.
use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use axum::{middleware, response::Json, Router};
use rmcp::{
    model::{ClientNotification, ClientRequest, ServerInfo, ServerResult},
    service::{NotificationContext, RequestContext},
    transport::streamable_http_server::{
        session::{local::LocalSessionManager, SessionManager},
        StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, Service,
};
use tokio::{sync::RwLock, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;

use super::{http_boundary, McpServer};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);

struct CancellableServer {
    inner: McpServer,
    cancellation: CancellationToken,
    handlers: Arc<RwLock<()>>,
}

fn stopped() -> ErrorData {
    ErrorData::internal_error("MCP server is stopping", None)
}

impl Service<RoleServer> for CancellableServer {
    async fn handle_request(
        &self,
        request: ClientRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, ErrorData> {
        let _guard = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return Err(stopped()),
            guard = self.handlers.read() => guard,
        };
        // The delegated future is dropped before the outer read guard. Do not
        // move application work into another untracked spawned task here.
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(stopped()),
            result = async { <McpServer as Service<RoleServer>>::handle_request(&self.inner, request, context).await } => result,
        }
    }

    async fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let _guard = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return Err(stopped()),
            guard = self.handlers.read() => guard,
        };
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(stopped()),
            result = async { <McpServer as Service<RoleServer>>::handle_notification(&self.inner, notification, context).await } => result,
        }
    }

    fn get_info(&self) -> ServerInfo {
        <McpServer as Service<RoleServer>>::get_info(&self.inner)
    }
}

pub(crate) struct McpRuntime {
    bound_port: u16,
    cancellation: CancellationToken,
    handlers: Arc<RwLock<()>>,
    sessions: Arc<LocalSessionManager>,
    root: Option<JoinHandle<io::Result<()>>>,
}

impl McpRuntime {
    /// A task-lifetime observation, not a health probe or authentication check.
    pub(crate) fn status(&self) -> (super::status::McpState, Option<u16>) {
        use super::status::McpState;
        if !self.root.as_ref().is_some_and(|root| !root.is_finished()) {
            (McpState::Exited, None)
        } else if self.cancellation.is_cancelled() {
            (McpState::Stopping, None)
        } else {
            (McpState::Running, Some(self.bound_port))
        }
    }

    pub(crate) fn is_running(&self) -> bool {
        !self.cancellation.is_cancelled() && self.root.as_ref().is_some_and(|root| !root.is_finished())
    }

    pub(crate) async fn stop(&mut self) -> io::Result<()> {
        self.cancellation.cancel();
        // A timeout must retain this stopping runtime in its managed slot. A
        // finished listener alone does not authorize a new account generation.
        let idle = timeout(STOP_TIMEOUT, self.handlers.write()).await.map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "MCP application handlers did not stop; configuration unchanged")
        })?;
        drop(idle);

        // close_session only enqueues a close event, not a worker join. Bound
        // this best-effort cleanup separately from application quiescence.
        if timeout(STOP_TIMEOUT, async {
            let ids: Vec<_> = self.sessions.sessions.read().await.keys().cloned().collect();
            for id in ids {
                if let Err(error) = self.sessions.close_session(&id).await {
                    tracing::warn!(%error, "MCP session close request failed");
                }
            }
        }).await.is_err() {
            tracing::warn!("MCP session close requests exceeded their deadline");
        }

        let Some(root) = self.root.as_mut() else { return Ok(()); };
        let result = match timeout(STOP_TIMEOUT, &mut *root).await {
            Ok(result) => result,
            Err(_) => {
                root.abort();
                timeout(STOP_TIMEOUT, &mut *root).await.map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "MCP listener termination is unconfirmed; configuration unchanged")
                })?
            }
        };
        self.root = None;
        match result {
            Ok(Err(error)) => tracing::warn!(%error, "MCP listener exited with an error"),
            Err(error) if !error.is_cancelled() => tracing::warn!(%error, "MCP listener task failed"),
            _ => {}
        }
        Ok(())
    }
}

impl Drop for McpRuntime {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(root) = &self.root { root.abort(); }
    }
}

pub(crate) async fn start_mcp_server(port: u16) -> io::Result<McpRuntime> {
    start_with_policy(port, false).await
}

pub(crate) async fn start_headless_mcp_server(port: u16) -> io::Result<McpRuntime> {
    start_with_policy(port, true).await
}

async fn start_with_policy(port: u16, headless: bool) -> io::Result<McpRuntime> {
    // Bind before publishing a running instance; bind errors are synchronous
    // results, not panics hidden inside an already-stored JoinHandle.
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    let bound_port = listener.local_addr()?.port();
    let cancellation = CancellationToken::new();
    let handlers = Arc::new(RwLock::new(()));
    let sessions = Arc::new(LocalSessionManager::default());
    let router = mcp_router_with_policy(bound_port, &cancellation, &handlers, &sessions, headless);
    let shutdown = cancellation.clone();
    // Capture the guard before spawning, including abort-before-first-poll.
    let guard = cancellation.clone().drop_guard();
    let root = tokio::spawn(async move {
        let _guard = guard;
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await
    });
    Ok(McpRuntime { bound_port, cancellation, handlers, sessions, root: Some(root) })
}

// Shared by the listener and local HTTP boundary tests. No application work is
// performed until the existing CancellableServer admits a protocol request.
#[cfg(test)]
pub(super) fn mcp_router(
    port: u16,
    cancellation: &CancellationToken,
    handlers: &Arc<RwLock<()>>,
    sessions: &Arc<LocalSessionManager>,
) -> Router {
    mcp_router_with_policy(port, cancellation, handlers, sessions, false)
}

fn mcp_router_with_policy(
    port: u16,
    cancellation: &CancellationToken,
    handlers: &Arc<RwLock<()>>,
    sessions: &Arc<LocalSessionManager>,
    headless: bool,
) -> Router {
    let factory_token = cancellation.clone();
    let factory_handlers = handlers.clone();
    let service = StreamableHttpService::new(
        move || {
            if factory_token.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "MCP server is stopping"));
            }
            Ok(CancellableServer {
                inner: McpServer { headless },
                cancellation: factory_token.clone(),
                handlers: factory_handlers.clone(),
            })
        },
        sessions.clone(),
        StreamableHttpServerConfig::default()
            .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
            .with_allowed_origins([
                format!("http://127.0.0.1:{port}"),
                format!("http://localhost:{port}"),
            ])
            .with_sse_keep_alive(Some(Duration::from_secs(15)))
            .with_stateful_mode(true)
            .with_cancellation_token(cancellation.child_token()),
    );
    Router::new()
        .route("/health", axum::routing::get(|| async { Json(serde_json::json!({"status": "ok"})) }))
        .fallback_service(service)
        .layer(middleware::from_fn_with_state(port, http_boundary::limit_body))
        // Last layer is outermost: cancellation also interrupts body buffering.
        .layer(middleware::from_fn_with_state(
            cancellation.clone(),
            http_boundary::reject_cancelled,
        ))
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "status_runtime_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "session_lifecycle_tests.rs"]
mod session_lifecycle_tests;

#[cfg(test)]
#[path = "session_admission_tests.rs"]
mod session_admission_tests;

#[cfg(test)]
#[path = "session_interruption_tests.rs"]
mod session_interruption_tests;
