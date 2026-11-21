import "pdfjs-dist/web/pdf_viewer.css";
import "./styles.css";
import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist";
import { invoke, Channel } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { marked } from "marked";
import DOMPurify from "dompurify";

pdfjs.GlobalWorkerOptions.workerSrc = new URL("pdfjs-dist/build/pdf.worker.mjs", import.meta.url).toString();

// ---------- types (mirror the Rust structs) ----------

interface Doc {
  id: string;
  path: string;
  title: string;
  page_count: number;
  last_page: number;
  overview: string | null;
}
interface Concept {
  term: string;
  explanation: string;
}
interface Section {
  id: number;
  parent_id: number | null;
  ord: number;
  depth: number;
  title: string;
  start_page: number;
  end_page: number;
  summary: string | null;
  concepts: Concept[];
  questions: string[];
  studied: boolean;
}
interface Note {
  id: number;
  page: number | null;
  text: string;
  source: string;
  created_at: number;
}
interface OutlineItem {
  title: string;
  page: number | null;
  items: OutlineItem[];
}
interface Settings {
  has_api_key: boolean;
  model: string;
  mcp_url: string;
  mcp_token: string;
  mcp_error: string | null;
}
interface ChatMsg {
  role: "user" | "assistant";
  content: string;
}

// ---------- state ----------

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

let pdf: PDFDocumentProxy | null = null;
let loadingTask: ReturnType<typeof pdfjs.getDocument> | null = null;
let doc: Doc | null = null;
let sections: Section[] = [];
let scale = 1.25;
let currentPage = 1;
let selection = "";
let chat: ChatMsg[] = [];
let chatBusy = false;
let chatError = "";
let mapBusy = false;
const expanded = new Set<number>();
const rendered = new Map<number, { scale: number; task?: { cancel(): void } }>();

const md = (s: string) => DOMPurify.sanitize(marked.parse(s, { async: false }) as string);
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

// ---------- opening documents ----------

async function openPath(path: string) {
  try {
    setStatus("Opening…");
    const opened = await invoke<{ id: string; document: Doc | null }>("open_document", { path });
    const bytes = new Uint8Array(await invoke<ArrayBuffer>("read_file", { path }));
    loadingTask?.destroy();
    loadingTask = pdfjs.getDocument({ data: bytes });
    pdf = await loadingTask.promise;

    if (opened.document) {
      doc = opened.document;
    } else {
      setStatus("Extracting text…");
      const meta = await pdf.getMetadata().catch(() => null);
      const metaTitle = (meta?.info as { Title?: string } | undefined)?.Title?.trim();
      const title = metaTitle || path.split("/").pop()!.replace(/\.pdf$/i, "");
      const pages = await extractText(pdf);
      const outline = await extractOutline(pdf);
      doc = await invoke<Doc>("ingest_document", { id: opened.id, path, title, pages, outline });
    }
    chat = [];
    renderChat();
    $("doc-title").textContent = doc.title;
    $("page-count").textContent = `/ ${pdf.numPages}`;
    ($("page-input") as HTMLInputElement).max = String(pdf.numPages);
    $("viewer-empty").hidden = true;
    for (const id of ["map-btn", "page-btn", "quiz-btn"]) ($(id) as HTMLButtonElement).disabled = false;
    await layoutPages();
    goToPage(doc.last_page || 1, false);
    await Promise.all([refreshMap(), refreshNotes(), refreshRecent()]);
    setStatus("");
  } catch (e) {
    setStatus("");
    alert(`Could not open the PDF: ${e}`);
  }
}

async function extractText(p: PDFDocumentProxy): Promise<string[]> {
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
    if (i % 25 === 0) setStatus(`Extracting text… ${i}/${p.numPages}`);
  }
  return out;
}

