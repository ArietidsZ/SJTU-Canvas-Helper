import { defineConfig } from "@playwright/test";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
for (const name of ["@playwright/test", "playwright", "playwright-core"]) {
  if (require(`${name}/package.json`).version !== "1.59.0") {
    throw new Error(`Review the browser security argument filter before changing ${name} 1.59.0`);
  }
}

// Public ignoreDefaultArgs uses exact matches. Keep the entire pinned upstream
// switch, rather than replacing it with a different --disable-features list.
// Source: microsoft/playwright v1.59.0, chromiumSwitches.ts and chromium.ts.
// The browser fixture rejects ANY surviving --disable-features argument, so an
// upstream change that no longer matches fails closed before fixture navigation.
const ignoredDefaults = [
  "--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,BoundaryEventDispatchTracksNodeRemoval,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate,AutoDeElevate,RenderDocument,OptimizationHints",
  "--enable-unsafe-swiftshader",
  "--unsafely-disable-devtools-self-xss-warnings",
  "--disable-client-side-phishing-detection",
  "--disable-ipc-flooding-protection",
  "--disable-hang-monitor",
  "--disable-popup-blocking",
  "--disable-prompt-on-repost",
  "--password-store=basic",
  "--use-mock-keychain",
  "--disable-component-update",
  "--disable-component-extensions-with-background-pages",
  "--disable-extensions",
  "--disable-infobars",
];

// This suite is for the standard GitHub-hosted runner, not the restricted local
// executor. Missing Chrome or a failed sandbox is a hard failure: no fallback.
export default defineConfig({
  testDir: "./browser-lifecycle",
  testMatch: "**/*.browser.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  maxFailures: 1, // Stop immediately on a browser safety gate or test failure.
  timeout: 30_000,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4179",
    browserName: "chromium",
    channel: "chrome",
    headless: true,
    ignoreHTTPSErrors: false,
    serviceWorkers: "block",
    launchOptions: { chromiumSandbox: true, ignoreDefaultArgs: ignoredDefaults },
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
