# MCP client configuration clipboard actions

The connection guide previously used a “Copy configuration” label for a payload containing prose and two JSON objects. That payload was useful as documentation but was not valid client configuration JSON.

Settings now separates four actions:

- **Copy URL** copies only the loopback Streamable HTTP endpoint.
- **Copy OpenCode JSON** copies one JSON object with the `mcp.sjtu-canvas` remote entry.
- **Copy generic JSON** copies one JSON object with the `mcpServers.sjtu-canvas` URL entry. Clients may require their own schema.
- **Copy instructions** retains the complete, selectable guide.

The guide and clipboard JSON share one formatter. No action includes the Canvas token, writes settings, connects a client, or invokes a model. All payloads follow the current draft port, with the existing unsaved/runtime distinction kept visible. The UI reminds users to merge JSON into their existing configuration rather than replace other servers. Missing tokens or invalid ports disable connection-payload copying. Clipboard rejection or an unavailable clipboard API shows a generic message with a manual-copy fallback.

## Regression checks

Run with the repository's installed dependencies:

```sh
node --test script/test-mcp-connection.mjs script/test-mcp-status.mjs
node node_modules/vitest/vitest.mjs run src/components/__tests__/settings_page.test.tsx
node node_modules/eslint/bin/eslint.js src/
node node_modules/typescript/bin/tsc --noEmit
```

The clipboard changes add two pure formatter tests and seven real React/MUI interaction cases. They cover parseable single-object JSON, exact URL copying, draft port changes and repeated clicks, token omission, disabled inputs, rejected/unavailable clipboard access and retry. Native calls, accounts, clipboard and external requests are test fixtures; this is not a real client connection or native GUI acceptance test.

The 2026-11-07 offline check passed all 76 Settings tests and all 27 focused MCP Node tests. Full-source ESLint had zero errors and 47 existing warnings. Full-project TypeScript remained blocked by missing `hls.js` and its two derived implicit-any diagnostics; unchanged `d9791fec` produced the identical three diagnostics with the same installed dependencies. The cached runtime differs from the lock (including Vite), so these results are not a locked dependency installation/build pass. No Rust production file or lockfile changed. Packaged desktop and live-account checks remain separate requirements.
