import type { Page, Worker } from "@playwright/test";
import { test, expect } from "./test";
import { PDF_PAGE_COUNT } from "./pdf-input";

// Scope high-DPI coverage to these PDF tests; leave the original seven alone.
test.use({ deviceScaleFactor: 2 });

const root = "#pdf-renderer";
const maxCanvases = 6; // A small visible/adjacent window, not forty retained pages.
const pageCanvas = (page: Page, number: number) =>
  page.locator(`${root} .react-pdf__Page[data-page-number="${number}"] canvas`);

async function expectPaintedPage(page: Page, number: number) {
  // Inspect actual browser raster pixels, not DOM presence or a mocked render.
  const color = number === PDF_PAGE_COUNT ? [0, 191, 64, 255] : [191, 0, 64, 255];
  await expect.poll(() => pageCanvas(page, number).evaluateAll((elements, expected) => {
    const canvas = elements[0] as HTMLCanvasElement | undefined;
    if (!canvas || !canvas.width || !canvas.height || getComputedStyle(canvas).display === "none") return false;
    const pixel = canvas.getContext("2d")!.getImageData(Math.floor(canvas.width / 2), Math.floor(canvas.height / 2), 1, 1).data;
    return expected.every((value, index) => Math.abs(pixel[index] - value) <= 2);
  }, color), { message: `Page ${number} must paint its vector color through real PDF.js` }).toBe(true);
}

async function rasterSnapshot(page: Page, stage: string) {
  const snapshot = await page.locator(`${root} canvas`).evaluateAll((elements) => ({
    devicePixelRatio: window.devicePixelRatio,
    canvases: elements.map((element) => {
      const canvas = element as HTMLCanvasElement;
      return {
        page: canvas.closest(".react-pdf__Page")?.getAttribute("data-page-number"),
        width: canvas.width,
        height: canvas.height,
      };
    }),
  }));
  console.log("PDF raster snapshot:", JSON.stringify({ stage, count: snapshot.canvases.length, ...snapshot }));
  expect(snapshot.devicePixelRatio, "PDF regression must exercise an actual high-DPI context").toBe(2);
  return snapshot;
}

async function expectBoundedCanvases(page: Page, stage: string) {
  // Await all currently mounted public React-PDF Page nodes, not an arbitrary
  // sleep. On the old implementation these are all forty pages, ensuring the
  // negative control fails on canvas count rather than winning an early race.
  await expect.poll(() => page.locator(`${root} .react-pdf__Page`).evaluateAll((pages) =>
    pages.length > 0 && pages.every((element) => element.querySelector("canvas")),
  )).toBe(true);
  const count = (await rasterSnapshot(page, stage)).canvases.length;
  expect(count, `Only nearby PDF pages should own canvases (document has ${PDF_PAGE_COUNT} pages)`).toBeLessThanOrEqual(maxCanvases);
  expect(count).toBeGreaterThan(0);
}

async function mountPdf(page: Page) {
  // Playwright observes native worker creation/close; no Worker interception or
  // fake-worker fallback. The unchanged network gate also rejects external URLs.
  const workerCreated = page.waitForEvent("worker");
  await page.evaluate(() => window.canvasLifecycle.mountPdf());
  const worker = await workerCreated;
  expect(new URL(worker.url()).origin).toBe("http://127.0.0.1:4179");
  expect(new URL(worker.url()).pathname).toMatch(/pdf\.worker(?:\.min)?\.js$/);
  await expect(page.getByText(`共 ${PDF_PAGE_COUNT} 页`, { exact: true })).toBeVisible();
  await expectPaintedPage(page, 1);
  expect(page.workers(), "PDF rendering must retain the real worker, not fall back to the main thread").toContain(worker);
  // Both old and new renderers expose their full extent through this existing
  // React-PDF DOM; wait for all forty equal-size pages, including progressive
  // metadata implementations. No data-pdf-* attribute from the fix is required.
  const pageHeight = await pageCanvas(page, 1).evaluate((canvas) => canvas.getBoundingClientRect().height);
  await expect.poll(() => page.locator(`${root} .react-pdf__Document`).evaluate((document) => document.scrollHeight)).toBeGreaterThanOrEqual(Math.floor(pageHeight) * PDF_PAGE_COUNT);
  return worker;
}

