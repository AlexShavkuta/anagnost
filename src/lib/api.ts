// Typed wrappers around the Tauri commands in src-tauri/src/lib.rs.

import { invoke, Channel } from "@tauri-apps/api/core";

export interface Doc {
  id: string;
  path: string;
  title: string;
  page_count: number;
  last_page: number;
  overview: string | null;
}
export interface Concept {
  term: string;
  explanation: string;
}
export interface Section {
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
export interface Note {
  id: number;
  page: number | null;
  text: string;
  source: string;
  created_at: number;
}
export interface OutlineItem {
  title: string;
  page: number | null;
  items: OutlineItem[];
}
export type Provider = "anthropic" | "cursor";
export interface Settings {
  provider: Provider;
  has_api_key: boolean;
  model: string;
  has_cursor_key: boolean;
  cursor_model: string;
  cursor_agent_path: string;
  cursor_agent: string | null;
  mcp_url: string;
  mcp_token: string;
  mcp_error: string | null;
  llama_server_path: string;
  ocr_autoload: boolean;
  map_model: string;
}
export interface SearchHit {
  page: number;
  snippet: string;
}
export interface TextStatus {
  missing: number[];
  recognized: number[];
}
export type OcrStatus =
  | { state: "stopped" }
  | { state: "starting"; detail: string; progress: number | null }
  | { state: "ready" }
  | { state: "error"; message: string };
export interface OcrInfo {
  binary: string | null;
  model: string;
  downloaded: boolean;
  status: OcrStatus;
}
export interface Usage {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}
export interface MapOutcome {
  stopped: boolean;
  sections: number;
  model: string | null;
  usage: Usage | null;
  cost: number | null;
}
export interface MapEstimate {
  sections: number;
  model: string | null;
  input_tokens: number;
  output_tokens: number;
  cost: number | null;
}
export const MAP_MODELS: [string, string][] = [
  ["claude-haiku-5-5", "Haiku 5.5"],
  ["claude-sonnet-5-5", "Sonnet 5.5"],
  ["claude-opus-5-5", "Opus 5.5"],
];
export function modelName(id: string | null | undefined) {
  return MAP_MODELS.find(([m]) => m === id)?.[1] ?? id ?? "";
}
export function usageTotal(u: Usage) {
  return u.input + u.output + u.cache_read + u.cache_write;
}
export function tokens(n: number) {
  return n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${Math.round(n / 1000)}k` : String(n);
}
export function dollars(n: number) {
  return n < 0.01 ? "under $0.01" : n < 10 ? `$${n.toFixed(2)}` : `$${Math.round(n)}`;
}
export interface UpdateRelease {
  version: string;
  notes: string[];
  url: string;
  downloadable: boolean;
}
export interface UpdateCheck {
  current: string;
  latest: UpdateRelease | null;
  newer: boolean;
  /** Why the app can't install it itself, if it can't. */
  unavailable: string | null;
}
export type UpdateStatus =
  | { state: "idle" }
  | { state: "running"; step: string; progress: number | null }
  | { state: "error"; message: string };
export interface ChatMsg {
  role: "user" | "assistant";
  content: string;
}

export const api = {
  readFile: async (path: string) => new Uint8Array(await invoke<ArrayBuffer>("read_file", { path })),
  openDocument: (path: string) => invoke<{ id: string; document: Doc | null }>("open_document", { path }),
  ingestDocument: (a: { id: string; path: string; title: string; pages: string[]; outline: OutlineItem[] }) =>
    invoke<Doc>("ingest_document", a),
  recentDocuments: () => invoke<Doc[]>("recent_documents"),
  getDocument: (id: string) => invoke<Doc | null>("get_document", { id }),
  getSections: (docId: string) => invoke<Section[]>("get_sections", { docId }),
  setStudied: (docId: string, sectionId: number, studied: boolean) => invoke("set_studied", { docId, sectionId, studied }),
  setView: (docId: string | null, page: number, selection: string | null) => invoke("set_view", { docId, page, selection }),
  search: (docId: string, query: string) => invoke<SearchHit[]>("search", { docId, query }),
  textStatus: (docId: string) => invoke<TextStatus>("text_status", { docId }),
  ocrInfo: () => invoke<OcrInfo>("ocr_info"),
  ocrPage: (docId: string, page: number, image: string) => invoke<string>("ocr_page", { docId, page, image }),
  ocrFinish: (docId: string) => invoke<boolean>("ocr_finish", { docId }),
  ocrStop: () => invoke("ocr_stop"),
  ocrSetup: () => invoke("ocr_setup"),
  cursorModels: () => invoke<string[]>("cursor_models"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkUpdate: () => invoke<UpdateCheck>("check_update"),
  startUpdate: () => invoke("start_update"),
  buildMap: (docId: string, redo: boolean) => invoke<MapOutcome>("build_map", { docId, redo }),
  mapEstimate: (docId: string, redo: boolean) => invoke<MapEstimate>("map_estimate", { docId, redo }),
  stopMap: () => invoke("stop_map"),
  ask: (a: { docId: string; sectionId: number | null; page: number; messages: ChatMsg[]; onDelta: Channel<string> }) =>
    invoke<string>("ask", a),
  addNote: (docId: string, page: number | null, text: string, source: string) => invoke<number>("add_note", { docId, page, text, source }),
  listNotes: (docId: string) => invoke<Note[]>("list_notes", { docId }),
  deleteNote: (docId: string, id: number) => invoke("delete_note", { docId, id }),
  getSettings: () => invoke<Settings>("get_settings"),
  /** Omitted fields stay as they are; an empty key string removes that key. */
  saveSettings: (update: {
    provider?: Provider;
    apiKey?: string;
    model?: string;
    cursorApiKey?: string;
    cursorModel?: string;
    cursorAgentPath?: string;
    llamaServerPath?: string;
    ocrAutoload?: boolean;
    mapModel?: string;
  }) => invoke("save_settings", { update }),
};

/** Whether the chosen provider can run at all; returns what's missing otherwise. */
export function providerProblem(s: Settings | null): { title: string; text: string; focus: string } | null {
  if (!s) return null;
  if (s.provider === "anthropic" && !s.has_api_key) {
    return { title: "Connect Claude", text: "The tutor uses your Anthropic API key, kept on this Mac. Or switch to your Cursor key.", focus: "api-key" };
  }
  if (s.provider === "cursor" && !s.cursor_agent) {
    return { title: "Install Cursor's CLI", text: "The tutor runs through cursor-agent, which wasn't found on this Mac.", focus: "cursor-path" };
  }
  if (s.provider === "cursor" && !s.has_cursor_key) {
    return { title: "Connect Cursor", text: "Add your Cursor API key (Cursor dashboard → Integrations). It's kept on this Mac.", focus: "cursor-key" };
  }
  return null;
}

/** "Add to Cursor" deeplink that installs Anagnost's MCP server in Cursor. */
export function cursorInstallLink(s: Settings): string {
  const config = { url: s.mcp_url, headers: { Authorization: `Bearer ${s.mcp_token}` } };
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=anagnost&config=${encodeURIComponent(btoa(JSON.stringify(config)))}`;
}

export function cursorMcpJson(s: Settings): string {
  return JSON.stringify({ mcpServers: { "anagnost": { url: s.mcp_url, headers: { Authorization: `Bearer ${s.mcp_token}` } } } }, null, 2);
}
