//! Real Unix signal regressions, isolated from the test runner in child processes.
//! Exercises production signal streams, finish(), and listener shutdown. No App,
//! account, diagnostics, Canvas request, or GUI initialization is performed.
use super::*;
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};

const CHILD_ENV: &str = "CANVAS_MCP_SIGNAL_TEST_DIRECTORY";
const MODE_ENV: &str = "CANVAS_MCP_SIGNAL_TEST_QUEUE_BEFORE_WAIT";
const DEADLINE: Duration = Duration::from_secs(10);

// The child is never left behind on a failed assertion or timeout. Only this
// test's own child PID is signalled; no process-name or process-group targeting.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_file(path: &Path, child: &mut Child) -> String {
    let started = Instant::now();
    loop {
        if let Ok(value) = fs::read_to_string(path) {
            if !value.is_empty() {
                return value;
            }
        }
        assert!(child.try_wait().unwrap().is_none(), "signal child exited before readiness");
        assert!(started.elapsed() < DEADLINE, "signal child did not become ready");
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_health(port: u16) {
    let mut connection = TcpStream::connect(("127.0.0.1", port)).unwrap();
    connection.set_read_timeout(Some(DEADLINE)).unwrap();
    connection.set_write_timeout(Some(DEADLINE)).unwrap();
    connection.write_all(format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
    ).as_bytes()).unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 "));
    assert!(response.ends_with(r#"{"status":"ok"}"#));
}

fn assert_signal_shutdown(signal: &str, queue_before_wait: bool) {
    let directory = tempfile::tempdir().unwrap();
    let mut child = ChildGuard(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "mcp::headless::signal_tests::unix_signal_child", "--ignored", "--nocapture"])
        .env(CHILD_ENV, directory.path())
        .env(MODE_ENV, if queue_before_wait { "1" } else { "0" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn().unwrap());
    let port = wait_for_file(&directory.path().join("ready"), &mut child.0)
        .parse::<u16>().unwrap();
    assert_health(port);
    assert!(child.0.try_wait().unwrap().is_none());

    let signal_sent = Command::new("/bin/kill")
        .args([signal, &child.0.id().to_string()])
        .status().expect("Unix signal tests require /bin/kill");
    assert!(signal_sent.success());
    if queue_before_wait {
        // Release the child only after the OS accepted the signal. Its receiver
        // exists, but ShutdownSignals::wait has not been called yet.
        fs::write(directory.path().join("begin-wait"), b"go").unwrap();
    }

    assert_eq!(wait_for_file(&directory.path().join("cleaned"), &mut child.0), "ok");
    assert!(child.0.try_wait().unwrap().is_none());
    // Reclaim the port while the child and its Tokio runtime are still alive.
    // Process exit alone must not masquerade as successful listener cleanup.
    let rebound = TcpListener::bind(("127.0.0.1", port)).expect("signal shutdown must release the listener before exit");
    drop(rebound);
    fs::write(directory.path().join("exit"), b"go").unwrap();

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() { break status; }
        assert!(started.elapsed() < DEADLINE, "signal child did not exit after {signal}");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "{signal} must complete graceful cleanup, not terminate the child: {status}");
}

#[test]
fn sigint_reaches_cleanup_and_releases_the_listener() {
    assert_signal_shutdown("-INT", false);
}

#[test]
fn sigterm_reaches_cleanup_and_releases_the_listener() {
    assert_signal_shutdown("-TERM", false);
}

#[test]
fn signal_sent_after_install_before_wait_is_not_lost() {
    assert_signal_shutdown("-TERM", true);
}

#[test]
#[ignore = "internal helper; run only as a child of the Unix signal tests"]
fn unix_signal_child() {
    let Some(directory) = std::env::var_os(CHILD_ENV).map(std::path::PathBuf::from) else { return; };
    let queue_before_wait = std::env::var(MODE_ENV).unwrap() == "1";
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build().unwrap();
    runtime.block_on(async {
        let signals = ShutdownSignals::install().unwrap();
        let mut listener = super::super::lifecycle::start_headless_mcp_server(0).await.unwrap();
        let (_, port) = listener.status();
        // Use rename so a reader cannot observe an incomplete port number.
        fs::write(directory.join("ready.pending"), port.unwrap().to_string()).unwrap();
        fs::rename(directory.join("ready.pending"), directory.join("ready")).unwrap();
        if queue_before_wait {
            // Block only block_on's calling thread. The single worker remains
            // available to service the listener and installed OS signal driver.
            let started = Instant::now();
            while !directory.join("begin-wait").exists() {
                assert!(started.elapsed() < DEADLINE, "parent never released the signal waiter");
                thread::sleep(Duration::from_millis(10));
            }
        }
        let signal = tokio::time::timeout(DEADLINE, signals.wait()).await
            .expect("installed signal stream must receive the parent's signal");
        finish(signal.map_err(Into::into), async {
            listener.stop().await.map_err(Into::into)
        }, CLEANUP_TIMEOUT).await.unwrap();
        fs::write(directory.join("cleaned"), b"ok").unwrap();
        let started = Instant::now();
        while !directory.join("exit").exists() {
            assert!(started.elapsed() < DEADLINE, "parent never confirmed live-process port release");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    runtime.shutdown_timeout(RUNTIME_TIMEOUT);
}
