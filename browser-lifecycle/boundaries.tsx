import type { RelationshipTopo } from "../src/lib/model";

export const pdfBytes = [37, 80, 68, 70, 45, 49, 46, 55, 10];
export const boundary = {
  calls: 0,
  deferred: false,
  pending: [] as Array<(value: number[]) => void>,
  relationship: {
    isLoading: false,
    data: {
      nodes: [
        { id: "me", label: "Me", nodeType: "Me" },
        { id: "course", label: "Course", nodeType: "Course" },
      ],
      edges: [{ source: "me", target: "course" }],
    } as RelationshipTopo,
  },
};

export function invoke(command: string): Promise<number[]> {
  if (command !== "convert_pptx_to_pdf") {
    throw new Error(`Unexpected native IPC in offline fixture: ${command}`);
  }
  boundary.calls += 1;
  return boundary.deferred
    ? new Promise((resolve) => boundary.pending.push(resolve))
    : Promise.resolve(pdfBytes);
}

export function useRelationship() { return boundary.relationship; }
export async function getFileType() { return "pdf"; }
export const BasicRenderers = [];

// Pure input decoding only, matching the production helpers. The existing utils
// alias excludes native/config side effects; it does not replace PDF rendering.
export function getBase64Data(raw?: string) {
  if (!raw) return "";
  return raw.startsWith("data:") && raw.includes(",") ? raw.split(",")[1] : raw;
}
export function base64ToBuffer(base64: string) {
  const binary = atob(base64);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

// This is deliberately NOT a PDF-rendering/UI acceptance test. Only the real
// PreviewModal's URL ownership is under test; the native browser fetches bytes.
export default function DocumentBoundary({ documents }: { documents: { uri: string }[] }) {
  return <div data-testid="documents">
    {documents.map((document, index) => <output key={index} data-url={document.uri}>{document.uri}</output>)}
  </div>;
}
