import { ChevronDown, ChevronRight, ChevronUp, FileText, FolderOpen, ZoomIn, ZoomOut } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Group, GroupSeparator } from "@/components/ui/group";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { Toolbar, ToolbarButton, ToolbarGroup, ToolbarInput, ToolbarSeparator } from "@/components/ui/toolbar";
import { Tooltip, TooltipPopup, TooltipTrigger } from "@/components/ui/tooltip";
import { ancestors, currentSection, useApp } from "@/lib/store";
import { PageRenderer } from "@/lib/viewer";
import { useZoom } from "@/lib/zoom";
import { tildePath } from "../views/LibraryView";

/** A toolbar button with a tooltip; Base UI composes Toolbar.Button, Tooltip.Trigger and our Button via `render`. */
function ToolButton({ label, shortcut, onClick, children }: { label: string; shortcut?: string; onClick(): void; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={<ToolbarButton render={<Button variant="outline" size="icon-xs" aria-label={label} onClick={onClick} className="sm:[&_svg:not([class*='size-'])]:size-3.5" />} />}
      >
        {children}
      </TooltipTrigger>
      <TooltipPopup side="bottom">
        <span className="flex items-center gap-2">
          {label}
          {shortcut && <Kbd className="h-4 text-[10px]">{shortcut}</Kbd>}
        </span>
      </TooltipPopup>
    </Tooltip>
  );
}

function PageField() {
  const page = useApp((s) => s.currentPage);
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <ToolbarInput
      aria-label="Page"
      inputMode="numeric"
      value={draft ?? String(page)}
      onFocus={(e) => {
        setDraft(String(page));
        e.target.select();
      }}
      onChange={(e) => setDraft(e.target.value.replace(/\D/g, ""))}
      onBlur={() => setDraft(null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          useApp.getState().goToPage(Number(draft) || page);
          (e.target as HTMLInputElement).blur();
        } else if (e.key === "Escape") (e.target as HTMLInputElement).blur();
      }}
      className="relative z-1 h-6 w-10 border-y border-input bg-background px-1.5 text-right text-xs text-foreground tabular-nums outline-none focus:border-ring dark:bg-input/32"
    />
  );
}

function EditorHeader() {
  const doc = useApp((s) => s.doc)!;
  const page = useApp((s) => s.currentPage);
  const total = useApp((s) => s.pdf?.numPages ?? 0);
  const { goToPage } = useApp.getState();
  const scale = useZoom((s) => s.scale);
  const { zoomBy, fitWidth } = useZoom.getState();
  return (
    <div className="@container flex h-[35px] shrink-0 items-stretch justify-between border-b bg-chrome">
      <div title={doc.path} className="-mb-px flex max-w-[60%] min-w-24 items-center gap-1.5 border-r bg-background pr-3.5 pl-3 shadow-[inset_0_1px_var(--primary)]">
        <FileText className="size-4 shrink-0 text-primary" />
        <span className="truncate">{doc.title}</span>
      </div>
      <Toolbar aria-label="Page and zoom" className="shrink-0 items-center gap-2 rounded-none border-0 bg-transparent px-2 py-0">
        <ToolbarGroup className="gap-1.5">
          <Group aria-label="Page">
            <ToolButton label="Previous page" onClick={() => goToPage(page - 1)}>
              <ChevronUp />
            </ToolButton>
            <PageField />
            <ToolButton label="Next page" onClick={() => goToPage(page + 1)}>
              <ChevronDown />
            </ToolButton>
          </Group>
          <span className="text-xs text-muted-foreground tabular-nums">of {total}</span>
        </ToolbarGroup>
        {/* Narrow editor: keep page navigation, drop the zoom buttons (⌘= ⌘− ⌘0 still work). */}
        <ToolbarSeparator orientation="vertical" className="hidden h-4 @[30rem]:block" />
        <Group aria-label="Zoom" className="hidden @[30rem]:flex">
          <ToolButton label="Zoom out" shortcut="⌘−" onClick={() => zoomBy(1 / 1.15)}>
            <ZoomOut />
          </ToolButton>
          <GroupSeparator />
          <Tooltip>
            <TooltipTrigger
              render={
                <ToolbarButton
                  render={<Button variant="outline" size="xs" onClick={fitWidth} className="w-12 px-0 font-normal text-muted-foreground tabular-nums" />}
                />
              }
            >
              {Math.round(scale * 100)}%
            </TooltipTrigger>
            <TooltipPopup side="bottom">
              <span className="flex items-center gap-2">
                Fit width <Kbd className="h-4 text-[10px]">⌘0</Kbd>
              </span>
            </TooltipPopup>
          </Tooltip>
          <GroupSeparator />
          <ToolButton label="Zoom in" shortcut="⌘=" onClick={() => zoomBy(1.15)}>
            <ZoomIn />
          </ToolButton>
        </Group>
      </Toolbar>
    </div>
  );
}

