// Imperative pdf.js page rendering: placeholders for every page, canvases and text layers
// rendered lazily as pages come near the viewport.

import { pdfjs, type PDFDocumentProxy } from "./pdf";

export class PageRenderer {
  private rendered = new Map<number, { scale: number; task?: { cancel(): void } }>();
  private observer: IntersectionObserver;

  constructor(
    private root: HTMLElement,
    private container: HTMLElement,
  ) {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) this.renderPage(Number((e.target as HTMLElement).dataset.page));
      },
      { root, rootMargin: "800px 0px" },
    );
  }

  private pdf: PDFDocumentProxy | null = null;
  private scale = 1;

  async layout(pdf: PDFDocumentProxy, scale: number) {
    this.pdf = pdf;
    this.scale = scale;
    this.observer.disconnect();
    for (const r of this.rendered.values()) r.task?.cancel();
    this.rendered.clear();
    this.container.replaceChildren();
    // Size every placeholder from page 1; each page corrects its own size when it renders.
    const vp = (await pdf.getPage(1)).getViewport({ scale });
    if (this.pdf !== pdf || this.scale !== scale) return;
    const frag = document.createDocumentFragment();
    for (let i = 1; i <= pdf.numPages; i++) {
      const div = document.createElement("div");
      div.className = "page";
      div.dataset.page = String(i);
      div.style.width = `${vp.width}px`;
      div.style.height = `${vp.height}px`;
      frag.appendChild(div);
    }
    this.container.appendChild(frag);
    for (const div of this.container.children) this.observer.observe(div);
  }

  pageElement(n: number) {
    return this.container.querySelector<HTMLElement>(`.page[data-page="${n}"]`);
  }

  /** The page under the top third of the viewport. */
  visiblePage(): number | null {
    const mid = this.root.getBoundingClientRect().top + this.root.clientHeight / 3;
    for (const div of this.container.children as HTMLCollectionOf<HTMLElement>) {
      const r = div.getBoundingClientRect();
      if (r.top <= mid && r.bottom >= mid) return Number(div.dataset.page);
    }
    return null;
  }

  private async renderPage(n: number) {
    const pdf = this.pdf;
    if (!pdf) return;
    const scale = this.scale;
    const prev = this.rendered.get(n);
    if (prev && prev.scale === scale) return;
    prev?.task?.cancel();
    const entry: { scale: number; task?: { cancel(): void } } = { scale };
    this.rendered.set(n, entry);

    const div = this.pageElement(n);
    if (!div) return;
    const page = await pdf.getPage(n);
    const vp = page.getViewport({ scale });
    const ratio = window.devicePixelRatio || 1;
    const canvas = document.createElement("canvas");
    canvas.width = Math.floor(vp.width * ratio);
    canvas.height = Math.floor(vp.height * ratio);
    canvas.style.width = `${vp.width}px`;
    canvas.style.height = `${vp.height}px`;
    const textDiv = document.createElement("div");
    textDiv.className = "textLayer";
    div.style.width = `${vp.width}px`;
    div.style.height = `${vp.height}px`;
    div.style.setProperty("--scale-factor", String(scale));

    const task = page.render({ canvas, viewport: vp, transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined });
    entry.task = task;
    try {
      await task.promise;
    } catch {
      return; // cancelled by a zoom change or a new document
    }
    if (this.rendered.get(n) !== entry) return;
    div.replaceChildren(canvas, textDiv);
    await new pdfjs.TextLayer({ textContentSource: await page.getTextContent(), container: textDiv, viewport: vp }).render();
  }

  destroy() {
    this.observer.disconnect();
    for (const r of this.rendered.values()) r.task?.cancel();
  }
}