async function extractOutline(p: PDFDocumentProxy): Promise<OutlineItem[]> {
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

// ---------- viewer ----------

async function layoutPages() {
  const container = $("pages");
  container.innerHTML = "";
  rendered.clear();
  if (!pdf) return;
  // Size every page placeholder from page 1 up front; correct per page on render.
  const first = await pdf.getPage(1);
  const vp = first.getViewport({ scale });
  for (let i = 1; i <= pdf.numPages; i++) {
    const div = document.createElement("div");
    div.className = "page";
    div.dataset.page = String(i);
    div.style.width = `${vp.width}px`;
    div.style.height = `${vp.height}px`;
    container.appendChild(div);
    observer.observe(div);
  }
  $("zoom-label").textContent = `${Math.round(scale * 100)}%`;
}

const observer = new IntersectionObserver(
  (entries) => {
    for (const e of entries) {
      if (e.isIntersecting) renderPage(Number((e.target as HTMLElement).dataset.page));
    }
  },
  { root: document.getElementById("viewer"), rootMargin: "800px 0px" },
);

async function renderPage(n: number) {
  if (!pdf) return;
  const prev = rendered.get(n);
  if (prev && prev.scale === scale) return;
  prev?.task?.cancel();
  const entry: { scale: number; task?: { cancel(): void } } = { scale };
  rendered.set(n, entry);

  const div = $("pages").querySelector<HTMLElement>(`.page[data-page="${n}"]`);
  if (!div) return;
  const page: PDFPageProxy = await pdf.getPage(n);
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

  const task = page.render({
    canvas,
    viewport: vp,
    transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : undefined,
  });
  entry.task = task;
  try {
    await task.promise;
  } catch {
    return; // cancelled by a zoom change
  }
  if (rendered.get(n) !== entry) return;
  div.replaceChildren(canvas, textDiv);
  const tl = new pdfjs.TextLayer({ textContentSource: await page.getTextContent(), container: textDiv, viewport: vp });
  await tl.render();
}

function setZoom(next: number) {
  if (!pdf) return;
  const page = currentPage;
  scale = Math.min(4, Math.max(0.5, next));
  layoutPages().then(() => goToPage(page, false));
}

function goToPage(n: number, smooth = true) {
  if (!pdf) return;
  n = Math.min(pdf.numPages, Math.max(1, Math.round(n)));
  const div = $("pages").querySelector<HTMLElement>(`.page[data-page="${n}"]`);
  div?.scrollIntoView({ behavior: smooth ? "smooth" : "auto", block: "start" });
  setCurrentPage(n);
}

function setCurrentPage(n: number) {
  if (n === currentPage && doc) return;
  currentPage = n;
  ($("page-input") as HTMLInputElement).value = String(n);
  reportView();
  highlightCurrentSection();
}

$("viewer").addEventListener("scroll", () => {
  if (!pdf) return;
  const viewer = $("viewer");
  const mid = viewer.getBoundingClientRect().top + viewer.clientHeight / 3;
  for (const div of $("pages").children as HTMLCollectionOf<HTMLElement>) {
    const r = div.getBoundingClientRect();
    if (r.top <= mid && r.bottom >= mid) {
      setCurrentPage(Number(div.dataset.page));
      break;
    }
  }
});

document.addEventListener("selectionchange", () => {
  const sel = window.getSelection();
  const inViewer = sel && sel.anchorNode && $("pages").contains(sel.anchorNode);
  const text = inViewer ? sel!.toString().trim() : "";
  if (text === selection) return;
  selection = text;
  ($("explain-btn") as HTMLButtonElement).disabled = !text;
  ($("note-sel-btn") as HTMLButtonElement).disabled = !text;
  reportView();
});

let viewTimer: number | undefined;
function reportView() {
  clearTimeout(viewTimer);
  viewTimer = window.setTimeout(() => {
    invoke("set_view", { docId: doc?.id ?? null, page: currentPage, selection: selection || null }).catch(console.error);
  }, 250);
}

// ---------- map ----------

function currentSection(): Section | null {
  let best: Section | null = null;
  for (const s of sections) {
    if (s.start_page <= currentPage && currentPage <= s.end_page && (!best || s.depth > best.depth)) best = s;
  }
  return best;
}

async function refreshMap() {
  if (!doc) return;
  const id = doc.id;
  const [d, secs] = await Promise.all([
    invoke<Doc | null>("get_document", { id }),
    invoke<Section[]>("get_sections", { docId: id }),
  ]);
  if (doc?.id !== id) return;
  if (d) doc = d;
  sections = secs;
  renderMap();
}

function renderMap() {
  $("map-empty").hidden = sections.length > 0;
  const overview = doc?.overview;
  $("overview-box").hidden = !overview;
  if (overview) $("overview").innerHTML = md(overview);

  const studied = sections.filter((s) => s.studied).length;
  const summarized = sections.filter((s) => s.summary).length;
  $("study-progress").textContent = sections.length
    ? `${studied}/${sections.length} sections studied · ${summarized}/${sections.length} mapped`
    : "";
  ($("map-btn") as HTMLButtonElement).textContent = summarized === sections.length && sections.length ? "Rebuild map" : "Build map";

  const byParent = new Map<number | null, Section[]>();
  for (const s of sections) {
    const list = byParent.get(s.parent_id) ?? [];
    list.push(s);
    byParent.set(s.parent_id, list);
  }
  const build = (parent: number | null): string =>
    (byParent.get(parent) ?? [])
      .sort((a, b) => a.ord - b.ord)
      .map((s) => {
        const kids = byParent.get(s.id);
        const open = expanded.has(s.id);
        const details = open && s.summary
          ? `<div class="details">
              <p>${esc(s.summary)}</p>
              ${s.concepts.length ? `<h4>Key concepts</h4><dl>${s.concepts.map((c) => `<dt>${esc(c.term)}</dt><dd>${esc(c.explanation)}</dd>`).join("")}</dl>` : ""}
              ${s.questions.length ? `<h4>Check yourself</h4><ol>${s.questions.map((q, i) => `<li><a href="#" data-ask="${s.id}:${i}">${esc(q)}</a></li>`).join("")}</ol>` : ""}
            </div>`
          : open ? `<div class="details muted">Not mapped yet. Build the map, or ask Claude Code to analyze this section.</div>` : "";
        return `<li data-id="${s.id}">
          <div class="node ${s.summary ? "mapped" : ""}">
            <input type="checkbox" data-studied="${s.id}" ${s.studied ? "checked" : ""} title="Mark as studied" />
            <a href="#" data-go="${s.id}" class="node-title">${esc(s.title)}</a>
            <span class="pages">${s.start_page === s.end_page ? s.start_page : `${s.start_page}–${s.end_page}`}</span>
            <button class="link" data-toggle="${s.id}">${open ? "▾" : "▸"}</button>
          </div>
          ${details}
          ${kids ? `<ul>${build(s.id)}</ul>` : ""}
        </li>`;
      })
      .join("");
  $("tree").innerHTML = build(null);
  highlightCurrentSection();
}

function highlightCurrentSection() {
  const cur = currentSection();
  document.querySelectorAll(".tree .node.current").forEach((n) => n.classList.remove("current"));
  if (cur) document.querySelector(`.tree li[data-id="${cur.id}"] > .node`)?.classList.add("current");
  $("chat-context").textContent = cur ? `· ${cur.title}` : doc ? `· page ${currentPage}` : "";
}

$("tree").addEventListener("click", (e) => {
  const t = e.target as HTMLElement;
  const find = (attr: string) => sections.find((s) => s.id === Number(t.dataset[attr]));
  if (t.dataset.go) {
    e.preventDefault();
    const s = find("go");
    if (s) goToPage(s.start_page);
  } else if (t.dataset.toggle) {
    const id = Number(t.dataset.toggle);
    expanded.has(id) ? expanded.delete(id) : expanded.add(id);
    renderMap();
  } else if (t.dataset.ask) {
    e.preventDefault();
    const [sid, qi] = t.dataset.ask.split(":").map(Number);
    const s = sections.find((x) => x.id === sid);
    if (s) {
      goToPage(s.start_page);
      sendChat(`Quiz me on this question, one step at a time. Don't give the answer until I've tried:\n\n"${s.questions[qi]}"`, s.id);
    }
  }
});

$("tree").addEventListener("change", async (e) => {
  const t = e.target as HTMLInputElement;
  if (!t.dataset.studied || !doc) return;
  await invoke("set_studied", { docId: doc.id, sectionId: Number(t.dataset.studied), studied: t.checked });
  refreshMap();
});

$("map-btn").addEventListener("click", async () => {
  if (!doc || mapBusy) return;
  const mapped = sections.filter((s) => s.summary).length;
  const redo = mapped === sections.length && mapped > 0;
  if (redo && !confirm("Rebuild every section summary? This makes new Claude API calls.")) return;
  mapBusy = true;
  ($("map-btn") as HTMLButtonElement).disabled = true;
  $("map-progress").hidden = false;
  try {
    await invoke("build_map", { docId: doc.id, redo });
  } catch (e) {
    alert(String(e));
  } finally {
    mapBusy = false;
    ($("map-btn") as HTMLButtonElement).disabled = false;
    $("map-progress").hidden = true;
    refreshMap();
  }
});

// ---------- notes ----------

async function refreshNotes() {
  if (!doc) return;
  const notes = await invoke<Note[]>("list_notes", { docId: doc.id });
  const label: Record<string, string> = { user: "You", "claude-app": "Tutor", "claude-code": "Claude Code" };
  $("notes").innerHTML = notes
    .map(
      (n) => `<li>
        <div class="note-meta">
          <span>${label[n.source] ?? n.source}</span>
          ${n.page ? `<a href="#" data-page="${n.page}">p. ${n.page}</a>` : ""}
          <button class="link" data-del="${n.id}">Delete</button>
        </div>
        <div class="md">${md(n.text)}</div>
      </li>`,
    )
    .join("") || `<li class="muted">No notes yet.</li>`;
}

async function addNote(text: string, source = "user") {
  if (!doc || !text.trim()) return;
  await invoke("add_note", { docId: doc.id, page: currentPage, text: text.trim(), source });
  refreshNotes();
}

$("note-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const input = $("note-input") as HTMLTextAreaElement;
  await addNote(input.value);
  input.value = "";
});

