# MCP runtime status snapshot

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This unpublished stage adds a read-only `get_mcp_status(expected_owner)` Tauri command. It checks the configuration owner and persistence readiness, and reads the saved configuration and managed runtime under the same owner → runtime → configuration lock order used by startup. It does not start or stop anything, probe a network endpoint, or return credentials, account names, owner tokens or raw errors.

`configured_enabled` and `configured_port` describe the saved configuration. `headless` describes the process policy. A headless CLI port override is not written to saved configuration. `listening_port` is the actual bound port and is present only in a running observation. A saved legacy port of zero can be observed; it is not a usable connection port.

States are deliberately about the tracked runtime task:

- `stopped`: no runtime is in the managed slot.
- `running`: its listener task has not finished and cancellation has not been requested.
- `stopping`: cancellation has been requested while that task remains active.
- `exited`: the tracked listener task is finished or no longer has a root handle.

These observations are not a live health guarantee. The listener may exit immediately after the snapshot. A cancelled runtime retained after a failed stop is not advertised as running. This status neither proves successful Canvas authentication nor tests access to any remote service. No new authentication mechanism is introduced for the loopback MCP endpoint.

The settings status panel is owner/session scoped and read-only. It refreshes for accepted saved snapshots or on explicit request, rather than continuously polling. Older overlapping reads and replies after owner/session changes or unmount cannot replace the current observation. Malformed replies and backend failures are displayed generically. The saved/runtime status is separate from unsaved form values; the existing connection guide still describes the draft port and retains its local-trust warning.

No changes to account transactions, Archive handling, bounded MCP stop, HTTP-origin/body restrictions, typed tools, safe-download policy or the headless AI denial are intended. Runtime status is not an AI exposure opt-in. Full native/React application and live-account validation remain separate from the isolated checks documented with the stage4 recovery bundle.
