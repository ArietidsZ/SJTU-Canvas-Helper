//! Headless process lifetime. Only the existing App owns the MCP listener;
//! this module never creates a parallel manager or bypasses its owner lock.
use std::{future::Future, io, time::Duration};

use crate::{app::App, error::Result, APP};
use super::cli::HeadlessOptions;

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(25);
const RUNTIME_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn run(options: HeadlessOptions) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        // Install every required signal stream before APP can recover/create
        // configuration or bind a listener. Setup failure owns no app resources.
        let shutdown = ShutdownSignals::install()?;
        let log_directory = App::config_dir()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("SJTU-Canvas-Helper"));
        crate::diagnostics::init(log_directory);
        // Do not call App::init(): it can contact the video login service.
        run_with_shutdown(&APP, options, shutdown.wait()).await
    });
    // Tokio's normal drop can wait forever for blocking tasks. Bound teardown
    // separately after the managed MCP/proxy cleanup attempt, even on failure.
    runtime.shutdown_timeout(RUNTIME_TIMEOUT);
    result
}

pub(crate) async fn run_with_shutdown(
    app: &App,
    options: HeadlessOptions,
    shutdown: impl Future<Output = io::Result<()>>,
) -> Result<()> {
    // A rejected configuration does not grant ownership of an existing runtime.
    // Only clean up resources after this invocation has installed its policy.
    let port = app.configure_headless_mcp(options).await?;
    let outcome: Result<()> = async {
        app.start_mcp().await?;
        tracing::info!(port, "Headless MCP ready on 127.0.0.1; AI and token-probe tools disabled");
        eprintln!("MCP ready: http://127.0.0.1:{port}/mcp (Streamable HTTP). Headless AI tools are disabled.");
        tokio::select! {
            biased;
            result = shutdown => result.map_err(Into::into),
            result = wait_for_listener_exit(app) => result,
        }
    }.await;
    finish(outcome, async {
        // Both cleanup paths start even if MCP shutdown fails or waits on an
        // owner lock. One deadline bounds their entire combined operation.
        let (mcp, ()) = tokio::join!(app.stop_mcp(), app.stop_proxy());
        mcp
    }, CLEANUP_TIMEOUT).await
}

async fn wait_for_listener_exit(app: &App) -> Result<()> {
    loop {
        if !app.mcp_is_running().await {
            return Err(io::Error::other("Headless MCP listener exited unexpectedly").into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn finish(
    outcome: Result<()>,
    cleanup: impl Future<Output = Result<()>>,
    deadline: Duration,
) -> Result<()> {
    let cleanup = tokio::time::timeout(deadline, cleanup).await
        .unwrap_or_else(|_| Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Headless MCP/proxy cleanup exceeded its deadline; termination is unconfirmed",
        ).into()));
    match (outcome, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(run), Err(stop)) => Err(io::Error::other(format!(
            "Headless MCP failed ({run}); cleanup also failed ({stop})"
        )).into()),
    }
}

struct ShutdownSignals {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
    #[cfg(windows)]
    terminate: tokio::signal::windows::CtrlBreak,
}

impl ShutdownSignals {
    fn install() -> io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Ok(Self {
                interrupt: signal(SignalKind::interrupt())?,
                terminate: signal(SignalKind::terminate())?,
            })
        }
        #[cfg(windows)]
        {
            Ok(Self {
                interrupt: tokio::signal::windows::ctrl_c()?,
                terminate: tokio::signal::windows::ctrl_break()?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        Err(io::Error::new(io::ErrorKind::Unsupported, "Headless MCP signals are unsupported on this platform"))
    }

    async fn wait(mut self) -> io::Result<()> {
        #[cfg(any(unix, windows))]
        {
            let signal = tokio::select! {
                signal = self.interrupt.recv() => signal,
                signal = self.terminate.recv() => signal,
            };
            signal.ok_or_else(|| io::Error::other("Headless MCP signal stream closed"))
        }
        #[cfg(not(any(unix, windows)))]
        Err(io::Error::new(io::ErrorKind::Unsupported, "Headless MCP signals are unsupported on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn shutdown_success_is_only_success_after_cleanup() {
        let cleaned = AtomicBool::new(false);
        finish(Ok(()), async { cleaned.store(true, Ordering::SeqCst); Ok(()) }, Duration::from_secs(1)).await.unwrap();
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn startup_or_signal_failure_still_runs_cleanup() {
        let cleaned = AtomicBool::new(false);
        let result = finish(Err(io::Error::other("signal wait failed").into()), async {
            cleaned.store(true, Ordering::SeqCst);
            Ok(())
        }, Duration::from_secs(1)).await;
        assert!(result.unwrap_err().to_string().contains("signal wait failed"));
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cleanup_failure_is_not_reported_as_success() {
        assert!(finish(Ok(()), async { Err(io::Error::other("stop failed").into()) }, Duration::from_secs(1))
            .await.unwrap_err().to_string().contains("stop failed"));
    }

    #[tokio::test]
    async fn primary_and_cleanup_errors_are_both_retained() {
        let error = finish(Err(io::Error::other("bind failed").into()), async {
            Err(io::Error::other("stop failed").into())
        }, Duration::from_secs(1)).await.unwrap_err().to_string();
        assert!(error.contains("bind failed") && error.contains("stop failed"));
    }

    #[tokio::test]
    async fn pending_cleanup_has_a_deadline_and_does_not_claim_termination() {
        let error = finish(Ok(()), std::future::pending(), Duration::from_millis(10))
            .await.unwrap_err().to_string();
        assert!(error.contains("deadline") && error.contains("unconfirmed"));
    }
}

#[cfg(all(test, unix))]
#[path = "headless_signal_tests.rs"]
mod signal_tests;