$("notes").addEventListener("click", async (e) => {
  const t = e.target as HTMLElement;
  if (t.dataset.page) {
    e.preventDefault();
    goToPage(Number(t.dataset.page));
  } else if (t.dataset.del && doc) {
    await invoke("delete_note", { docId: doc.id, id: Number(t.dataset.del) });
    refreshNotes();
  }
});

// ---------- chat ----------

function renderChat(streaming?: string) {
  const msgs = chat.map(
    (m, i) => `<div class="msg ${m.role}">
      ${m.role === "assistant" ? md(m.content) : `<p>${esc(m.content)}</p>`}
      ${m.role === "assistant" ? `<button class="link" data-save="${i}">Save as note</button>` : ""}
    </div>`,
  );
  if (chatError) msgs.push(`<div class="msg error"><p>${esc(chatError)}</p></div>`);
  if (streaming !== undefined) msgs.push(`<div class="msg assistant">${streaming ? md(streaming) : `<p class="muted">Thinking…</p>`}</div>`);
  $("messages").innerHTML =
    msgs.join("") ||
    `<p class="muted">Ask anything about the document. Answers are grounded in the section you're reading${doc ? "" : " once a PDF is open"}.</p>`;
  $("messages").scrollTop = $("messages").scrollHeight;
}

