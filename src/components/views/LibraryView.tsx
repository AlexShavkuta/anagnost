import { FileText, FolderOpen, Library } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { EmptyState, IconButton, ViewHeader } from "../workbench/common";

export const tildePath = (p: string) => p.replace(/^\/Users\/[^/]+/, "~");

export function LibraryView() {
  const recent = useApp((s) => s.recent);
  const docId = useApp((s) => s.doc?.id);
  const { openPath, choosePdf } = useApp.getState();
  return (
    <div className="flex h-full flex-col">
      <ViewHeader title="Library">
        <IconButton label="Open PDF…" shortcut="⌘O" onClick={choosePdf}>
          <FolderOpen />
        </IconButton>
      </ViewHeader>
      {recent.length === 0 && (
        <EmptyState icon={<Library />} title="Library is empty" action={<Button size="xs" variant="outline" onClick={choosePdf}>Open PDF…</Button>}>
          PDFs you open appear here.
        </EmptyState>
      )}
      <ul className="min-h-0 flex-1 overflow-auto pb-3">
        {recent.map((d) => (
          <li key={d.id}>
            <button
              title={d.path}
              onClick={() => openPath(d.path)}
              className={cn("grid w-full grid-cols-[16px_1fr] gap-x-2 px-4 py-1.5 text-left hover:bg-accent/70", d.id === docId && "bg-selected")}
            >
              <FileText className="row-span-2 mt-0.5 size-4 text-muted-foreground" />
              <span className="truncate">{d.title}</span>
              <span className="truncate text-[11px] text-muted-foreground [direction:rtl]">
                <bdi>{tildePath(d.path)}</bdi>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
