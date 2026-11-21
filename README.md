# PDF Tutor

A local PDF reader that helps you learn what you're reading, with Claude.

- **Viewer**: pdf.js inside a Tauri 2 window, with selectable text, zoom, and page tracking.
- **Study map**: splits the PDF into sections (from its bookmarks, or ~4,000-word chunks when it has none) and asks Claude for a summary, key concepts and review questions for each section. It then rolls these up into parent sections and a whole-document overview with a suggested reading path. You can tick sections off as studied.
- **Tutor chat**: answers are grounded in the section you're reading and cite pages. Quick actions: explain the selection, explain the page, quiz me.
- **Notes**: per page, written by you, saved from the tutor, or added by Claude Code.
- **Claude Code integration**: while the app is open it runs an MCP server, so Claude Code can read the document, see your page and selection, move the viewer, and write to the map and notes.

Everything is stored locally in SQLite: `~/Library/Application Support/com.alex.pdftutor/library.db`. Documents are keyed by a hash of their contents, so a PDF that is moved or renamed keeps its map.

## Run

```bash
npm install
npm run tauri dev
```

Build a `.app`: `npm run tauri build`.

## Claude API key (in-app features)

Open **Settings** and paste an Anthropic API key. It is stored in the macOS Keychain, and `ANTHROPIC_API_KEY` from the environment works as a fallback. The default model is `claude-opus-5-5`; you can change it in Settings.

Map building makes one API call per section, plus one per parent section and one for the overview, with up to 4 calls running at once. The cost grows with the length of the document.

## Connecting Claude Code (MCP)

Settings shows a command like this one, with your own token:

```bash
claude mcp add --transport http pdf-tutor http://127.0.0.1:7337/mcp --header "Authorization: Bearer <token>"
```

After that, in Claude Code you can say things like *"use pdf-tutor to map out the document I have open"* or *"quiz me on the section I'm reading"*. These run on your Claude Code plan, so no API key is needed for them.

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
src/main.ts           viewer, map, notes, chat UI
src-tauri/src/
  lib.rs              Tauri commands + startup
  db.rs               SQLite schema, ingest (outline → section tree), search
  claude.rs           Claude Messages API client (raw HTTP + SSE streaming)
  mapping.rs          map pipeline: leaves → parents → overview
  mcp.rs              MCP server (Streamable HTTP, JSON responses)
  state.rs            shared state, Keychain API key
```

## Limits

- Scanned PDFs have no text layer. Text extraction comes back empty, so they need OCR first.
- Sections longer than about 400k characters are only partly sent when building the map, and the prompt tells Claude this.