async function sendChat(text: string, sectionId?: number) {
  if (!doc || chatBusy || !text.trim()) return;
  chatBusy = true;
  chatError = "";
  ($("send-btn") as HTMLButtonElement).disabled = true;
  chat.push({ role: "user", content: text.trim() });
  let reply = "";
  renderChat(reply);

  const onDelta = new Channel<string>();
  let frame = 0;
  onDelta.onmessage = (t) => {
    reply += t;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => renderChat(reply));
  };
  try {
    const full = await invoke<string>("ask", {
      docId: doc.id,
      sectionId: sectionId ?? currentSection()?.id ?? null,
      page: currentPage,
      messages: chat,
      onDelta,
    });
    cancelAnimationFrame(frame);
    chat.push({ role: "assistant", content: full || reply });
  } catch (e) {
    cancelAnimationFrame(frame);
    // Drop the unanswered question so the history sent next time stays valid,
    // and put it back in the input so it can be retried.
    chat.pop();
    ($("chat-input") as HTMLTextAreaElement).value = text.trim();
    chatError = String(e);
    if (chatError.includes("API key")) openSettings();
  } finally {
    chatBusy = false;
    ($("send-btn") as HTMLButtonElement).disabled = false;
    renderChat();
  }
}

$("chat-form").addEventListener("submit", (e) => {
  e.preventDefault();
  const input = $("chat-input") as HTMLTextAreaElement;
  const text = input.value;
  input.value = "";
  sendChat(text);
});
$("chat-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    ($("chat-form") as HTMLFormElement).requestSubmit();
  }
});
$("chat-clear").addEventListener("click", () => {
  chat = [];
  chatError = "";
  renderChat();
});
$("messages").addEventListener("click", (e) => {
  const t = e.target as HTMLElement;
  if (t.dataset.save) {
    addNote(chat[Number(t.dataset.save)].content, "claude-app");
    (t as HTMLButtonElement).textContent = "Saved";
  }
});
$("explain-btn").addEventListener("click", () => {
  if (selection) sendChat(`Explain this passage (page ${currentPage}) in plain terms, and how it fits the section:\n\n"${selection}"`);
});
$("page-btn").addEventListener("click", () => sendChat(`Walk me through page ${currentPage}: the main points and anything tricky.`));
$("quiz-btn").addEventListener("click", () =>
  sendChat("Quiz me on the section I'm reading. Ask one question at a time, wait for my answer, then give feedback."),
);
$("note-sel-btn").addEventListener("click", () => selection && addNote(`> ${selection.replace(/\n/g, "\n> ")}`));

