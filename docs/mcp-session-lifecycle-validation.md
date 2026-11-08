# MCP session and SSE shutdown regressions

This test-only follow-up is based on `b8edd6a532683b667032aa71d4aabf1ecbfac137`.
It adds no production behavior, service installation or account changes.

## Newly covered boundaries

Four loopback tests exercise the production MCP listener, router, session
manager and rmcp transport with real HTTP initialization and open SSE streams:

1. Graceful stop closes an initialized SSE stream. Restarting on the same port
   in headless mode rejects the old session with HTTP 404. A fresh session can
   discover tools, but neither discovery nor direct HTTP invocation exposes
   `chat`, `explain_file` or `test_token`.
2. Dropping the runtime closes the admitted SSE stream and releases its port.
3. Dropping a client's SSE stream allows a new stream and tool discovery in the
   same initialized session.
4. Interrupting `stop()` while a synthetic handler lock is held still closes
   SSE. The retained runtime can complete cleanup after the lock is released,
   and a further stop is idempotent.

The tests first establish that each stream is actually pending. A client-side
whole-response timeout is not used to fake server shutdown. Every operation
has a bounded deadline, and only temporary IPv4 loopback ports are contacted.
The held handler lock is a fixture, not a real Canvas operation. Tool discovery
and rejected sensitive-tool calls do not invoke application work.

## Verification on 2026-11-07

- All 51 MCP-module tests passed, including the 47 existing tests and these four.
- The four new tests also passed five consecutive additional runs.
- Disconnecting the rmcp service from the runtime's cancellation token in an
  isolated harness caused the Drop and interrupted-stop tests to fail because
  their SSE streams stayed open. Normal graceful shutdown still passed because
  explicit session cleanup provides a separate close path. The mutation was
  removed before the final 51-test run.
- All copied MCP modules matched the final source byte-for-byte. The harness
  retained its existing manifest and lockfile. All 139 Linux dependency archives
  matched the project's lockfile checksums, and their unpacked source files
  matched the archives.

The run used the existing offline harness, Rust/Cargo 1.93.1, one build job,
one test thread, one CPU and low priority. No dependency was fetched.

## Limits

This is MCP-module integration, not full App/Tauri integration. Application
construction and external-service methods remain harness fixtures. The full
project pins Rust 1.99.0; full native builds, Clippy, Rustfmt, all app tests,
real accounts, live Canvas/model calls and packaged desktop acceptance were
not run. No frontend code changed, so frontend suites were not rerun. These
checks do not prove that stopping revokes already-issued remote work or stops
an uncooperative blocking parser.

On a compatible full project environment, the new tests can be selected with:

```sh
cargo test --manifest-path src-tauri/Cargo.toml \
  mcp::lifecycle::session_lifecycle_tests -- --test-threads=1
```
