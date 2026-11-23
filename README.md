# PDF Tutor

A local PDF reader that helps you learn what you're reading, with Claude.

The window is laid out like VS Code: the PDF in the middle, a side bar on the left (Map, Search, Notes, Library), the tutor on the right, and a status bar. Drag the edges to resize; `⌘B` and `⌥⌘B` hide the side bars; `⌘K` opens a command palette that jumps to any section. The UI is React with [coss ui](https://coss.com/ui) components (Base UI) on Tailwind. In the tutor box, type `/` for its actions.

- **Viewer**: pdf.js, with selectable text, zoom (`⌘=` `⌘−`, `⌘0` fits the width), and page tracking. A breadcrumb shows which section you're in.
- **Map**: splits the PDF into sections (from its bookmarks, or ~4,000-word chunks when it has none). Claude writes a summary, key concepts and review questions for each section, then rolls them up into parent sections and a whole-document overview with a suggested reading path. The tree follows your reading position, and the section you're in is shown in full below it. Tick sections off as studied.
- **Tutor**: answers are grounded in the section you're reading and cite pages. Select text in the PDF to have it explained or saved as a note, or use *Explain page* and *Quiz me*.
- **Search** (`⌘F`), **Notes** (yours, saved from the tutor, or added by Claude Code), and a **Library** of recent PDFs.
- **Text recognition (OCR)**: pages without a text layer (scans) are recognized locally with [Qianfan-OCR](https://huggingface.co/baidu/Qianfan-OCR). See below.
- **Claude Code integration**: while the app is open it runs an MCP server, so Claude Code can read the document, see your page and selection, move the viewer, and write to the map and notes.

Everything is stored locally in SQLite: `~/Library/Application Support/com.alex.pdftutor/library.db`. Documents are keyed by a hash of their contents, so a PDF that is moved or renamed keeps its map.

## Install (macOS)

You need Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs) and [Node.js](https://nodejs.org). Then:

```bash
git clone https://github.com/AlexShavkuta/pdf-tutor.git
cd pdf-tutor
npm run install-app
```

This builds the app and installs it into `/Applications` (or `~/Applications` if you can't write there). The first build takes a few minutes. Run the same command again after `git pull` to update.

Optional: `brew install llama.cpp` for text recognition in scanned PDFs, and Cursor's CLI (`curl https://cursor.com/install -fsS | bash`) to run the tutor on a Cursor key.

The app is built on your Mac and signed for it alone, so macOS opens it without warnings. A copy sent to another Mac would be blocked by Gatekeeper until it's notarized with an Apple Developer account.

## Develop

```bash
npm install
npm run tauri dev
```

## Model for the tutor and map: Anthropic or Cursor (bring your own key)

In **Settings → Tutor and map**, choose which account answers:

- **Anthropic API key**: calls the Claude API directly. The default model is `claude-opus-5-5`.
- **Cursor API key**: runs your Cursor plan through Cursor's CLI, `cursor-agent`. Create a key in the Cursor dashboard under Integrations. Leave the model empty for Cursor's default, or name one your plan offers. Install the CLI with `curl https://cursor.com/install -fsS | bash` if you don't have it.

Keys are stored in the macOS Keychain. `ANTHROPIC_API_KEY` and `CURSOR_API_KEY` in the environment work as fallbacks.

`cursor-agent` is a coding agent, so PDF Tutor keeps it read-only. It runs in ask mode, in an empty temporary folder that's deleted afterwards, with a workspace config that denies shell commands and file writes, and never with `--force`. The key is passed in the environment, so it doesn't appear in the process list.

Map building makes one model call per section, plus one per parent section and one for the overview, with up to 4 running at once. The cost grows with the length of the document.

## Text recognition (OCR)

When a PDF has pages with no text layer, the Map view offers to recognize them. PDF Tutor renders each page and sends it to [Qianfan-OCR](https://huggingface.co/baidu/Qianfan-OCR) (a 4B document model from Baidu), running locally in llama.cpp's `llama-server`. Nothing leaves your Mac.

- Needs llama.cpp: `brew install llama.cpp`. The app finds `llama-server` in Homebrew's folders or your `PATH`, or you can set its path in Settings.
- The first run downloads the model, about 4.8 GB (`ggml-org/Qianfan-OCR-GGUF`, Q8_0), into the Hugging Face cache (`~/.cache/huggingface/hub`).
- The app starts the server when it's needed and stops it after 5 idle minutes, or when you click *OCR model loaded* in the status bar.
- Speed: about 20 seconds per page on an M2 Pro.
- Recognized text replaces the empty page text, so the tutor, map, search and Claude Code all use it. A scanned PDF without bookmarks is re-split into sections once it has text.

## Connecting Claude Code or Cursor (MCP)

While PDF Tutor is open it runs an MCP server, so a coding agent can work with the PDF you have open. **Settings → Connect an agent** has the setup with your own token:

- **Claude Code**: run the `claude mcp add --transport http pdf-tutor http://127.0.0.1:7337/mcp --header "Authorization: Bearer <token>"` command once.
- **Cursor**: click **Add to Cursor** and confirm in Cursor, or paste the JSON shown into `~/.cursor/mcp.json`. `cursor-agent` reads the same file.

Then ask things like *"use pdf-tutor to map out the document I have open"* or *"quiz me on the section I'm reading"*. These run on your Claude Code or Cursor plan.

| Tool | What it does |
|---|---|
| `get_status` | Open document, current page, selection, map progress |
| `get_map` | Section tree with ids, pages, summaries, concepts, questions |
| `read_section` / `read_pages` | Page-tagged text (max 30 pages per call) |
| `search` | Find text across the document |
| `go_to_page` | Scroll your viewer |
| `save_section_analysis` / `set_overview` | Write to the map |
| `mark_studied`, `add_note`, `list_notes` | Track progress and notes |

The server listens only on 127.0.0.1. It requires the bearer token and rejects browser requests (any request with an `Origin` header).

## Layout

```
src/
  App.tsx             workbench layout (resizable panels), keyboard shortcuts, backend events
  components/ui/      coss ui components
  components/workbench/  title bar, activity bar, editor (PDF viewer), tutor, status bar, settings, ⌘K palette
  components/views/   side bar views: map, search, notes, library
  lib/store.ts        app state (zustand) and actions
  lib/api.ts          typed Tauri commands
  lib/pdf.ts, viewer.ts  pdf.js loading, text/outline extraction, page rendering
src-tauri/src/
  lib.rs              Tauri commands + startup
  db.rs               SQLite schema, ingest (outline → section tree), search
  claude.rs           Claude Messages API client (raw HTTP + SSE streaming)
  mapping.rs          map pipeline: leaves → parents → overview
  mcp.rs              MCP server (Streamable HTTP, JSON responses)
  llm.rs, cursor.rs   model provider: Claude API, or Cursor via cursor-agent
  ocr.rs              runs Qianfan-OCR in llama-server, on demand
  state.rs            shared state, Keychain API key
```

## Limits

- Recognized pages have no selectable text layer in the viewer, so *Explain selection* doesn't work on them; *Explain page*, the map and search do.
- Sections longer than about 400k characters are only partly sent when building the map, and the prompt tells Claude this.
