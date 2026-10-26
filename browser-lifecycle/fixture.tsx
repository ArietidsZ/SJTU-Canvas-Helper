import G6, { type Graph } from "@antv/g6";
import { StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import PreviewModal from "../src/components/preview_modal";
import RelationshipGraph from "../src/components/relationship_graph";
import type { File } from "../src/lib/model";
import { boundary, pdfBytes } from "./boundaries";

const blobs = { created: [] as string[], revoked: [] as string[] };
const nativeCreate = URL.createObjectURL.bind(URL);
const nativeRevoke = URL.revokeObjectURL.bind(URL);
// Call-through observers: allocation, revocation and fetch stay browser-native.
URL.createObjectURL = (object) => {
  const url = nativeCreate(object);
  blobs.created.push(url);
  return url;
};
URL.revokeObjectURL = (url) => {
  blobs.revoked.push(url);
  nativeRevoke(url);
};

type GraphMethod = "render" | "destroy" | "changeSize";
const graphEvents: { id: number; method: GraphMethod }[] = [];
const graphIds = new WeakMap<Graph, number>();
let nextGraphId = 0;
for (const method of ["render", "destroy", "changeSize"] as const) {
  const original = G6.Graph.prototype[method];
  // Observe the actual implementation, not a fake Graph or fake canvas.
  Object.defineProperty(G6.Graph.prototype, method, {
    configurable: true,
    writable: true,
    value(this: Graph, ...args: unknown[]) {
      let id = graphIds.get(this);
      if (id === undefined) { id = ++nextGraphId; graphIds.set(this, id); }
      graphEvents.push({ id, method });
      return Reflect.apply(original, this, args);
    },
  });
}

type Mounted = { root: Root; host: HTMLDivElement; files: File[]; strict: boolean };
const previews = new Map<string, Mounted>();
let graphMount: { root: Root; host: HTMLDivElement; strict: boolean } | undefined;
function makeRoot(id: string) {
  const host = document.createElement("div");
  host.id = id;
  host.style.width = "800px";
  document.getElementById("fixtures")!.append(host);
  return { host, root: createRoot(host) };
}
function drawPreview(mount: Mounted, open: boolean) {
  const component = <PreviewModal open={open} files={mount.files} />;
  mount.root.render(mount.strict ? <StrictMode>{component}</StrictMode> : component);
}
function drawGraph() {
  if (!graphMount) throw new Error("No graph mounted");
  const component = <RelationshipGraph />;
  graphMount.root.render(graphMount.strict ? <StrictMode>{component}</StrictMode> : component);
}

const api = {
  snapshot: () => ({
    blobs: { created: [...blobs.created], revoked: [...blobs.revoked] },
    conversions: boundary.calls,
    pending: boundary.pending.length,
    graphEvents: [...graphEvents],
  }),
  setDeferred(value: boolean) { boundary.deferred = value; },
  finishConversions() { boundary.pending.splice(0).forEach((resolve) => resolve(pdfBytes)); },
  mountPreview(id: string, options: { strict?: boolean; duplicate?: boolean; callerPdf?: boolean } = {}) {
    if (previews.has(id)) throw new Error(`Preview already mounted: ${id}`);
    // Input ownership belongs to the fixture/caller, not PreviewModal.
    const url = nativeCreate(new Blob([new Uint8Array(pdfBytes)], { type: "application/pdf" }));
    const file = { display_name: options.callerPdf ? "caller.pdf" : "slides.pptx", url } as File;
    const mount = { ...makeRoot(id), strict: Boolean(options.strict), files: options.duplicate ? [file, file] : [file] };
    previews.set(id, mount);
    drawPreview(mount, true);
    return url;
  },
  setPreviewOpen(id: string, open: boolean) {
    const mount = previews.get(id);
    if (!mount) throw new Error(`Unknown preview: ${id}`);
    drawPreview(mount, open);
  },
  replacePreview(id: string) {
    const mount = previews.get(id);
    if (!mount) throw new Error(`Unknown preview: ${id}`);
    mount.files = [{ display_name: "replacement.pptx", url: mount.files[0].url } as File];
    drawPreview(mount, true);
  },
  unmountPreview(id: string) {
    const mount = previews.get(id);
    if (!mount) throw new Error(`Unknown preview: ${id}`);
    mount.root.unmount();
    mount.host.remove();
    previews.delete(id);
  },
  releaseCaller(url: string) { nativeRevoke(url); },
  mountGraph(strict = false) {
    if (graphMount) throw new Error("Graph already mounted");
    graphMount = { ...makeRoot("graph"), strict };
    drawGraph();
  },
  setGraphLoading(value: boolean) { boundary.relationship.isLoading = value; drawGraph(); },
  updateGraphData() {
    boundary.relationship.data = {
      nodes: [{ id: "updated", label: "Updated", nodeType: "Course" }], edges: [],
    };
    drawGraph();
  },
  resize() { window.dispatchEvent(new Event("resize")); },
  unmountGraph() {
    if (!graphMount) throw new Error("No graph mounted");
    graphMount.root.unmount();
    graphMount.host.remove();
    graphMount = undefined;
  },
};

export type LifecycleApi = typeof api;
declare global { interface Window { canvasLifecycle: LifecycleApi } }
window.canvasLifecycle = api;
