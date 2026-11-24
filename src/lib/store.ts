// App state (zustand). Components read slices; actions talk to the backend.

import { create } from "zustand";
import { Channel } from "@tauri-apps/api/core";
import { ask, message, open as openDialog } from "@tauri-apps/plugin-dialog";
import { toastManager } from "@/components/ui/toast";
import { api, providerProblem, type ChatMsg, type Doc, type Note, type OcrStatus, type Section, type Settings, type TextStatus, type UpdateCheck, type UpdateStatus, type Usage, dollars, modelName, tokens, usageTotal } from "./api";
import { extractOutline, extractText, loadPdf, pageImage, pdfTitle, type PDFDocumentProxy } from "./pdf";
import { load, save } from "./persist";
import { perf } from "./perf";

export type View = "map" | "search" | "notes" | "library";

interface Task {
  text: string;
  done?: boolean;
}

interface State {
  pdf: PDFDocumentProxy | null;
  doc: Doc | null;
  sections: Section[];
  notes: Note[];
  recent: Doc[];
  textStatus: TextStatus;
  settings: Settings | null;

  currentPage: number;
  /** Bumped to ask the viewer to scroll; the viewer reports back through setCurrentPage. */
  scrollTo: { page: number; smooth: boolean; seq: number } | null;
  selection: string;

  view: View;
  leftOpen: boolean;
  rightOpen: boolean;
  settingsOpen: boolean;
  settingsFocus: string | null;
  paletteOpen: boolean;
  searchFocusSeq: number;
  chatFocusSeq: number;

  chat: ChatMsg[];
  streaming: string | null;
  chatError: string;

  task: Task | null;
  mapRun: { done: number; total: number; tokens: number | null; cost: number | null; stopping: boolean } | null;
  ocrRun: { done: number; total: number; cancelled: boolean } | null;
  ocrServer: OcrStatus;
  /** `dismissed`: the version whose popup the reader closed this session. */
  update: { check: UpdateCheck | null; checking: boolean; checkError: string; status: UpdateStatus; dismissed: string | null };
  expanded: Set<number>;
}

interface Actions {
  init(): Promise<void>;
  choosePdf(): Promise<void>;
  /** `quiet`: reopening at launch, so a missing file just falls back to the welcome screen. */
  openPath(path: string, quiet?: boolean): Promise<void>;
  refreshMap(): Promise<void>;
  refreshNotes(): Promise<void>;
  refreshTextStatus(): Promise<void>;
  refreshRecent(): Promise<void>;
  loadSettings(): Promise<void>;
  goToPage(n: number, smooth?: boolean): void;
  setCurrentPage(n: number): void;
  setSelection(text: string): void;
  showView(v: View, toggle?: boolean): void;
  setLeftOpen(open: boolean): void;
  setRightOpen(open: boolean): void;
  openSettings(focus?: string): void;
  closeSettings(): void;
  setPaletteOpen(open: boolean): void;
  focusChat(): void;
  toggleExpanded(id: number): void;
  collapseAll(): void;
  toggleStudied(id: number): Promise<void>;
  buildMap(): Promise<void>;
  recognize(): Promise<void>;
  stopOcr(): void;
  sendChat(text: string, sectionId?: number): Promise<void>;
  clearChat(): void;
  addNote(text: string, source?: string): Promise<void>;
  deleteNote(id: number): Promise<void>;
  setOcrServer(s: OcrStatus): void;
  /** Ask GitHub for new commits; `quiet` keeps errors out of the UI (the launch check). */
  checkForUpdates(quiet?: boolean): Promise<void>;
  startUpdate(): Promise<void>;
  dismissUpdate(): void;
  setUpdateStatus(s: UpdateStatus): void;
  setMapProgress(p: { done: number; total: number; current: string; usage: Usage | null; cost: number | null }): void;
  stopMap(): void;
  flash(text: string): void;
}

export function currentSection(sections: Section[], page: number): Section | null {
  let best: Section | null = null;
  for (const s of sections) {
    if (s.start_page <= page && page <= s.end_page && (!best || s.depth > best.depth)) best = s;
  }
  return best;
}

export function ancestors(sections: Section[], s: Section | null): Section[] {
  const out: Section[] = [];
  while (s) {
    out.unshift(s);
    const pid: number | null = s.parent_id;
    s = pid === null ? null : (sections.find((x) => x.id === pid) ?? null);
  }
  return out;
}

let flashTimer: number | undefined;
let viewTimer: number | undefined;
let scrollSeq = 0;

