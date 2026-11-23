// Imperative pdf.js page rendering: a sized placeholder for every page, canvases and text
// layers rendered as pages come near the viewport and dropped when they're far away.

import { pdfjs, type PDFDocumentProxy } from "./pdf";

type Entry = { scale: number; done: boolean; task?: { cancel(): void } };

/** How far outside the viewport pages are rendered ahead, and how far before they're freed. */
const RENDER_MARGIN = "1200px 0px";
const KEEP_MARGIN = "4000px 0px";

export class PageRenderer {
  private rendered = new Map<number, Entry>();
  private near: IntersectionObserver;
  private far: IntersectionObserver;
  private pdf: PDFDocumentProxy | null = null;
  private scale = 1;
  private divs: HTMLElement[] = [];

  constructor(
    private root: HTMLElement,
    private container: HTMLElement,
  ) {
    this.near = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const n = Number((e.target as HTMLElement).dataset.page);
          if (e.isIntersecting) this.renderPage(n);
          else this.cancelPending(n); // scrolled past before it finished: don't hold up the queue
        }
      },
      { root, rootMargin: RENDER_MARGIN },
    );
    this.far = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (!e.isIntersecting) this.release(Number((e.target as HTMLElement).dataset.page));
      },
      { root, rootMargin: KEEP_MARGIN },
    );
  }

  async layout(pdf: PDFDocumentProxy, scale: number) {
    this.pdf = pdf;
    this.scale = scale;
    // Every page's real size up front, so nothing shifts when pages render.
    const sizes = await Promise.all(
      Array.from({ length: pdf.numPages }, (_, i) => pdf.getPage(i + 1).then((p) => p.getViewport({ scale }))),
    );
    if (this.pdf !== pdf || this.scale !== scale) return;
    this.near.disconnect();
    this.far.disconnect();
    for (const r of this.rendered.values()) r.task?.cancel();
    this.rendered.clear();
    const frag = document.createDocumentFragment();
    this.divs = sizes.map((vp, i) => {
      const div = document.createElement("div");
      div.className = "page";
      div.dataset.page = String(i + 1);
      div.style.width = `${vp.width}px`;
      div.style.height = `${vp.height}px`;
      frag.appendChild(div);
      return div;
    });
    this.container.replaceChildren(frag);
    for (const div of this.divs) {
      this.near.observe(div);
      this.far.observe(div);
    }
  }

  pageElement(n: number): HTMLElement | null {
    return this.divs[n - 1] ?? null;
  }

  /** Jump to a page without animating through the pages in between; render it first. */
  show(n: number) {
    const div = this.pageElement(n);
    if (!div) return;
    this.renderPage(n);
    this.root.scrollTop = div.offsetTop - 12;
  }

  /** The page under the top third of the viewport (binary search; pages are in order). */
  visiblePage(): number | null {
    if (this.divs.length === 0) return null;
    const y = this.root.scrollTop + this.root.clientHeight / 3;
    let lo = 0;
    let hi = this.divs.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.divs[mid].offsetTop <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  }

  private cancelPending(n: number) {
    const r = this.rendered.get(n);
    if (r && !r.done) {
      r.task?.cancel();
      this.rendered.delete(n);
    }
  }

  /** Free a far-away page's canvas; it renders again when it comes back. */
  private release(n: number) {
    const r = this.rendered.get(n);
    if (!r) return;
    r.task?.cancel();
    this.rendered.delete(n);
    this.pageElement(n)?.replaceChildren();
  }

  private async renderPage(n: number) {
    const pdf = this.pdf;
    if (!pdf) return;
    const scale = this.scale;
    const prev = this.rendered.get(n);
    if (prev && prev.scale === scale) return;
    prev?.task?.cancel();
    const entry: Entry = { scale, done: false };
    this.rendered.set(n, entry);

    const div = this.pageElement(n);
    if (!div) return;
    const page = await pdf.getPage(n);
    if (this.rendered.get(n) !== entry) return;
    const vp = page.getViewport({ scale });
    const ratio = window.devicePixelRatio || 1;
    const canvas = document.createElement("canvas");
    canvas.width = Math.floor(vp.width * ratio);
    canvas.height = Math.floor(vp.height * ratio);
    canvas.style.width = `${vp.width}px`;
    canvas.style.height = `${vp.height}px`;
    const textDiv = document.createElement("div");
    textDiv.className = "textLayer";
    div.style.setProperty("--scale-factor", String(scale));

    const task = page.render({ canvas, viewport: vp, transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined });
    entry.task = task;
    try {
      await task.promise;
    } catch {
      return; // cancelled: zoom change, new document, or scrolled away
    }
    if (this.rendered.get(n) !== entry) return;
    entry.done = true;
    div.replaceChildren(canvas, textDiv);
    await new pdfjs.TextLayer({ textContentSource: await page.getTextContent(), container: textDiv, viewport: vp }).render();
  }

  destroy() {
    this.near.disconnect();
    this.far.disconnect();
    for (const r of this.rendered.values()) r.task?.cancel();
  }
}
