import { defineConfig } from "@playwright/test";

// This suite is for the standard GitHub-hosted runner, not the restricted local
// executor. Missing Chrome or a failed sandbox is a hard failure: no fallback.
export default defineConfig({
  testDir: "./browser-lifecycle",
  testMatch: "**/*.browser.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 30_000,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4179",
    browserName: "chromium",
    channel: "chrome",
    headless: true,
    ignoreHTTPSErrors: false,
    serviceWorkers: "block",
    launchOptions: { chromiumSandbox: true },
    trace: "off",
    screenshot: "off",
    video: "off",
  },
  webServer: {
    command: "node node_modules/vite/bin/vite.js --config vite.lifecycle.config.ts",
    url: "http://127.0.0.1:4179/browser-lifecycle/",
    reuseExistingServer: false,
    timeout: 60_000,
  },
});
