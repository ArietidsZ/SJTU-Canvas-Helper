import { test, expect } from "./test";

test("real G6 paints a canvas, retains layout on unrelated render, and updates data", async ({ page }) => {
  await page.evaluate(() => window.canvasLifecycle.mountGraph());
  await expect(page.locator("#graph canvas")).toHaveCount(1);
  await expect.poll(() => page.locator("#graph canvas").evaluate((element) => {
    const canvas = element as HTMLCanvasElement;
    const pixels = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height).data;
    return pixels.some((value, index) => index % 4 === 3 && value > 0);
  })).toBe(true);
  const before = await page.evaluate(() => window.canvasLifecycle.snapshot().graphEvents.filter((event) => event.method === "render"));
  expect(before).toHaveLength(1);
  await page.evaluate(() => window.canvasLifecycle.setGraphLoading(true));
  await expect(page.getByText("计算中，请耐心等待...")).toBeVisible();
  expect(await page.evaluate(() => window.canvasLifecycle.snapshot().graphEvents.filter((event) => event.method === "render"))).toEqual(before);
  await page.evaluate(() => window.canvasLifecycle.updateGraphData());
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().graphEvents.filter((event) => event.method === "render").length)).toBe(2);
  await page.evaluate(() => window.canvasLifecycle.unmountGraph());
  await expect(page.locator("canvas")).toHaveCount(0);
});

test("real G6: repeated mount/unmount removes only owned resize listeners", async ({ page }) => {
  await page.evaluate(() => {
    Object.assign(window, { otherResizeCount: 0 });
    window.addEventListener("resize", () => {
      const state = window as typeof window & { otherResizeCount: number };
      state.otherResizeCount += 1;
    });
  });
  for (let index = 0; index < 3; index += 1) {
    await page.evaluate(() => window.canvasLifecycle.mountGraph());
    await expect(page.locator("#graph canvas")).toHaveCount(1);
    const before = (await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents;
    const id = before.filter((event) => event.method === "render").at(-1)!.id;
    await page.evaluate(() => window.canvasLifecycle.resize());
    const resized = (await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents;
    expect(resized.filter((event) => event.id === id && event.method === "changeSize").length)
      .toBe(before.filter((event) => event.id === id && event.method === "changeSize").length + 1);
    await page.evaluate(() => window.canvasLifecycle.unmountGraph());
    await expect(page.locator("canvas")).toHaveCount(0);
    const departed = (await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents;
    expect(departed.filter((event) => event.id === id && event.method === "destroy")).toHaveLength(1);
    await page.evaluate(() => window.canvasLifecycle.resize());
    expect((await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents).toEqual(departed);
  }
  expect(await page.evaluate(() => (window as typeof window & { otherResizeCount: number }).otherResizeCount)).toBe(6);
});

test("real G6: StrictMode destroys each departed instance exactly once", async ({ page }) => {
  await page.evaluate(() => window.canvasLifecycle.mountGraph(true));
  await expect.poll(() => page.evaluate(() => window.canvasLifecycle.snapshot().graphEvents.filter((event) => event.method === "render").length)).toBe(2);
  await expect(page.locator("#graph canvas")).toHaveCount(1);
  const before = (await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents;
  const ids = before.filter((event) => event.method === "render").map((event) => event.id);
  expect(new Set(ids).size).toBe(2);
  expect(before.filter((event) => event.id === ids[0] && event.method === "destroy")).toHaveLength(1);
  await page.evaluate(() => window.canvasLifecycle.resize());
  const resized = (await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents;
  expect(resized.filter((event) => event.id === ids[0] && event.method === "changeSize"))
    .toEqual(before.filter((event) => event.id === ids[0] && event.method === "changeSize"));
  await page.evaluate(() => window.canvasLifecycle.unmountGraph());
  const after = (await page.evaluate(() => window.canvasLifecycle.snapshot())).graphEvents;
  for (const id of ids) expect(after.filter((event) => event.id === id && event.method === "destroy")).toHaveLength(1);
  await expect(page.locator("canvas")).toHaveCount(0);
});
