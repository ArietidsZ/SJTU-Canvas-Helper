import { test, expect } from "./test";

test("native Blob: session deduplication, close/reopen, and caller ownership", async ({ page }) => {
  const caller = await page.evaluate(() => window.canvasLifecycle.mountPreview("preview", { duplicate: true }));
  await expect(page.locator("output")).toHaveCount(2);
  const first = await page.locator("output").first().getAttribute("data-url");
  expect(first).toMatch(/^blob:http:\/\/127\.0\.0\.1:4179\//);
  expect(await page.evaluate(async (url) => (await fetch(url!)).text(), first)).toBe("%PDF-1.7\n");
  expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).conversions).toBe(1);
  await page.evaluate(() => window.canvasLifecycle.setPreviewOpen("preview", false));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().blobs.revoked)).toEqual([first]);
  expect(await page.evaluate(async (url) => { try { await fetch(url!); return false; } catch { return true; } }, first)).toBe(true);
  expect(await page.evaluate(async (url) => (await fetch(url)).ok, caller)).toBe(true);
  await page.evaluate(() => window.canvasLifecycle.setPreviewOpen("preview", true));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().blobs.created.length)).toBe(2);
  const second = (await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.created[1];
  expect(second).not.toBe(first);
  expect(await page.evaluate(async (url) => (await fetch(url)).ok, second)).toBe(true);
  await page.evaluate(() => window.canvasLifecycle.unmountPreview("preview"));
  expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.revoked).toEqual([first, second]);
  await page.evaluate((url) => window.canvasLifecycle.releaseCaller(url), caller);
});

test("native Blob: late conversion after dismissal creates no orphan URL", async ({ page }) => {
  const caller = await page.evaluate(() => {
    window.canvasLifecycle.setDeferred(true);
    return window.canvasLifecycle.mountPreview("late");
  });
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().pending)).toBe(1);
  await page.evaluate(() => {
    window.canvasLifecycle.unmountPreview("late");
    window.canvasLifecycle.finishConversions();
  });
  // Drain conversion continuation and a render frame before observing allocation.
  await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
  expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs).toEqual({ created: [], revoked: [] });
  await page.evaluate((url) => window.canvasLifecycle.releaseCaller(url), caller);
});

test("native Blob: replacement and separate modals retain independent ownership", async ({ page }) => {
  const one = await page.evaluate(() => window.canvasLifecycle.mountPreview("one"));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().blobs.created.length)).toBe(1);
  const two = await page.evaluate(() => window.canvasLifecycle.mountPreview("two"));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().blobs.created.length)).toBe(2);
  const initial = (await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.created;
  await page.evaluate(() => window.canvasLifecycle.replacePreview("one"));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().blobs.created.length)).toBe(3);
  const replacement = (await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.created[2];
  expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.revoked).toEqual([initial[0]]);
  await page.evaluate(() => window.canvasLifecycle.unmountPreview("one"));
  expect(await page.evaluate(async (url) => (await fetch(url)).ok, initial[1])).toBe(true);
  expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.revoked).toEqual([initial[0], replacement]);
  await page.evaluate(() => window.canvasLifecycle.unmountPreview("two"));
  expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs.revoked).toEqual([initial[0], replacement, initial[1]]);
  await page.evaluate((urls) => urls.forEach((url) => window.canvasLifecycle.releaseCaller(url)), [one, two]);
});

test("native Blob: StrictMode cleanup and caller-owned PDF passthrough", async ({ page }) => {
  const strictCaller = await page.evaluate(() => window.canvasLifecycle.mountPreview("strict", { strict: true }));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().blobs.created.length)).toBe(1);
  await page.evaluate(() => window.canvasLifecycle.unmountPreview("strict"));
  const afterStrict = await page.evaluate(() => window.canvasLifecycle.snapshot().blobs);
  expect(afterStrict.revoked).toEqual(afterStrict.created);
  const pdfCaller = await page.evaluate(() => window.canvasLifecycle.mountPreview("pdf", { callerPdf: true }));
  await expect(page.locator("output")).toHaveAttribute("data-url", pdfCaller);
  await page.evaluate(() => window.canvasLifecycle.unmountPreview("pdf"));
  expect(await page.evaluate(async (url) => (await fetch(url)).ok, pdfCaller)).toBe(true);
  expect(await page.evaluate(() => window.canvasLifecycle.snapshot().blobs)).toEqual(afterStrict);
  await page.evaluate((urls) => urls.forEach((url) => window.canvasLifecycle.releaseCaller(url)), [strictCaller, pdfCaller]);
});
