import { ChevronRight, Circle, CircleCheck, FileText, ListCollapse, ScanText, Sparkles } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsiblePanel, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Meter, MeterIndicator, MeterTrack } from "@/components/ui/meter";
import { Progress, ProgressIndicator, ProgressTrack } from "@/components/ui/progress";
import type { Section } from "@/lib/api";
import { renderMarkdown } from "@/lib/markdown";
import { currentSection, useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { EmptyState, IconButton, ViewHeader } from "../workbench/common";

/** A collapsible pane inside a view, like VS Code's Explorer sections. */
function Pane({ title, meta, open, onOpenChange, className, children }: {
  title: string; meta?: ReactNode; open: boolean; onOpenChange(o: boolean): void; className?: string; children: ReactNode;
}) {
  return (
    <Collapsible open={open} onOpenChange={onOpenChange} className={cn("flex min-h-0 flex-col border-t", open ? className : "flex-none")}>
      <CollapsibleTrigger className="flex h-[22px] w-full shrink-0 items-center gap-0.5 pr-2 pl-1 text-left text-[11px] font-bold tracking-[0.04em] uppercase outline-none hover:bg-accent/60 focus-visible:bg-accent">
        <ChevronRight className={cn("size-4 transition-transform", open && "rotate-90")} />
        {title}
        {meta && <span className="ml-auto flex items-center gap-2 font-normal tracking-normal text-muted-foreground normal-case tabular-nums">{meta}</span>}
      </CollapsibleTrigger>
      {/* The panel scrolls inside the flex layout, so it can't use coss's animated height. */}
      <CollapsiblePanel className="h-auto! min-h-0 flex-1 overflow-auto">{children}</CollapsiblePanel>
    </Collapsible>
  );
}

function OcrBanner() {
  const missing = useApp((s) => s.textStatus.missing.length);
  const pages = useApp((s) => s.pdf?.numPages ?? 0);
  const running = useApp((s) => !!s.ocrRun);
  if (!missing || running) return null;
  const all = missing === pages;
  return (
    <div className="relative mx-3 mb-3 flex gap-2.5 rounded-lg border bg-card p-3 text-xs leading-relaxed shadow-xs/5 dark:bg-input/32">
      <ScanText className="mt-0.5 size-4 shrink-0 text-primary" />
      <div>
        <p className="mb-2">
          {all
            ? `This PDF is scanned: none of its ${missing} pages has text.`
            : `${missing} ${missing === 1 ? "page has" : "pages have"} no text layer, likely scanned.`}{" "}
          Recognize {missing === 1 ? "it" : "them"} on this Mac so the tutor, map and search can read {missing === 1 ? "it" : "them"}.
        </p>
        <Button size="xs" variant="outline" onClick={() => useApp.getState().recognize()}>
          Recognize {missing} {missing === 1 ? "page" : "pages"}
        </Button>
      </div>
    </div>
  );
}

function Tree({ byParent, parent, depth, current }: {
  byParent: Map<number | null, Section[]>; parent: number | null; depth: number; current: number | undefined;
}) {
  const expanded = useApp((s) => s.expanded);
  const { goToPage, toggleExpanded, toggleStudied } = useApp.getState();
  return (
    <ul role={depth ? "group" : "tree"} aria-label={depth ? undefined : "Sections"}>
      {(byParent.get(parent) ?? []).map((s) => {
        const kids = byParent.has(s.id);
        const open = expanded.has(s.id);
        const isCurrent = s.id === current;
        return (
          <li key={s.id} role="treeitem" aria-expanded={kids ? open : undefined} aria-selected={isCurrent}>
            <div
              data-current={isCurrent || undefined}
              onClick={() => goToPage(s.start_page)}
              title={s.summary ? s.title : `${s.title} (not mapped yet)`}
              style={{ paddingLeft: 4 + depth * 12 }}
              className={cn(
                "group relative flex h-[22px] items-center gap-1 pr-2 hover:bg-accent/70",
                isCurrent && "bg-selected before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-primary",
              )}
            >
              <span
                className="grid w-4 place-items-center"
                onClick={(e) => {
                  if (!kids) return;
                  e.stopPropagation();
                  toggleExpanded(s.id);
                }}
              >
                {kids && <ChevronRight className={cn("size-4 text-muted-foreground transition-transform", open && "rotate-90")} />}
              </span>
              <span className={cn("min-w-0 flex-1 truncate", !s.summary && "text-muted-foreground")}>{s.title}</span>
              <span className="text-[11px] text-muted-foreground tabular-nums">
                {s.start_page === s.end_page ? s.start_page : `${s.start_page}–${s.end_page}`}
              </span>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  toggleStudied(s.id);
                }}
                aria-pressed={s.studied}
                title={s.studied ? "Studied" : "Mark as studied"}
                className={cn("grid size-5 place-items-center rounded-sm hover:bg-accent", !s.studied && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100")}
              >
                {s.studied ? <CircleCheck className="size-3.5 text-success" /> : <Circle className="size-3.5 text-muted-foreground" />}
              </button>
            </div>
            {kids && open && <Tree byParent={byParent} parent={s.id} depth={depth + 1} current={current} />}
          </li>
        );
      })}
    </ul>
  );
}

function CurrentSection({ s }: { s: Section }) {
  const { toggleStudied, sendChat } = useApp.getState();
  return (
    <div className="px-4 pt-1 pb-4 select-text">
      <h3 className="font-semibold">{s.title}</h3>
      <div className="mt-0.5 mb-2 flex items-center gap-2 text-xs text-muted-foreground">
        {s.start_page === s.end_page ? `p. ${s.start_page}` : `pp. ${s.start_page}–${s.end_page}`}
        <button onClick={() => toggleStudied(s.id)} className={cn("hover:underline", s.studied ? "text-success" : "text-primary")}>
          {s.studied ? "Studied ✓" : "Mark as studied"}
        </button>
      </div>
      {!s.summary ? (
        <p className="text-xs leading-relaxed text-muted-foreground">
          No summary yet. Build the map to get one, with key concepts and questions to test yourself.
        </p>
      ) : (
        <>
          <p className="leading-relaxed">{s.summary}</p>
          {s.concepts.length > 0 && (
            <>
              <h4 className="mt-3 mb-1 text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">Key concepts</h4>
              <dl>
                {s.concepts.map((c) => (
                  <div key={c.term} className="mt-1.5">
                    <dt className="font-semibold">{c.term}</dt>
                    <dd className="leading-snug text-muted-foreground">{c.explanation}</dd>
                  </div>
                ))}
              </dl>
            </>
          )}
          {s.questions.length > 0 && (
            <>
              <h4 className="mt-3 mb-1 text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">Check yourself</h4>
              <ol className="list-decimal space-y-1 pl-4.5">
                {s.questions.map((q) => (
                  <li key={q}>
                    <button
                      title="Have the tutor quiz you on this"
                      className="block text-left leading-snug hover:text-primary hover:underline"
                      onClick={() =>
                        sendChat(`Quiz me on this question, one step at a time. Don't give the answer until I've tried:\n\n"${q}"`, s.id).catch(() => {})
                      }
                    >
                      {q}
                    </button>
                  </li>
                ))}
              </ol>
            </>
          )}
        </>
      )}
    </div>
  );
}

export function MapView() {
  const doc = useApp((s) => s.doc);
  const sections = useApp((s) => s.sections);
  const page = useApp((s) => s.currentPage);
  const mapRun = useApp((s) => s.mapRun);
  const { buildMap, collapseAll, choosePdf } = useApp.getState();
  const [open, setOpen] = useState({ overview: true, sections: true, current: true });
  const treeRef = useRef<HTMLDivElement>(null);

  const byParent = useMemo(() => {
    const m = new Map<number | null, Section[]>();
    for (const s of [...sections].sort((a, b) => a.ord - b.ord)) {
      m.set(s.parent_id, [...(m.get(s.parent_id) ?? []), s]);
    }
    return m;
  }, [sections]);
  const cur = currentSection(sections, page);

  useEffect(() => {
    treeRef.current?.querySelector("[data-current]")?.scrollIntoView({ block: "nearest" });
  }, [cur?.id]);

  const studied = sections.filter((s) => s.studied).length;
  const mapped = sections.filter((s) => s.summary).length;
  const complete = mapped === sections.length && sections.length > 0;

  return (
    <div className="flex h-full flex-col">
      <ViewHeader title="Map">
        <IconButton
          label={complete ? "Rebuild map with Claude" : mapped ? `Finish map with Claude (${mapped}/${sections.length})` : "Build map with Claude"}
          disabled={!doc || !!mapRun}
          onClick={() => buildMap()}
        >
          <Sparkles />
        </IconButton>
        <IconButton label="Collapse all" onClick={collapseAll} disabled={!sections.length}>
          <ListCollapse />
        </IconButton>
      </ViewHeader>
      {mapRun && (
        <Progress value={mapRun.total ? (100 * mapRun.done) / mapRun.total : null} className="shrink-0 gap-0">
          <ProgressTrack className="h-0.5 rounded-none bg-transparent">
            <ProgressIndicator />
          </ProgressTrack>
        </Progress>
      )}
      <OcrBanner />
      {!doc ? (
        <EmptyState
          icon={<FileText />}
          title="No document"
          action={<Button size="xs" variant="outline" onClick={choosePdf}>Open PDF…</Button>}
        >
          Open a PDF to see its sections.
        </EmptyState>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          {doc.overview && (
            <Pane title="Overview" open={open.overview} onOpenChange={(o) => setOpen({ ...open, overview: o })} className="max-h-[30%] flex-[0_1_auto]">
              <div className="md px-4 pt-0.5 pb-3 pl-6 text-xs leading-relaxed select-text" dangerouslySetInnerHTML={{ __html: renderMarkdown(doc.overview) }} />
            </Pane>
          )}
          <Pane
            title="Sections"
            meta={
              sections.length ? (
                <>
                  <Meter value={studied} max={sections.length} aria-label="Sections studied" className="w-12 gap-0">
                    <MeterTrack className="h-1 rounded-full">
                      <MeterIndicator className="bg-success" />
                    </MeterTrack>
                  </Meter>
                  {studied}/{sections.length}
                </>
              ) : undefined
            }
            open={open.sections}
            onOpenChange={(o) => setOpen({ ...open, sections: o })}
            className={cn(cur && open.current ? "max-h-[50%] min-h-[66px] flex-[0_1_auto]" : "flex-1")}
          >
            <div ref={treeRef} className="pb-2">
              <Tree byParent={byParent} parent={null} depth={0} current={cur?.id} />
            </div>
          </Pane>
          {cur && (
            <Pane title="Current section" open={open.current} onOpenChange={(o) => setOpen({ ...open, current: o })} className="flex-1">
              <CurrentSection s={cur} />
            </Pane>
          )}
        </div>
      )}
    </div>
  );
}