export const useApp = create<State & Actions>()((set, get) => ({
  pdf: null,
  doc: null,
  sections: [],
  notes: [],
  recent: [],
  textStatus: { missing: [], recognized: [] },
  settings: null,
  currentPage: 1,
  scrollTo: null,
  selection: "",
  view: load<View>("view", "map"),
  leftOpen: load("leftOpen", true),
  rightOpen: load("rightOpen", true),
  settingsOpen: false,
  settingsFocus: null,
  paletteOpen: false,
  searchFocusSeq: 0,
  chatFocusSeq: 0,
  chat: [],
  streaming: null,
  chatError: "",
  task: null,
  mapRun: null,
  ocrRun: null,
  ocrServer: { state: "stopped" },
  update: { check: null, checking: false, checkError: "", status: { state: "idle" }, dismissed: null },
  expanded: new Set(),

  async init() {
    perf.start();
    await Promise.all([get().loadSettings(), get().refreshRecent()]);
    // Reopen what was open when the app last quit, at the page you were on.
    const last = load<string | null>("lastPath", null);
    if (last && !get().doc) await get().openPath(last, true);
  },

  async choosePdf() {
    const path = await openDialog({ multiple: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
    if (typeof path === "string") await get().openPath(path);
  },

  async openPath(path, quiet = false) {
    const run = get().ocrRun;
    if (run) run.cancelled = true;
    try {
      set({ task: { text: "Opening…" } });
      const opened = await api.openDocument(path);
      const pdf = await loadPdf(await api.readFile(path));
      let doc = opened.document;
      if (!doc) {
        const title = await pdfTitle(pdf, path);
        const pages = await extractText(pdf, (i) => set({ task: { text: `Reading text ${i}/${pdf.numPages}` } }));
        set({ task: { text: "Reading bookmarks…" } });
        const outline = await extractOutline(pdf);
        doc = await api.ingestDocument({ id: opened.id, path, title, pages, outline });
      }
      set({
        pdf,
        doc,
        sections: [],
        notes: [],
        chat: [],
        chatError: "",
        streaming: null,
        selection: "",
        expanded: new Set(),
        currentPage: doc.last_page || 1,
      });
      get().goToPage(doc.last_page || 1, false);
      save("lastPath", path);
      await Promise.all([get().refreshMap(), get().refreshNotes(), get().refreshTextStatus(), get().refreshRecent()]);
      set({ task: null });
    } catch (e) {
      set({ task: null });
      if (quiet) {
        // Reopening at launch: the file was moved or deleted. Start on the welcome screen.
        save("lastPath", null);
        return;
      }
      await message(`Could not open the PDF.\n\n${e}`, { title: "Anagnost", kind: "error" });
    }
  },

  async refreshMap() {
    const doc = get().doc;
    if (!doc) return;
    const [d, sections] = await Promise.all([api.getDocument(doc.id), api.getSections(doc.id)]);
    if (get().doc?.id !== doc.id) return;
    const expanded = new Set(get().expanded);
    for (const s of ancestors(sections, currentSection(sections, get().currentPage)).slice(0, -1)) expanded.add(s.id);
    set({ doc: d ?? doc, sections, expanded });
  },

  async refreshNotes() {
    const doc = get().doc;
    if (doc) set({ notes: await api.listNotes(doc.id) });
  },

  async refreshTextStatus() {
    const doc = get().doc;
    if (doc) set({ textStatus: await api.textStatus(doc.id) });
  },

  async refreshRecent() {
    set({ recent: await api.recentDocuments() });
  },

  async loadSettings() {
    set({ settings: await api.getSettings() });
  },

  goToPage(n, smooth = false) {
    perf.mark(`go ${n}`);
    const pdf = get().pdf;
    if (!pdf) return;
    const page = Math.min(pdf.numPages, Math.max(1, Math.round(n)));
    set({ scrollTo: { page, smooth, seq: ++scrollSeq } });
    get().setCurrentPage(page);
  },

  setCurrentPage(n) {
    const { currentPage, sections, expanded } = get();
    if (n === currentPage) return;
    const before = currentSection(sections, currentPage)?.id;
    const now = currentSection(sections, n);
    let next = expanded;
    if (now && now.id !== before) {
      // Follow the reader in the tree, like VS Code's outline.
      const chain = ancestors(sections, now).slice(0, -1);
      if (chain.some((s) => !expanded.has(s.id))) {
        next = new Set(expanded);
        chain.forEach((s) => next.add(s.id));
      }
    }
    set({ currentPage: n, expanded: next });
    reportView();
  },

  setSelection(text) {
    if (text === get().selection) return;
    set({ selection: text });
    reportView();
  },

  showView(v, toggle = false) {
    const { leftOpen, view } = get();
    if (toggle && leftOpen && view === v) {
      get().setLeftOpen(false);
      return;
    }
    set({ view: v, leftOpen: true });
    save("view", v);
    save("leftOpen", true);
    if (v === "search") set((s) => ({ searchFocusSeq: s.searchFocusSeq + 1 }));
    if (v === "library") get().refreshRecent();
  },

  setLeftOpen(open) {
    set({ leftOpen: open });
    save("leftOpen", open);
  },
  setRightOpen(open) {
    set({ rightOpen: open });
    save("rightOpen", open);
  },

  openSettings(focus) {
    get().loadSettings();
    set({ settingsOpen: true, settingsFocus: focus ?? null });
  },
  closeSettings() {
    set({ settingsOpen: false, settingsFocus: null });
  },
  setPaletteOpen(open) {
    set({ paletteOpen: open });
  },
  focusChat() {
    get().setRightOpen(true);
    set((s) => ({ chatFocusSeq: s.chatFocusSeq + 1 }));
  },

  toggleExpanded(id) {
    const expanded = new Set(get().expanded);
    expanded.has(id) ? expanded.delete(id) : expanded.add(id);
    set({ expanded });
  },
  collapseAll() {
    set({ expanded: new Set() });
  },

  async toggleStudied(id) {
    const { doc, sections } = get();
    const s = sections.find((x) => x.id === id);
    if (!doc || !s) return;
    await api.setStudied(doc.id, id, !s.studied);
    await get().refreshMap();
  },

  async buildMap() {
    const { doc, sections, settings, textStatus, mapRun } = get();
    if (!doc || mapRun) return;
    const problem = providerProblem(settings);
    if (problem) return get().openSettings(problem.focus);
    if (textStatus.missing.length) {
      const go = await ask(
        `${textStatus.missing.length} pages have no text yet, so the map can't cover them. Build it anyway, or recognize their text first?`,
        { title: "Build map", kind: "warning", okLabel: "Build anyway", cancelLabel: "Cancel" },
      );
      if (!go) return;
    }
    const mapped = sections.filter((s) => s.summary).length;
    const redo = mapped === sections.length && mapped > 0;
    // Say what it will cost before spending anything.
    let est;
    try {
      est = await api.mapEstimate(doc.id, redo);
    } catch (e) {
      await message(String(e), { title: "Build map", kind: "error" });
      return;
    }
    const what = `${redo ? "Rebuild" : mapped ? "Finish" : "Build"} the map: ${est.sections} ${est.sections === 1 ? "section" : "sections"}.`;
    const cost = est.model
      ? `With ${modelName(est.model)}: about ${tokens(est.input_tokens)} input and ${tokens(est.output_tokens)} output tokens, roughly ${est.cost !== null ? dollars(est.cost) : "an unknown cost"} on your Anthropic account. You can change the model in Settings.`
      : "It runs on your Cursor plan with Cursor's model. Cursor doesn't report token use.";
    const go = await ask(`${what}\n\n${cost}\n\nYou can stop at any time; finished sections are kept.`, {
      title: "Build map",
      kind: "info",
      okLabel: redo ? "Rebuild" : "Build",
      cancelLabel: "Cancel",
    });
    if (!go) return;
    set({ mapRun: { done: 0, total: est.sections + 1, tokens: null, cost: null, stopping: false }, task: { text: "Building map…" } });
    try {
      const out = await api.buildMap(doc.id, redo);
      const spent = out.usage ? ` · ${tokens(usageTotal(out.usage))} tokens${out.cost !== null ? `, ${dollars(out.cost)}` : ""}` : "";
      get().flash(out.stopped ? `Map stopped after ${out.sections} sections${spent}` : `Map built${spent}`);
    } catch (e) {
      set({ task: null });
      await message(String(e), { title: "Build map", kind: "error" });
    } finally {
      set({ mapRun: null });
      get().refreshMap();
    }
  },
  setMapProgress({ done, total, current, usage, cost }) {
    const run = get().mapRun;
    set({
      mapRun: { done, total, tokens: usage ? usageTotal(usage) : null, cost, stopping: run?.stopping ?? false },
      task: { text: run?.stopping ? "Stopping the map…" : `Mapping ${done}/${total} · ${current}` },
    });
  },
  stopMap() {
    const run = get().mapRun;
    if (!run || run.stopping) return;
    set({ mapRun: { ...run, stopping: true }, task: { text: "Stopping the map…" } });
    api.stopMap();
  },

  async recognize() {
    const { doc, pdf, ocrRun, textStatus } = get();
    if (!doc || !pdf || ocrRun || textStatus.missing.length === 0) return;
    const pages = [...textStatus.missing];
    const run = { done: 0, total: pages.length, cancelled: false };
    set({ ocrRun: run });
    let failure = "";
    for (const p of pages) {
      if (run.cancelled || get().doc?.id !== doc.id) break;
      try {
        await api.ocrPage(doc.id, p, await pageImage(pdf, p));
        run.done++;
        const ts = get().textStatus;
        set({
          ocrRun: { ...run },
          textStatus: { missing: ts.missing.filter((x) => x !== p), recognized: [...ts.recognized, p] },
        });
      } catch (e) {
        failure = String(e);
        break;
      }
    }
    set({ ocrRun: null });
    if (get().doc?.id === doc.id) {
      await api.ocrFinish(doc.id);
      await Promise.all([get().refreshTextStatus(), get().refreshMap()]);
    }
    if (failure) await message(failure, { title: "Text recognition stopped", kind: "error" });
    else if (!run.cancelled) get().flash(`Recognized text on ${run.done} ${run.done === 1 ? "page" : "pages"}`);
  },

  stopOcr() {
    const run = get().ocrRun;
    if (run) {
      run.cancelled = true;
      set({ ocrRun: { ...run } });
    }
  },

  async checkForUpdates(quiet = false) {
    const up = () => get().update;
    if (up().checking) return;
    set({ update: { ...up(), checking: true, checkError: "" } });
    try {
      const [check, status] = await Promise.all([api.checkUpdate(), api.updateStatus()]);
      set({ update: { ...up(), check, status: status ?? { state: "idle" }, checking: false } });
    } catch (e) {
      set({ update: { ...up(), checking: false, checkError: quiet ? "" : String(e) } });
    }
  },
  dismissUpdate() {
    const v = get().update.check?.latest?.version ?? null;
    set({ update: { ...get().update, dismissed: v } });
  },
  async startUpdate() {
    try {
      await api.startUpdate();
    } catch (e) {
      set({ update: { ...get().update, status: { state: "error", message: String(e) } } });
    }
  },
  setUpdateStatus(status) {
    set({ update: { ...get().update, status } });
  },
  setOcrServer(s) {
    set({ ocrServer: s });
  },

  async sendChat(text, sectionId) {
    const { doc, streaming, settings, currentPage, sections } = get();
    if (!doc || streaming !== null || !text.trim()) return;
    const problem = providerProblem(settings);
    if (problem) return get().openSettings(problem.focus);
    const chat = [...get().chat, { role: "user" as const, content: text.trim() }];
    set({ chat, chatError: "", streaming: "", rightOpen: true });
    const onDelta = new Channel<string>();
    let reply = "";
    let frame = 0;
    onDelta.onmessage = (t) => {
      reply += t;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => set({ streaming: reply }));
    };
    try {
      const full = await api.ask({
        docId: doc.id,
        sectionId: sectionId ?? currentSection(sections, currentPage)?.id ?? null,
        page: currentPage,
        messages: chat,
        onDelta,
      });
      cancelAnimationFrame(frame);
      set({ chat: [...chat, { role: "assistant", content: full || reply }], streaming: null });
    } catch (e) {
      cancelAnimationFrame(frame);
      // Drop the unanswered question so the next request's history stays valid.
      set({ chat: chat.slice(0, -1), streaming: null, chatError: String(e) });
      throw e;
    }
  },

  clearChat() {
    if (get().streaming === null) set({ chat: [], chatError: "" });
  },

  async addNote(text, source = "user") {
    const { doc, currentPage } = get();
    if (!doc || !text.trim()) return;
    await api.addNote(doc.id, currentPage, text.trim(), source);
    await get().refreshNotes();
  },

  async deleteNote(id) {
    const doc = get().doc;
    if (!doc) return;
    await api.deleteNote(doc.id, id);
    await get().refreshNotes();
  },

  flash(text) {
    set({ task: { text, done: true } });
    toastManager.add({ title: text, type: "success", timeout: 4000 });
    clearTimeout(flashTimer);
    flashTimer = window.setTimeout(() => {
      if (get().task?.done) set({ task: null });
    }, 4000);
  },
}));

/** Tell the backend what the user is looking at, so Claude Code's tools can see it. */
function reportView() {
  clearTimeout(viewTimer);
  viewTimer = window.setTimeout(() => {
    const { doc, currentPage, selection } = useApp.getState();
    api.setView(doc?.id ?? null, currentPage, selection || null).catch(console.error);
  }, 250);
}
