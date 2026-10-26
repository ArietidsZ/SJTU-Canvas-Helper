import { test as base, expect, type Page } from "@playwright/test";
import type {} from "./fixture";

const fixtureOrigin = "http://127.0.0.1:4179";

export const test = base.extend<{ offlineGuard: void }>({
  // Override the public worker-scoped browser fixture, preserving its real
  // browser instance. Context/page creation depends on this safety verification.
  browser: [async ({ browser }, use) => {
    const session = await browser.newBrowserCDPSession();
    try {
      const { arguments: args } = await session.send("Browser.getBrowserCommandLine");
      const forbidden = args.filter((argument) =>
        /unsafe|no-sandbox|no-zygote|single-process|ignore.*certif|disable.*(?:sandbox|web-security|site-isolation|phishing|flooding|hang-monitor|popup-blocking|prompt-on-repost|extensions|component-update|infobars)|password-store=basic|use-mock-keychain|allow-(?:insecure-localhost|running-insecure-content|file-access-from-files)|^--disable-features(?:=|$)/i.test(argument));
      expect(forbidden, "Actual Chrome arguments must not weaken browser protections").toEqual([]);
      console.log("Browser security preflight:", JSON.stringify({
        version: browser.version(), chromiumSandbox: true, ignoreHTTPSErrors: false, arguments: args,
      }));
    } finally {
      await session.detach();
    }
    await use(browser);
  }, { scope: "worker" }],
  offlineGuard: [async ({ context, page, ignoreHTTPSErrors }, runTest) => {
    expect(ignoreHTTPSErrors, "TLS certificate validation must remain enabled").toBe(false);
    const external: string[] = [];
    const errors: string[] = [];
    const observed = new WeakSet<Page>();
    const observe = (target: Page) => {
      if (observed.has(target)) return;
      observed.add(target);
      target.on("pageerror", (error) => errors.push(error.message));
    };
    // `page` may already exist when this auto fixture starts; cover both it and
    // every later page before any fixture content is loaded.
    context.on("page", observe);
    context.pages().forEach(observe);
    await context.route("**/*", async (route) => {
      const url = route.request().url();
      if (new URL(url).origin === fixtureOrigin) await route.continue();
      else { external.push(url); await route.abort("blockedbyclient"); }
    });
    await context.routeWebSocket("**/*", (socket) => {
      if (new URL(socket.url()).origin === fixtureOrigin.replace("http:", "ws:")) socket.connectToServer();
      else { external.push(socket.url()); socket.close(); }
    });
    await page.goto("/browser-lifecycle/");
    await page.waitForFunction(() => Boolean(window.canvasLifecycle));
    try {
      await runTest();
    } finally {
      // Real cleanup callbacks finish before network/error assertions. These
      // are render frames, not a timed sleep, retry, or relaxed assertion.
      await page.evaluate(() => new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      }));
      expect(external, "No non-loopback application network attempts are permitted").toEqual([]);
      expect(errors, "No uncaught browser errors are permitted").toEqual([]);
    }
  }, { auto: true }],
});
export { expect };