async function unmountPdf(page: Page, worker: Worker) {
  const workerClosed = worker.waitForEvent("close");
  await page.evaluate(() => window.canvasLifecycle.unmountPdf());
  await expect(page.locator(`${root} canvas`)).toHaveCount(0);
  await rasterSnapshot(page, "unmounted");
  await workerClosed;
  expect(page.workers(), "Unmount must stop the real PDF.js worker").toEqual([]);
  const blobs = (await page.evaluate(() => window.canvasLifecycle.snapshot())).blobs;
  expect(blobs.created).toHaveLength(1);
  expect(blobs.revoked).toEqual(blobs.created);
}

async function scrollPdf(page: Page, to: "start" | "end") {
  await page.locator(`${root} .react-pdf__Document`).evaluate((document, target) => {
    let container = document.parentElement;
    while (container && !/^(auto|scroll)$/.test(getComputedStyle(container).overflowY)) container = container.parentElement;
    if (!container) throw new Error("Production PDF scroll container is missing");
    container.scrollTop = target === "end" ? container.scrollHeight : 0;
  }, to);
}

test("real PDF: first viewport owns bounded canvases, then unmount stops its worker", async ({ page }) => {
  const worker = await mountPdf(page);
  await expectBoundedCanvases(page, "first viewport");
  await unmountPdf(page, worker);
});

test("real PDF: scrolling to the last page paints it and releases distant canvases", async ({ page }) => {
  const worker = await mountPdf(page);
  await expectBoundedCanvases(page, "before scroll");
  await scrollPdf(page, "end");
  await expectPaintedPage(page, PDF_PAGE_COUNT);
  await expect(pageCanvas(page, PDF_PAGE_COUNT)).toBeInViewport();
  await expectBoundedCanvases(page, "scrolled to last page");
  await expect(pageCanvas(page, 1)).toHaveCount(0);
  await scrollPdf(page, "start");
  await expectPaintedPage(page, 1);
  await expect(pageCanvas(page, 1)).toBeInViewport();
  await expectBoundedCanvases(page, "returned to first page");
  await expect(pageCanvas(page, PDF_PAGE_COUNT)).toHaveCount(0);
  await unmountPdf(page, worker);
});

test("real PDF: internal last-page link survives zoom and resize with bounded canvases", async ({ page }) => {
  const worker = await mountPdf(page);
  await expectBoundedCanvases(page, "before zoom");
  const initialWidth = await pageCanvas(page, 1).evaluate((canvas) => (canvas as HTMLCanvasElement).width);
  await page.getByRole("button", { name: "放大", exact: true }).click();
  await expect(page.getByText("缩放 110%", { exact: true })).toBeVisible();
  await expect.poll(() => pageCanvas(page, 1).evaluate((canvas) => (canvas as HTMLCanvasElement).width)).toBeGreaterThan(initialWidth);
  await expectPaintedPage(page, 1);
  await expectBoundedCanvases(page, "zoom 110%");
  const zoomWidth = await pageCanvas(page, 1).evaluate((canvas) => (canvas as HTMLCanvasElement).width);
  await page.evaluate(() => window.canvasLifecycle.resizePdf(620));
  await expect.poll(() => pageCanvas(page, 1).evaluate((canvas) => (canvas as HTMLCanvasElement).width)).toBeLessThan(zoomWidth);
  await expectPaintedPage(page, 1);
  await expectBoundedCanvases(page, "resized host to 620px");
  const internalLink = page.locator(`${root} .react-pdf__Page[data-page-number="1"] .linkAnnotation a`);
  await expect(internalLink).toHaveCount(1);
  await internalLink.click();
  await expectPaintedPage(page, PDF_PAGE_COUNT);
  await expect(pageCanvas(page, PDF_PAGE_COUNT)).toBeInViewport();
  await expectBoundedCanvases(page, "internal link to last page");
  await expect(pageCanvas(page, 1)).toHaveCount(0);
  await unmountPdf(page, worker);
});
