# MCP runtime status UI, stage 4

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This candidate adds a read-only runtime snapshot to Advanced settings, next to the existing MCP form and connection guide. The form/clipboard guide still use the draft port and explicitly do not claim that it is saved or listening.

## Contract and presentation

The panel invokes `get_mcp_status` with `{ expectedOwner }` from the last accepted configuration. The backend must validate that owner against the active configuration/store before returning:

```ts
{
  state: "stopped" | "running" | "stopping" | "exited";
  configured_enabled: boolean;
  configured_port: number;
  listening_port: number | null;
  headless: boolean;
}
```

Saved enablement, saved port, runtime state, actual listening address, desktop/headless mode, and local receipt time are separate fields. They are never inferred from draft settings or from one another. A saved port of zero is permitted as a legacy observation and labeled as not specifying a fixed port. A listening port must be an integer from 1 through 65535 and can be present only in `running` state. Other states display no listening address. Malformed responses and native failures show a generic retry message without native error text.

“Running” means the backend still tracks an active listener task. This is not a live network, Canvas credential, or authentication test. The UI calls every observation a snapshot and explains that it is not continuously updated. Reading status does not start, stop, probe, or mutate MCP, configuration, or credentials.

## Ownership and refresh behavior

- The separate `savedConfig` state is updated only after guarded configuration initialization or a successful accepted save, including inline saves. It does not rely on the dirty-check baseline, which inline saves do not update.
- A read runs when that saved configuration changes and on manual Refresh. Draft edits do not trigger reads; no polling or timers are added.
- Every request checks the captured session and current configuration owner. A generation counter means overlapping refreshes are latest-request-wins, for both success and error.
- Session invalidation clears the observation and revokes pending callbacks. Owner changes also prevent publication. Render-time ownership prevents an old observation being displayed before effect cleanup.
- Cleanup unsubscribes and disposes the reader, so neither a late response nor a late rejection can update an unmounted panel. The existing settings owner/session/unmount/save guards are unchanged.
- Refresh removes the previous snapshot while loading. An unavailable result is not mislabeled as stopped.

## Verification performed

From this checkout, Node v24.22.1 successfully ran:

```sh
node --test script/test-mcp-status.mjs script/test-mcp-connection.mjs
```

All 25 tests passed: 20 status tests and 5 connection-guide tests. The parser and asynchronous request gate are executed from their exact production TypeScript source using Node's built-in TypeScript stripping. Tests cover all four states, saved/runtime mismatch, headless flags, legacy saved zero and real listening ports, malformed values, redaction, retry, overlapping success/error responses, session/owner/invalidation/unmount boundaries, and no callbacks after disposal. Settings/component wiring and CRLF preservation checks are static source assertions, not rendered UI verification.

`src/components/__tests__/settings_page.test.tsx` also contains React integration coverage for draft-versus-saved status, manual refresh, successful and inline saves, owner rotation, all states, malformed results, redacted errors, zero ports, session and owner invalidation, and unmount. Two obsolete existing guide expectations were corrected to match the already implemented `127.0.0.1:<port>/mcp` and OpenCode `type: "remote"` guide. These integration tests were authored but **not run**.

Full Vitest/React tests, TypeScript checking, lint, app build, browser rendering, and native Tauri-to-UI integration were **not run**: this isolated checkout has no installed frontend dependency tree, and registry access/install attempts are excluded. No dependency substitutions, new paid worker calls, network installs, commits, pushes, or PRs were made. These focused checks are not a full application or runtime interoperability pass.
