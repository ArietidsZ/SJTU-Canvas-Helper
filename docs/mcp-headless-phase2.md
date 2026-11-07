# Headless MCP CLI, phase 2 candidate

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This incremental candidate retains the PR #139 configuration owner, exclusive
configuration-store lock, account recovery, transactional saves and bounded MCP
lifecycle. It adds an explicit CLI path before Tauri setup. It is not a native
release verification or a replacement for the remaining MCP safety work.

## Start and stop

Configure the account in the desktop application, then **fully quit the desktop
application** before starting the same executable in a terminal:

```sh
"/path/to/SJTU Canvas Helper" --mcp
"/path/to/SJTU Canvas Helper" --mcp --mcp-port 3101
```

On macOS, invoke the executable inside the application bundle, for example:

```sh
"/Applications/SJTU Canvas Helper.app/Contents/MacOS/SJTU Canvas Helper" --mcp
```

The endpoint is `http://127.0.0.1:<port>/mcp`, using Streamable HTTP. The saved
MCP port is used unless `--mcp-port` supplies an integer from 1 to 65535. An
invalid saved port fails startup unless a valid process override is supplied.
`--mcp` enables this process regardless of the saved GUI `mcp_enabled` setting.
Neither option changes saved settings, the selected account, or credentials.
Normal startup account loading/recovery and diagnostic log writes still occur;
this is not a filesystem-read-only mode. Invalid or unknown MCP options are
rejected before diagnostics, account loading/recovery or configuration mutation.
Only `--mcp` and `--mcp-port <port>` are supported. Repeated options, other
headless arguments, and `--mcp-allow-ai` fail rather than being silently ignored.

No window, WebView, video-login initialization or GUI event loop starts in this
mode. No-argument startup keeps the existing desktop application behavior. The
CLI does not install a service, a startup item or a background daemon. The
calling terminal or service manager owns its process lifetime.

On Unix, SIGINT (usually Ctrl+C) and SIGTERM request cleanup. Signal handlers are
registered before application loading or listener startup. Registration failure
therefore opens no application listener. Signal-wait errors, bind errors after
headless setup and unexpected listener exits return failure after cleanup. MCP
and playback-proxy cleanup share a 25-second deadline; runtime teardown gets a
further two seconds. A cleanup error or timeout is a nonzero exit, not a report
of confirmed termination. Stopping cannot revoke requests already sent to a
remote service. SIGKILL and other forced process termination cannot run cleanup.

The Windows release binary retains its existing GUI subsystem. Windows console
attachment, Ctrl+C/Ctrl+Break delivery and packaged-process behavior have not
been validated. The code registers those control streams, but this guide does
not promise that Ctrl+C works when launching a GUI-subsystem executable, or that
closing a terminal/using Task Manager performs graceful cleanup. Windows and
macOS native validation remain release gates.

## Ownership, ports and tools

The GUI and headless process **cannot share one configuration directory at the
same time**, even with different MCP ports. The existing exclusive store lock
is intentionally retained. Stop the other process; do not remove the lock file
or bypass ownership checks. An unavailable owner, corrupt configuration or an
uncertain account transaction must fail closed. This phase does not add an
account selector or an alternative configuration directory option.

The process-only port/policy is installed once under the owner lock. It stays
separate from `AppConfig`, so a later owner-checked save/restart cannot persist
the CLI port or re-enable the disabled headless tools. A rejected headless
configuration does not stop a pre-existing runtime. Binding an occupied port is
an immediate startup failure; a successful TCP bind is not account validity.

Headless discovery and direct invocation reject `chat`, `explain_file` and the
legacy `test_token` credential-probe tool. There is deliberately no AI opt-in in
this phase. **The GUI MCP path retains its existing tool exposure, including AI
tools**, which may send content to the configured external model provider and
incur charges. This CLI gate does not make the existing GUI tools safe by default.
Other tools continue to use their existing implementations and may contact
Canvas or access local state. Typed schemas, the earlier download/explanation
safety design, and complete tool-surface review have not been integrated here;
do not describe the whole service as audited or read-only.

Loopback is not user authentication: other local processes may access Canvas
data through an enabled listener. Use a trusted local environment. Do not
expose the service through a reverse proxy or assume Host/Origin checks provide
a credential boundary. Client-provided Canvas content is untrusted data.

## Focused verification and limits

The argument parser is std-only and can be compiled/tested without the app or
Cargo dependency resolution:

```sh
rustc --edition 2021 --test src-tauri/src/mcp/cli.rs -o /tmp/canvas-mcp-cli-tests
/tmp/canvas-mcp-cli-tests
```

The app tests in `app/mcp_headless_tests.rs` use temporary configuration roots,
loopback listeners and no Canvas/LLM calls. They cover ownership failure,
one-time options, invalid saved ports, process-only overrides, existing-runtime
protection, bind failure, signal-wait success/failure, listener loss, proxy
cleanup and transactional restart. `mcp/headless.rs` also tests the cleanup
result/deadline composition, while `mcp/mod.rs` tests discovery and direct guards.
These source tests still require execution on a compatible full app toolchain.
A dependency-isolated harness can validate selected modules, but it cannot
prove the real Tauri entrypoint, account-store integration or native packaging.

The root toolchain requests Rust 1.99.0, whereas the manifest minimum is 1.93.1.
No toolchain/dependency downloads, native app launches, real account/API calls,
AI requests, commits or pushes are part of this candidate. Consult the delivered
validation report for exactly which offline checks ran; source inclusion is
not evidence that a test passed.
