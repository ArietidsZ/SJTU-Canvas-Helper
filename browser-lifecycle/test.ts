import { test as base, expect } from "@playwright/test";
import type {} from "./fixture";

const fixtureOrigin = "http://127.0.0.1:4179";

export const test = base.extend<{ offlineGuard: void }>({
  offlineGuard: [async ({ context }, use) => {
    const external: string[] = [];
    const errors: string[] = [];
    context.on("page", (page) => page.on("pageerror", (error) => errors.push(error.message)));
    await context.route("**/*", async (route) => {
      const url = route.request().url();
      if (new URL(url).origin === fixtureOrigin) await route.continue();
      else { external.push(url); await route.abort("blockedbyclient"); }
    });
    await context.routeWebSocket("**/*", (socket) => {
      if (new URL(socket.url()).origin === fixtureOrigin.replace("http:", "ws:")) socket.connectToServer();
      else { external.push(socket.url()); socket.close(); }
    });
    await use();
    expect(external, "No non-loopback application network attempts are permitted").toEqual([]);
    expect(errors, "No uncaught browser errors are permitted").toEqual([]);
  }, { auto: true }],
});
export { expect };

test.beforeEach(async ({ page }) => {
  await page.goto("/browser-lifecycle/");
  await page.waitForFunction(() => Boolean(window.canvasLifecycle));
});

test.afterEach(async ({ page }) => {
  // Let real G6/React cleanup callbacks run before the error/network assertions.
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));
});
