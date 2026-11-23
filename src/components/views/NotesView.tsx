import { NotebookPen, Trash2 } from "lucide-react";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { renderMarkdown } from "@/lib/markdown";
import { useApp } from "@/lib/store";
import { EmptyState, IconButton, ViewHeader } from "../workbench/common";

const source: Record<string, { label: string; variant: "secondary" | "info" | "warning" }> = {
  user: { label: "You", variant: "secondary" },
  "claude-app": { label: "Tutor", variant: "info" },
  "claude-code": { label: "Agent", variant: "warning" },
};

export function NotesView() {
  const doc = useApp((s) => s.doc);
  const notes = useApp((s) => s.notes);
  const page = useApp((s) => s.currentPage);
  const { addNote, deleteNote, goToPage } = useApp.getState();
  const [text, setText] = useState("");

  const submit = async () => {
    if (!text.trim()) return;
    await addNote(text);
    setText("");
  };

  return (
    <div className="flex h-full flex-col">
      <ViewHeader title="Notes" />
      {!doc ? (
        <EmptyState icon={<NotebookPen />} title="No document">Open a PDF to take notes on it.</EmptyState>
      ) : (
        <>
          <form
            className="px-3 pb-3"
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            <Textarea
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && e.metaKey) {
                  e.preventDefault();
                  submit();
                }
              }}
              placeholder={`Note for page ${page}  ·  ⌘↵ to add`}
              className="text-[13px] sm:text-[13px] [&_textarea]:min-h-16 [&_textarea]:resize-none"
            />
            <div className="mt-1.5 flex justify-end">
              <Button type="submit" size="xs" variant="outline" disabled={!text.trim()}>
                Add note
              </Button>
            </div>
          </form>
          {notes.length === 0 && (
            <EmptyState icon={<NotebookPen />} title="No notes yet">
              Write one above, save a tutor answer, or select text in the PDF and add it from the tutor.
            </EmptyState>
          )}
          <ul className="min-h-0 flex-1 overflow-auto pb-3">
            {notes.map((n) => {
              const src = source[n.source] ?? { label: n.source, variant: "secondary" as const };
              return (
                <li key={n.id} className="group border-t px-4 py-2">
                  <div className="mb-1 flex items-center gap-2 text-[11px] text-muted-foreground">
                    <Badge variant={src.variant} size="sm">{src.label}</Badge>
                    {n.page && (
                      <button className="tabular-nums hover:text-primary hover:underline" onClick={() => goToPage(n.page!)}>
                        p. {n.page}
                      </button>
                    )}
                    <IconButton label="Delete note" className="ml-auto size-5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100" onClick={() => deleteNote(n.id)}>
                      <Trash2 className="size-3.5" />
                    </IconButton>
                  </div>
                  <div className="md text-xs leading-relaxed select-text" dangerouslySetInnerHTML={{ __html: renderMarkdown(n.text) }} />
                </li>
              );
            })}
          </ul>
        </>
      )}
    </div>
  );
}
