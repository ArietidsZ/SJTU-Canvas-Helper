# MCP connection guide, phase 1

> 阶段记录：本文保留该阶段的实现和验证范围。当前开发分支的完整使用说明与限制见 [MCP 使用指南](mcp.md)。

This overlay is based on PR #139 head 889278b6, copied from the verified 0200 snapshot. Only the settings connection guide, its pure generator, and focused tests are included. It does not add runtime status, a headless CLI, or AI opt-in controls.

The endpoint is `http://127.0.0.1:<port>/mcp` using Streamable HTTP. Enable MCP and save settings in the running desktop application first. The guide reflects the form value, not confirmed runtime status. Generic URL snippets may need client-specific adaptation. The OpenCode example uses the `mcp` field, `type: "remote"`, and `enabled: true`. A client running on another host cannot use this loopback URL to reach the desktop app.

Loopback is a network boundary, not user authentication: other local processes may access Canvas data. Use only in a trusted local environment; do not expose through a reverse proxy. Existing AI tools may send content to the configured external model provider and incur charges; review client permissions and model configuration before invoking them. This phase does not add an AI opt-in switch or claim those tools are disabled by default. Invalid ports generate an error message without advertising a fallback endpoint. An omitted port uses 3100.

## Verification

Run `node --test script/test-mcp-connection.mjs` from this overlay. Node v24.22.1 executed five passing focused tests using built-in TypeScript stripping on the exact utility source. Tests cover default/custom/boundary and invalid ports, JSON structure, no legacy SSE or unimplemented CLI/status claims, and shared display/clipboard generation. The settings checks are static source checks, not rendered UI tests. The standalone script is outside Vitest’s default test-file glob and uses node:test only; no Vitest file was added. The safety wording assertions also require the external-model cost notice and reject claims of default-disabled AI tools.

The settings diff contains exactly four localized changes: one import, the existing memo's guide implementation, and two explanatory text blocks. All mounted/session/owner checks, saveConfig successor handling, unmount guards, inline-save guards, form behavior, and clipboard callbacks are unchanged from the baseline. Existing source CRLF endings were preserved.

Full Vitest, TypeScript type checking, lint, build and UI/runtime interoperability were NOT run: this isolated snapshot has no node_modules, and dependency installation/network use is excluded. No old dependency tree was reused. No backend changes, commits, pushes, or PRs were made.

## Later CLI candidate

The separate [phase 2 headless guide](mcp-headless-phase2.md) describes the new explicit CLI path. The original phase 1 settings guide and its GUI behavior remain unchanged. Phase 2 disables AI and token-probe tools only in headless mode; it does not change the GUI MCP exposure described above.

The later [stage 4 status UI](mcp-status-stage4-ui.md) adds a separate saved/runtime snapshot. The connection guide remains draft-based; its warning that it is not runtime status still applies.