function Breadcrumbs() {
  const doc = useApp((s) => s.doc)!;
  const sections = useApp((s) => s.sections);
  const page = useApp((s) => s.currentPage);
  const ts = useApp((s) => s.textStatus);
  const { goToPage } = useApp.getState();
  const chain = ancestors(sections, currentSection(sections, page));
  const crumbs = [{ title: doc.title, page: 1, id: "doc" }, ...chain.map((s) => ({ title: s.title, page: s.start_page, id: String(s.id) }))];
  const tag = ts.recognized.includes(page)
    ? { text: "Recognized text", title: "This page's text came from Qianfan-OCR" }
    : ts.missing.includes(page)
      ? { text: "No text layer", title: "The tutor can't read this page until its text is recognized" }
      : null;
  return (
    <nav aria-label="Location" className="flex h-[22px] shrink-0 items-center gap-0.5 overflow-hidden border-b bg-background px-3 text-xs whitespace-nowrap text-muted-foreground">
      {crumbs.map((c, i) => (
        <span key={c.id} className="flex min-w-0 items-center gap-0.5">
          {i > 0 && <ChevronRight className="size-3.5 shrink-0 opacity-70" />}
          <button onClick={() => goToPage(c.page)} className="truncate rounded-sm px-1 hover:bg-accent hover:text-foreground">
            {c.title}
          </button>
        </span>
      ))}
      {tag && (
        <Badge title={tag.title} variant={tag.text === "No text layer" ? "warning" : "info"} size="sm" className="ml-auto">
          {tag.text}
        </Badge>
      )}
    </nav>
  );
}

function Viewer() {
  const pdf = useApp((s) => s.pdf);
  const scrollTo = useApp((s) => s.scrollTo);
  const scale = useZoom((s) => s.scale);
  const fit = useZoom((s) => s.fit);
  const root = useRef<HTMLDivElement>(null);
  const pages = useRef<HTMLDivElement>(null);
  const renderer = useRef<PageRenderer | null>(null);
  const [baseWidth, setBaseWidth] = useState<number | null>(null);
  const [laidOut, setLaidOut] = useState(0);

  useEffect(() => {
    renderer.current = new PageRenderer(root.current!, pages.current!);
    return () => renderer.current?.destroy();
  }, []);

  useEffect(() => {
    setBaseWidth(null);
    pdf?.getPage(1).then((p) => setBaseWidth(p.getViewport({ scale: 1 }).width));
  }, [pdf]);

  // Fit to width: follow the viewer's size as panels open, close and resize.
  useEffect(() => {
    if (!fit || !baseWidth) return;
    const el = root.current!;
    // Fit the width, but past ~1000px a page stops being readable as a whole, so stop there.
    const apply = () => useZoom.getState().setScale(Math.min(el.clientWidth - 64, 1000) / baseWidth, true);
    apply();
    let t: number | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(t);
      t = window.setTimeout(apply, 120);
    });
    ro.observe(el);
    return () => {
      clearTimeout(t);
      ro.disconnect();
    };
  }, [fit, baseWidth]);

  // Re-lay out on a new document or zoom, keeping the reader's page.
  useLayoutEffect(() => {
    if (!pdf || !baseWidth) return;
    const page = useApp.getState().currentPage;
    let live = true;
    renderer.current!.layout(pdf, scale).then(() => {
      if (!live) return;
      renderer.current!.show(page);
      setLaidOut((n) => n + 1);
    });
    return () => {
      live = false;
    };
  }, [pdf, scale, baseWidth]);

  useEffect(() => {
    if (!scrollTo || !laidOut) return;
    // Jump straight there: animating through the pages in between queues their renders first.
    renderer.current?.show(scrollTo.page);
  }, [scrollTo?.seq, laidOut]);

  // Track the page in view and the text selected in the PDF.
  useEffect(() => {
    const el = root.current!;
    let frame = 0;
    const onScroll = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        const p = renderer.current?.visiblePage();
        if (p) {
          renderer.current!.setFocus(p);
          useApp.getState().setCurrentPage(p);
        }
      });
    };
    const onSelection = () => {
      const sel = window.getSelection();
      if (!sel?.anchorNode) return;
      if (!pages.current!.contains(sel.anchorNode)) {
        // Clicking elsewhere (e.g. into the tutor input) keeps the PDF selection.
        return;
      }
      useApp.getState().setSelection(sel.toString().replace(/\s+/g, " ").trim());
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    document.addEventListener("selectionchange", onSelection);
    return () => {
      cancelAnimationFrame(frame);
      el.removeEventListener("scroll", onScroll);
      document.removeEventListener("selectionchange", onSelection);
    };
  }, []);

  return (
    <div ref={root} className="relative min-h-0 flex-1 overflow-auto bg-viewer">
      <div ref={pages} className="pdfViewer" />
    </div>
  );
}

