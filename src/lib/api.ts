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
  buildMap: (docId: string, redo: boolean) => invoke("build_map", { docId, redo }),
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
  }) => invoke("save_settings", { update }),
};

/** Whether the chosen provider can run at all; returns what's missing otherwise. */
export function providerProblem(s: Settings | null): { title: string; text: string; focus: string } | null {
  if (!s) return null;
  if (s.provider === "anthropic" && !s.has_api_key) {
    return { title: "Connect Claude", text: "The tutor uses your Anthropic API key, stored in the macOS Keychain. Or switch to your Cursor key.", focus: "api-key" };
  }
  if (s.provider === "cursor" && !s.cursor_agent) {
    return { title: "Install Cursor's CLI", text: "The tutor runs through cursor-agent, which wasn't found on this Mac.", focus: "cursor-path" };
  }
  if (s.provider === "cursor" && !s.has_cursor_key) {
    return { title: "Connect Cursor", text: "Add your Cursor API key (Cursor dashboard → Integrations). It's stored in the macOS Keychain.", focus: "cursor-key" };
  }
  return null;
}

/** "Add to Cursor" deeplink that installs PDF Tutor's MCP server in Cursor. */
export function cursorInstallLink(s: Settings): string {
  const config = { url: s.mcp_url, headers: { Authorization: `Bearer ${s.mcp_token}` } };
  return `cursor://anysphere.cursor-deeplink/mcp/install?name=pdf-tutor&config=${encodeURIComponent(btoa(JSON.stringify(config)))}`;
}

export function cursorMcpJson(s: Settings): string {
  return JSON.stringify({ mcpServers: { "pdf-tutor": { url: s.mcp_url, headers: { Authorization: `Bearer ${s.mcp_token}` } } } }, null, 2);
}
