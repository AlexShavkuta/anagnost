import { ArrowUp, BookOpenCheck, Check, CircleAlert, Copy, FileText, KeyRound, ListTree, NotebookPen, Plus, Quote, Sparkles, User } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Composer,
  ComposerActionGroup,
  ComposerActions,
  ComposerInput,
  ComposerItem,
  type ComposerItemValue,
  ComposerList,
  ComposerPopup,
  ComposerSubmit,
  type ComposerTrigger,
  type ComposerValue,
  getComposerText,
} from "@/components/ui/composer";
import { Spinner } from "@/components/ui/spinner";
import { providerProblem } from "@/lib/api";
import { renderMarkdown } from "@/lib/markdown";
import { currentSection, useApp } from "@/lib/store";
import { EmptyState, IconButton, ViewHeader } from "./common";

const QUIZ = "Quiz me on the section I'm reading. Ask one question at a time, wait for my answer, then give feedback.";

function Actions({ text }: { text: string }) {
  const [done, setDone] = useState<"note" | "copy" | null>(null);
  return (
    <div className="mt-1 flex gap-0.5 opacity-0 transition-opacity group-last:opacity-100 group-hover:opacity-100 focus-within:opacity-100">
      <IconButton label={done === "note" ? "Saved to notes" : "Save as note"} side="top" className="size-[22px]" onClick={() => useApp.getState().addNote(text, "claude-app").then(() => setDone("note"))}>
        {done === "note" ? <Check className="size-3.5" /> : <NotebookPen className="size-3.5" />}
      </IconButton>
      <IconButton label={done === "copy" ? "Copied" : "Copy"} side="top" className="size-[22px]" onClick={() => navigator.clipboard.writeText(text).then(() => setDone("copy"))}>
        {done === "copy" ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </IconButton>
    </div>
  );
}

function Message({ role, content, streaming }: { role: "user" | "assistant"; content: string; streaming?: boolean }) {
  return (
    <article className="group border-b px-4 pt-2.5 pb-3 last:border-b-0">
      <header className="mb-1 flex items-center gap-1.5 text-xs font-semibold">
        {role === "assistant" ? <Sparkles className="size-3.5 text-primary" /> : <User className="size-3.5 text-muted-foreground" />}
        {role === "assistant" ? "Tutor" : "You"}
      </header>
      {role === "user" ? (
        <div className="leading-relaxed whitespace-pre-wrap">{content}</div>
      ) : streaming && !content ? (
        <div className="flex items-center gap-1.5 text-muted-foreground">
          <Spinner className="size-3.5" /> Reading the section…
        </div>
      ) : (
        <div className="md leading-relaxed" dangerouslySetInnerHTML={{ __html: renderMarkdown(content) }} />
      )}
      {role === "assistant" && !streaming && <Actions text={content} />}
    </article>
  );
}

function Empty() {
  const settings = useApp((s) => s.settings);
  const doc = useApp((s) => s.doc);
  const sections = useApp((s) => s.sections);
  const page = useApp((s) => s.currentPage);
  const problem = providerProblem(settings);
  if (problem) {
    return (
      <EmptyState
        icon={<KeyRound />}
        title={problem.title}
        action={<Button size="sm" onClick={() => useApp.getState().openSettings(problem.focus)}>Open settings</Button>}
      >
        {problem.text}
      </EmptyState>
    );
  }
  if (!doc) {
    return (
      <EmptyState icon={<Sparkles />} title="Tutor">
        Open a PDF, then ask about whatever you're reading. Answers come from the section on screen and cite pages.
      </EmptyState>
    );
  }
  const s = currentSection(sections, page);
  return (
    <EmptyState icon={<Sparkles />} title={s?.title ?? `Page ${page}`}>
      Ask about this section, select a passage to have it explained, or type <kbd className="rounded-sm bg-muted px-1 font-mono">/</kbd> for actions.
    </EmptyState>
  );
}

/** What the tutor is reading, and the PDF selection with its quick actions. */
function Context() {
  const doc = useApp((s) => s.doc);
  const sections = useApp((s) => s.sections);
  const page = useApp((s) => s.currentPage);
  const selection = useApp((s) => s.selection);
  const busy = useApp((s) => s.streaming !== null);
  const [saved, setSaved] = useState(false);
  useEffect(() => setSaved(false), [selection]);
  if (!doc) return null;
  const s = currentSection(sections, page);
  const chip = "inline-flex h-[22px] max-w-full items-center gap-1 rounded-md border bg-background pr-1 pl-1.5 text-[11px] text-muted-foreground shadow-xs/5 dark:bg-input/32";
  return (
    <div className="mb-1.5 flex flex-wrap gap-1">
      <span className={chip} title="The tutor reads this section">
        <ListTree className="size-3.5 shrink-0" />
        <span className="truncate">{s ? s.title : `Page ${page}`} · p. {page}</span>
      </span>
      {selection && (
        <span className={`${chip} border-primary/60 text-foreground`} title={selection.slice(0, 400)}>
          <Quote className="size-3.5 shrink-0" />
          <span className="truncate">{selection.length > 44 ? `${selection.slice(0, 44)}…` : selection}</span>
          <Button
            size="xs"
            variant="ghost"
            disabled={busy}
            className="h-[18px] px-1 text-[11px] sm:h-[18px] sm:text-[11px]"
            onClick={() => useApp.getState().sendChat(`Explain this passage (page ${page}) in plain terms, and how it fits the section:\n\n"${selection}"`).catch(() => {})}
          >
            Explain
          </Button>
          <Button
            size="xs"
            variant="ghost"
            className="h-[18px] px-1 text-[11px] sm:h-[18px] sm:text-[11px]"
            onClick={() => useApp.getState().addNote(`> ${selection.replace(/\n/g, "\n> ")}`).then(() => setSaved(true))}
          >
            {saved ? "Saved" : "Note"}
          </Button>
        </span>
      )}
    </div>
  );
}

export function Tutor() {
  const doc = useApp((s) => s.doc);
  const chat = useApp((s) => s.chat);
  const streaming = useApp((s) => s.streaming);
  const error = useApp((s) => s.chatError);
  const focusSeq = useApp((s) => s.chatFocusSeq);
  const page = useApp((s) => s.currentPage);
  const selection = useApp((s) => s.selection);
  const { sendChat, clearChat, addNote } = useApp.getState();
  const [value, setValue] = useState<ComposerValue>([]);
  const list = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLElement>(null);
  const busy = streaming !== null;

  useEffect(() => {
    if (focusSeq) root.current?.querySelector<HTMLElement>('[data-slot="composer-input"]')?.focus();
  }, [focusSeq]);

  useLayoutEffect(() => {
    const el = list.current!;
    el.scrollTop = el.scrollHeight;
  }, [chat.length, streaming, error]);

  const send = async (t: string) => {
    if (!t.trim() || busy || !doc) return;
    setValue([]);
    try {
      await sendChat(t);
    } catch {
      setValue([{ type: "text", text: t }]); // put the question back so it can be retried
    }
  };

  // "/" opens the tutor's actions, coss Composer's slash-command trigger.
  const actions: ComposerItemValue[] = [
    { id: "explain-page", label: "Explain this page", description: `Walk through page ${page}`, type: "action", icon: <FileText /> },
    { id: "quiz", label: "Quiz me", description: "One question at a time on this section", type: "action", icon: <BookOpenCheck /> },
    { id: "explain-selection", label: "Explain selection", description: selection ? "The text selected in the PDF" : "Select text in the PDF first", type: "action", icon: <Quote />, disabled: !selection },
    { id: "note-selection", label: "Save selection as note", description: selection ? `To page ${page}` : "Select text in the PDF first", type: "action", icon: <NotebookPen />, disabled: !selection },
  ];
  const triggers: ComposerTrigger[] = [{ character: "/", items: actions }];
  const onAction = (item: ComposerItemValue) => {
    if (item.id === "explain-page") send(`Walk me through page ${page}: the main points and anything tricky.`);
    else if (item.id === "quiz") send(QUIZ);
    else if (item.id === "explain-selection") send(`Explain this passage (page ${page}) in plain terms, and how it fits the section:\n\n"${selection}"`);
    else if (item.id === "note-selection") addNote(`> ${selection.replace(/\n/g, "\n> ")}`);
  };

  return (
    <aside ref={root} aria-label="Tutor" className="relative flex h-full flex-col bg-chrome">
      <ViewHeader title="Tutor">
        <IconButton label="New conversation" onClick={clearChat} disabled={busy || (!chat.length && !error)}>
          <Plus />
        </IconButton>
      </ViewHeader>
      <div ref={list} aria-live="polite" className="min-h-0 flex-1 overflow-auto select-text">
        {chat.length === 0 && !busy && !error && <Empty />}
        {chat.map((m, i) => (
          <Message key={i} role={m.role} content={m.content} />
        ))}
        {busy && <Message role="assistant" content={streaming} streaming />}
        {error && (
          <div className="flex gap-2 px-4 py-3 text-destructive-foreground">
            <CircleAlert className="mt-0.5 size-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}
      </div>
      <div className="relative shrink-0 border-t px-3 pt-2 pb-3">
        <Context />
        <Composer
          value={value}
          onValueChange={setValue}
          triggers={triggers}
          onAction={onAction}
          onSubmit={(v) => send(getComposerText(v))}
          disabled={!doc}
          submitDisabled={busy}
          className="text-[13px] [--composer-max-rows:8] [--composer-radius:var(--radius-lg)] md:[--composer-max-rows:8] [&_[data-slot=composer-scroll-area]]:text-[13px] [&_[data-slot=composer-scroll-area]]:md:text-[13px]"
        >
          <ComposerInput aria-label="Message the tutor" placeholder={doc ? "Ask about what you're reading, or / for actions" : "Open a PDF to ask about it"} />
          <ComposerPopup>
            <ComposerList>
              {(item: ComposerItemValue) => (
                <ComposerItem key={item.id} value={item} className="gap-2 [&_svg]:size-4 [&_svg]:opacity-80">
                  {item.icon}
                  <span className="flex min-w-0 flex-col">
                    <span className="truncate">{item.label}</span>
                    <span className="truncate text-xs text-muted-foreground">{item.description}</span>
                  </span>
                </ComposerItem>
              )}
            </ComposerList>
          </ComposerPopup>
          <ComposerActions>
            <ComposerActionGroup>
              <Button size="xs" variant="ghost" className="h-6 text-[11px] text-muted-foreground sm:h-6 sm:text-[11px]" disabled={!doc || busy} onClick={() => onAction(actions[0])}>
                Explain page
              </Button>
              <Button size="xs" variant="ghost" className="h-6 text-[11px] text-muted-foreground sm:h-6 sm:text-[11px]" disabled={!doc || busy} onClick={() => onAction(actions[1])}>
                Quiz me
              </Button>
            </ComposerActionGroup>
            <ComposerActionGroup align="end">
              <ComposerSubmit render={<Button aria-label="Send" size="icon-xs" className="size-[26px] sm:size-[26px]" loading={busy}><ArrowUp /></Button>} />
            </ComposerActionGroup>
          </ComposerActions>
        </Composer>
      </div>
    </aside>
  );
}
