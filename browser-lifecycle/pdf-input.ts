// Self-authored ASCII PDF: forty vector-only pages, no fonts/images/attachments,
// JavaScript, external links, or network resources. A real internal annotation
// links page 1 to page 40. Offsets and stream lengths are exact ASCII byte counts.
export const PDF_PAGE_COUNT = 40;

export function makePdfDataUri() {
  const objects: string[] = [];
  const pageId = (index: number) => 3 + index * 2;
  const annotationId = 3 + PDF_PAGE_COUNT * 2;
  objects.push("<< /Type /Catalog /Pages 2 0 R >>");
  objects.push(`<< /Type /Pages /Count ${PDF_PAGE_COUNT} /Kids [${Array.from(
    { length: PDF_PAGE_COUNT }, (_, index) => `${pageId(index)} 0 R`,
  ).join(" ")}] >>`);
  for (let index = 0; index < PDF_PAGE_COUNT; index += 1) {
    const color = index === PDF_PAGE_COUNT - 1 ? "0 0.75 0.25" : "0.75 0 0.25";
    const content = `${color} rg 0 0 240 320 re f\n${index === 0 ? "1 1 1 rg 24 248 192 48 re f\n" : ""}`;
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 240 320] /Resources << >> /Contents ${pageId(index) + 1} 0 R${index === 0 ? ` /Annots [${annotationId} 0 R]` : ""} >>`);
    objects.push(`<< /Length ${content.length} >>\nstream\n${content}endstream`);
  }
  objects.push(`<< /Type /Annot /Subtype /Link /Rect [24 248 216 296] /Border [0 0 2] /C [0 0 0] /Dest [${pageId(PDF_PAGE_COUNT - 1)} 0 R /Fit] >>`);
  let pdf = "%PDF-1.7\n";
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(pdf.length);
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = pdf.length;
  pdf += `xref\n0 ${offsets.length}\n0000000000 65535 f \n`;
  pdf += offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  pdf += `trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return `data:application/pdf;base64,${btoa(pdf)}`;
}