// ---------- settings ----------

async function openSettings() {
  const s = await invoke<Settings>("get_settings");
  $("key-status").textContent = s.has_api_key ? "(saved — leave blank to keep)" : "(not set)";
  ($("api-key") as HTMLInputElement).value = "";
  ($("model") as HTMLInputElement).value = s.model;
  $("mcp-cmd").textContent = `claude mcp add --transport http pdf-tutor ${s.mcp_url} --header "Authorization: Bearer ${s.mcp_token}"`;
  $("mcp-error").hidden = !s.mcp_error;
  $("mcp-error").textContent = s.mcp_error ? `MCP server isn't running: ${s.mcp_error}` : "";
  ($("settings") as HTMLDialogElement).showModal();
}

$("settings-btn").addEventListener("click", openSettings);
$("copy-mcp").addEventListener("click", () => navigator.clipboard.writeText($("mcp-cmd").textContent ?? ""));
$("settings").addEventListener("close", async () => {
  const dlg = $("settings") as HTMLDialogElement;
  if (dlg.returnValue !== "save") return;
  const key = ($("api-key") as HTMLInputElement).value.trim();
  const model = ($("model") as HTMLInputElement).value.trim();
  try {
    await invoke("save_settings", { apiKey: key || null, model: model || null });
  } catch (e) {
    alert(`Could not save settings: ${e}`);
  }
});

// ---------- toolbar, tabs, events ----------

async function refreshRecent() {
  const docs = await invoke<Doc[]>("recent_documents");
  $("recent").innerHTML =
    `<option value="">Recent…</option>` + docs.map((d) => `<option value="${esc(d.path)}">${esc(d.title)}</option>`).join("");
}

$("open-btn").addEventListener("click", async () => {
  const path = await openDialog({ multiple: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
  if (typeof path === "string") openPath(path);
});
$("recent").addEventListener("change", (e) => {
  const sel = e.target as HTMLSelectElement;
  if (sel.value) openPath(sel.value);
  sel.value = "";
});
$("prev-btn").addEventListener("click", () => goToPage(currentPage - 1));
$("next-btn").addEventListener("click", () => goToPage(currentPage + 1));
$("page-input").addEventListener("change", (e) => goToPage(Number((e.target as HTMLInputElement).value)));
$("zoom-in").addEventListener("click", () => setZoom(scale + 0.25));
$("zoom-out").addEventListener("click", () => setZoom(scale - 0.25));

document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((b) =>
  b.addEventListener("click", () => {
    document.querySelectorAll(".tabs button").forEach((x) => x.classList.toggle("active", x === b));
    $("tab-map").hidden = b.dataset.tab !== "map";
    $("tab-notes").hidden = b.dataset.tab !== "notes";
  }),
);

function setStatus(text: string) {
  $("doc-title").textContent = text || doc?.title || "No document";
}

listen<{ doc_id: string; done: number; total: number; current: string }>("map-progress", (e) => {
  if (e.payload.doc_id !== doc?.id) return;
  $("map-progress").hidden = false;
  $("map-bar").style.width = `${(100 * e.payload.done) / Math.max(1, e.payload.total)}%`;
  $("map-status").textContent = `${e.payload.done}/${e.payload.total} · ${e.payload.current}`;
});
listen<string>("doc-changed", (e) => {
  if (e.payload === doc?.id) {
    refreshMap();
    refreshNotes();
  }
});
listen<number>("goto-page", (e) => goToPage(e.payload));

refreshRecent();
renderChat();
