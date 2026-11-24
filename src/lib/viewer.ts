// Imperative pdf.js page rendering: a sized placeholder for every page; canvases and text
// layers are drawn by one scheduler, one page at a time, nearest to the reader's page first.
//
// Why a scheduler: pdf.js paints on the main thread. Starting a render for every page that
// comes near the viewport (or every page you click through quickly) makes them all compete,
// so the page you want waits behind ones you've already left, and the UI stutters.

import { pdfjs, type PDFDocumentProxy } from "./pdf";

type State = { scale: number; canvas: boolean; text: boolean };

/** How far outside the viewport pages are drawn ahead, and how far before they're freed. */
const NEAR_MARGIN = "1000px 0px";
const KEEP_MARGIN = "3500px 0px";
/** Cap each canvas (in device pixels) so huge or zoomed pages stay quick to paint. */
const MAX_CANVAS_PIXELS = 12_000_000;

export class PageRenderer {
  private pdf: PDFDocumentProxy | null = null;
  private scale = 1;
  private divs: HTMLElement[] = [];
  private states = new Map<number, State>();
  /** Pages in the drawing zone around the viewport. */
  private near = new Set<number>();
  /** The page the reader is on or jumping to; work closest to it goes first. */
  private focus = 1;
  private busy: { page: number; cancel(): void } | null = null;
  private nearObs: IntersectionObserver;
  private farObs: IntersectionObserver;

  constructor(
    private root: HTMLElement,
    private container: HTMLElement,
  ) {
    this.nearObs = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const n = Number((e.target as HTMLElement).dataset.page);
          if (e.isIntersecting) this.near.add(n);
          else this.near.delete(n);
        }
        this.pump();
      },
      { root, rootMargin: NEAR_MARGIN },
    );
    this.farObs = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (!e.isIntersecting) this.release(Number((e.target as HTMLElement).dataset.page));
      },
      { root, rootMargin: KEEP_MARGIN },
    );
  }

  async layout(pdf: PDFDocumentProxy, scale: number) {
    this.pdf = pdf;
    this.scale = scale;
    // Every page's real size up front, so nothing shifts when pages draw.
    const sizes = await Promise.all(
      Array.from({ length: pdf.numPages }, (_, i) => pdf.getPage(i + 1).then((p) => p.getViewport({ scale }))),
    );
    if (this.pdf !== pdf || this.scale !== scale) return;
    this.nearObs.disconnect();
    this.farObs.disconnect();
    this.busy?.cancel();
    this.busy = null;
    this.states.clear();
    this.near.clear();
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
      this.nearObs.observe(div);
      this.farObs.observe(div);
    }
  }

  pageElement(n: number): HTMLElement | null {
    return this.divs[n - 1] ?? null;
  }

  /** Jump to a page without passing through the ones in between; it's drawn first. */
  show(n: number) {
    const div = this.pageElement(n);
    if (!div) return;
    this.near.add(n); // before the observer catches up
    this.root.scrollTop = div.offsetTop - 12;
    this.setFocus(n);
  }

  /** Called as the reader scrolls. */
  setFocus(n: number) {
    this.focus = n;
    // Drop work for a page the reader has jumped away from.
    if (this.busy && Math.abs(this.busy.page - n) > 2) {
      this.busy.cancel();
      this.busy = null;
    }
    this.pump();
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

  /** The next job: canvases nearest the focus first, then text layers for the pages in view. */
  private next(): { page: number; kind: "canvas" | "text" } | null {
    let best: { page: number; kind: "canvas" | "text"; cost: number } | null = null;
    for (const n of this.near) {
      const st = this.states.get(n);
      const d = Math.abs(n - this.focus);
      let job: { kind: "canvas" | "text"; cost: number } | null = null;
      if (!st || st.scale !== this.scale || !st.canvas) job = { kind: "canvas", cost: d };
      else if (!st.text && d <= 1) job = { kind: "text", cost: 2.5 + d };
      if (job && (!best || job.cost < best.cost)) best = { page: n, ...job };
    }
    return best;
  }

  private pump() {
    if (this.busy || !this.pdf) return;
    const job = this.next();
    if (!job) return;
    const run = job.kind === "canvas" ? this.drawCanvas(job.page) : this.drawText(job.page);
    const mine = this.busy;
    run.finally(() => {
      if (this.busy === mine) this.busy = null;
      // Yield a frame between pages so scrolling and clicks stay responsive.
      requestAnimationFrame(() => this.pump());
    });
  }

  private async drawCanvas(n: number) {
    const pdf = this.pdf!;
    const scale = this.scale;
    let cancelled = false;
    let task: { cancel(): void } | null = null;
    this.busy = { page: n, cancel: () => ((cancelled = true), task?.cancel()) };

    const page = await pdf.getPage(n);
    if (cancelled || pdf !== this.pdf) return;
    const vp = page.getViewport({ scale });
    const ratio = Math.min(window.devicePixelRatio || 1, Math.sqrt(MAX_CANVAS_PIXELS / (vp.width * vp.height)));
    const canvas = document.createElement("canvas");
    canvas.width = Math.floor(vp.width * ratio);
    canvas.height = Math.floor(vp.height * ratio);
    canvas.style.width = `${vp.width}px`;
    canvas.style.height = `${vp.height}px`;
    const render = page.render({ canvas, viewport: vp, transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined });
    task = render;
    try {
      await render.promise;
    } catch {
      canvas.width = canvas.height = 0;
      return; // cancelled: jumped elsewhere, zoomed, or a new document
    }
    if (cancelled || pdf !== this.pdf || scale !== this.scale) return;
    const div = this.pageElement(n)!;
    div.style.setProperty("--scale-factor", String(scale));
    const old = div.querySelector("canvas");
    if (old) old.width = old.height = 0;
    div.replaceChildren(canvas);
    this.states.set(n, { scale, canvas: true, text: false });
  }

  private async drawText(n: number) {
    const pdf = this.pdf!;
    const scale = this.scale;
    let cancelled = false;
    let layer: { cancel(): void } | null = null;
    this.busy = { page: n, cancel: () => ((cancelled = true), layer?.cancel()) };
    const st = this.states.get(n);
    if (st) st.text = true; // don't retry a failed one in a loop
    const page = await pdf.getPage(n);
    const content = await page.getTextContent();
    if (cancelled || pdf !== this.pdf || scale !== this.scale) {
      if (st) st.text = false;
      return;
    }
    const div = this.pageElement(n)!;
    const textDiv = document.createElement("div");
    textDiv.className = "textLayer";
    const tl = new pdfjs.TextLayer({ textContentSource: content, container: textDiv, viewport: page.getViewport({ scale }) });
    layer = tl;
    try {
      await tl.render();
    } catch {
      if (st) st.text = false;
      return;
    }
    if (cancelled) {
      if (st) st.text = false;
      return;
    }
    div.querySelector(".textLayer")?.remove();
    div.appendChild(textDiv);
  }

  /** Free a far-away page; it draws again when it comes back. */
  private release(n: number) {
    if (this.busy?.page === n) {
      this.busy.cancel();
      this.busy = null;
    }
    if (!this.states.delete(n)) return;
    const div = this.pageElement(n);
    const canvas = div?.querySelector("canvas");
    if (canvas) canvas.width = canvas.height = 0;
    div?.replaceChildren();
  }

  destroy() {
    this.nearObs.disconnect();
    this.farObs.disconnect();
    this.busy?.cancel();
  }
}