function Welcome() {
  const recent = useApp((s) => s.recent);
  const { choosePdf, openPath } = useApp.getState();
  const row = "flex h-7 items-center justify-between gap-3";
  const link = "flex min-w-0 items-center gap-2 text-primary hover:underline";
  return (
    <div className="absolute inset-0 flex justify-center overflow-auto bg-background">
      <div className="w-[min(460px,100%-48px)] pt-[14vh] pb-12">
        <h1 className="text-[26px] font-semibold tracking-tight">PDF Tutor</h1>
        <p className="mt-1 mb-7 text-sm text-muted-foreground">Read a PDF with a tutor that knows what's on the page.</p>
        <h3 className="mt-6 mb-1.5 text-[11px] font-semibold tracking-[0.06em] text-muted-foreground uppercase">Start</h3>
        <div className={row}>
          <button className={link} onClick={choosePdf}>
            <FolderOpen className="size-4" />
            Open PDF…
          </button>
          <Kbd>⌘O</Kbd>
        </div>
        {recent.length > 0 && (
          <>
            <h3 className="mt-6 mb-1.5 text-[11px] font-semibold tracking-[0.06em] text-muted-foreground uppercase">Recent</h3>
            {recent.slice(0, 6).map((d) => (
              <div key={d.id} className={row}>
                <button className={link} onClick={() => openPath(d.path)} title={d.path}>
                  <FileText className="size-4 shrink-0" />
                  <span className="truncate">{d.title}</span>
                </button>
                <span className="max-w-[45%] truncate text-xs text-muted-foreground [direction:rtl]">
                  <bdi>{tildePath(d.path).replace(/\/[^/]+$/, "")}</bdi>
                </span>
              </div>
            ))}
          </>
        )}
        <h3 className="mt-6 mb-1.5 text-[11px] font-semibold tracking-[0.06em] text-muted-foreground uppercase">Keys</h3>
        <dl className="grid grid-cols-[1fr_auto] gap-y-1.5 text-muted-foreground">
          <dt>Command palette</dt>
          <dd><Kbd>⌘K</Kbd></dd>
          <dt>Search the document</dt>
          <dd><Kbd>⌘F</Kbd></dd>
          <dt>Ask the tutor</dt>
          <dd><Kbd>⌘L</Kbd></dd>
          <dt>Toggle side bar / tutor</dt>
          <dd><KbdGroup><Kbd>⌘B</Kbd><Kbd>⌥⌘B</Kbd></KbdGroup></dd>
        </dl>
      </div>
    </div>
  );
}

export function Editor() {
  const doc = useApp((s) => s.doc);
  return (
    <main className="relative flex h-full min-w-0 flex-col bg-background">
      {doc && <EditorHeader />}
      {doc && <Breadcrumbs />}
      <Viewer />
      {!doc && <Welcome />}
    </main>
  );
}
