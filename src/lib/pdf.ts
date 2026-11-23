// pdf.js helpers: loading, text and outline extraction, and page images for OCR.

import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy } from "pdfjs-dist";
import type { OutlineItem } from "./api";

pdfjs.GlobalWorkerOptions.workerSrc = new URL("pdfjs-dist/build/pdf.worker.mjs", import.meta.url).toString();

export { pdfjs };
export type { PDFDocumentProxy };

let loadingTask: ReturnType<typeof pdfjs.getDocument> | null = null;

/** Load a PDF, destroying the previously loaded one. */
export async function loadPdf(bytes: Uint8Array): Promise<PDFDocumentProxy> {
  await loadingTask?.destroy();
  loadingTask = pdfjs.getDocument({ data: bytes });
  return loadingTask.promise;
}

export async function extractText(p: PDFDocumentProxy, onProgress: (done: number) => void): Promise<string[]> {
  const out: string[] = [];
  for (let i = 1; i <= p.numPages; i++) {
    const page = await p.getPage(i);
    const tc = await page.getTextContent();
    let text = "";
    for (const item of tc.items) {
      if ("str" in item) text += item.str + (item.hasEOL ? "\n" : "");
    }
    out.push(text.replace(/[ \t]+\n/g, "\n").trim());
    page.cleanup();
    if (i % 10 === 0 || i === p.numPages) onProgress(i);
  }
  return out;
}

export async function extractOutline(p: PDFDocumentProxy): Promise<OutlineItem[]> {
  const raw = await p.getOutline().catch(() => null);
  if (!raw) return [];
  const resolve = async (dest: unknown): Promise<number | null> => {
    try {
      const d = typeof dest === "string" ? await p.getDestination(dest) : (dest as unknown[] | null);
      if (!Array.isArray(d) || d.length === 0) return null;
      const target = d[0];
      if (typeof target === "number") return target + 1;
      return (await p.getPageIndex(target as Parameters<PDFDocumentProxy["getPageIndex"]>[0])) + 1;
    } catch {
      return null;
    }
  };
  type Raw = { title: string; dest: unknown; items: Raw[] };
  const walk = async (items: Raw[]): Promise<OutlineItem[]> =>
    Promise.all(items.map(async (it) => ({ title: it.title, page: await resolve(it.dest), items: await walk(it.items ?? []) })));
  return walk(raw as unknown as Raw[]);
}

export async function pdfTitle(p: PDFDocumentProxy, path: string): Promise<string> {
  const meta = await p.getMetadata().catch(() => null);
  const title = (meta?.info as { Title?: string } | undefined)?.Title?.trim();
  return title || path.split("/").pop()!.replace(/\.pdf$/i, "");
}

/** Render a page for OCR as base64 PNG: white background, longest side about 1800 px. */
export async function pageImage(p: PDFDocumentProxy, n: number): Promise<string> {
  const page = await p.getPage(n);
  const base = page.getViewport({ scale: 1 });
  const vp = page.getViewport({ scale: Math.min(3, 1792 / Math.max(base.width, base.height)) });
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(vp.width);
  canvas.height = Math.round(vp.height);
  const ctx = canvas.getContext("2d")!;
  // A transparent background reads as black to the model, and it returns nothing.
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvas, viewport: vp, background: "#ffffff" }).promise;
  return canvas.toDataURL("image/png").split(",")[1];
}
