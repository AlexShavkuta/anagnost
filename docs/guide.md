# Using Anagnost

Anagnost (from *anagnōstēs*, Greek for the reader who read texts aloud to you) is a macOS PDF reader for studying long technical documents with a model and with your coding agent.

The window is laid out like VS Code: the PDF in the middle, a side bar on the left (Map, Search, Notes, Library), the tutor on the right, and a status bar. Drag the edges to resize; `⌘B` and `⌥⌘B` hide the side bars; `⌘K` opens a command palette that jumps to any section; `⌘F` searches. In the tutor box, type `/` for its actions.

## Install and update

Download `Anagnost-<version>-macos-universal.zip` from the [Releases page](https://github.com/AlexShavkuta/anagnost/releases/latest), unzip it and move **Anagnost** to Applications. It runs on Apple silicon and Intel Macs.

The app isn't notarized, so macOS blocks a downloaded copy the first time. Allow it under System Settings → Privacy & Security, or run:

```bash
xattr -dr com.apple.quarantine /Applications/Anagnost.app
```

When a new version is released, Anagnost shows a popup in the bottom-right corner. **Update and restart** downloads it from GitHub Releases, checks its SHA-256, replaces the app and reopens it; updates installed this way don't need the step above. The app checks at launch and every 6 hours, and Settings → Updates checks on demand.

To build from source instead, see [development.md](development.md).

## The model: Anthropic or Cursor (bring your own key)

In **Settings → Tutor and map**, choose which account answers:

- **Anthropic API key**: calls the Claude API directly. The tutor uses `claude-opus-5-5` by default; the map has its own model setting (see below).
- **Cursor API key**: runs your Cursor plan through Cursor's CLI, `cursor-agent`. Create a key in the Cursor dashboard under Integrations, and install the CLI with `curl https://cursor.com/install -fsS | bash` if you don't have it. Leave Model empty for Cursor's default, or pick one of the models your account offers from the suggestions.

Keys are kept in `keys.json` in the app's data folder, readable only by your user (mode 600). The app never touches the Keychain: it's only ad-hoc signed, so the Keychain would ask for your password after every update. `ANTHROPIC_API_KEY` and `CURSOR_API_KEY` in the environment work as fallbacks.

`cursor-agent` is a coding agent, so Anagnost keeps it read-only: it runs in ask mode, in an empty temporary folder that's deleted afterwards, with a workspace config that denies shell commands and file writes. The key is passed in the environment, so it doesn't appear in the process list.

## The map

The map splits the PDF into sections from its bookmarks, or into chunks of about 4,000 words when it has none. Building it writes a summary, key concepts and review questions for each section, rolls them up into parent sections, and ends with an overview and a suggested reading path. The tree follows your reading position; tick sections off as studied.

Building makes one model call per section, plus one per parent section and one for the overview, eight at a time.

- **Before it starts**, a confirmation shows the number of sections, the model, and the estimated tokens and cost.
- **While it runs**, the status bar shows progress, tokens used and the cost so far, with a stop button (also in `⌘K`). Finished sections are kept, and building again picks up where it stopped.
- **Settings → Map model** picks the model on Anthropic: Haiku 5.5 (the default; a 300-section book costs well under a dollar), Sonnet 5.5 (about 20× Haiku's price), or Opus 5.5 (about 40×). Costs are estimates at Anthropic's list prices.
- On Cursor, the map runs on your plan with Cursor's model, and token use isn't shown because Cursor doesn't report it.

## Text recognition for scanned PDFs

When a PDF has pages with no text layer, the Map view offers to recognize them. Anagnost renders each page and sends it to [Qianfan-OCR](https://huggingface.co/baidu/Qianfan-OCR), a 4B document model from Baidu, running locally in llama.cpp's `llama-server`. Nothing leaves your Mac.

- **Settings → Text recognition → Install and load** downloads the official macOS build of llama.cpp (about 17 MB) into the app's data folder, then downloads (about 4.8 GB, into `~/.cache/huggingface/hub`) and loads the model. A Homebrew copy (`brew install llama.cpp`), one on your `PATH`, or a path set in Settings works too.
- By default the model loads when Anagnost opens and stays loaded while it's open (about 5 GB of memory). Turn this off to load it on first use and unload it after 5 idle minutes. It never starts a download on its own.
- Speed: about 20 seconds per page on an M2 Pro.
- Recognized text replaces the empty page text, so the tutor, the map, search and your agent all use it. A scanned PDF without bookmarks is re-split into sections once it has text.

## Connecting Claude Code or Cursor (MCP)

While Anagnost is open it runs an MCP server, so a coding agent can work with the PDF you have open. **Settings → Connect an agent** shows the setup with your own token:

- **Claude Code**: run the `claude mcp add --transport http anagnost http://127.0.0.1:7337/mcp --header "Authorization: Bearer <token>"` command once.
- **Cursor**: click **Add to Cursor** and confirm in Cursor, or paste the JSON shown into `~/.cursor/mcp.json`. `cursor-agent` reads the same file.

Then ask things like *"use anagnost to map out the document I have open"* or *"quiz me on the section I'm reading"*. These run on your Claude Code or Cursor plan.

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

## Your data

Everything is stored locally in `~/Library/Application Support/com.alex.pdftutor/`:

- `library.db` (SQLite): documents, page text, the map, notes and settings. Documents are keyed by a hash of their contents, so a PDF that is moved or renamed keeps its map.
- `keys.json`: your API keys.
- `perf.log`: page jumps, render times and moments the window stopped responding, kept under 512 KB, to diagnose slowdowns. It never leaves your Mac.

Anagnost reopens the PDF that was open when it quit, at the page you were on.

## Limits

- Recognized (scanned) pages have no selectable text in the viewer, so *Explain selection* doesn't work on them; *Explain page*, the map and search do.
- Sections longer than about 400k characters are only partly sent when building the map, and the prompt says so.
