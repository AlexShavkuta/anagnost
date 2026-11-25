# Developing Anagnost

## Build and install from source

You need Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs) and [Node.js](https://nodejs.org). Then:

```bash
git clone https://github.com/AlexShavkuta/anagnost.git
cd anagnost
npm run install-app
```

This builds the app and installs it into `/Applications` (or `~/Applications` if you can't write there). The first build takes a few minutes. A copy built this way is signed for your Mac alone, so macOS opens it without warnings.

## Run in development

```bash
npm install
npm run tauri dev
```

Checks:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
npx tsc --noEmit
```

## Release

```bash
npm run release -- 0.4.0
```

This bumps the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, commits, tags `v0.4.0` and pushes. The tag starts `.github/workflows/release.yml`, which runs the tests, builds a universal (Apple silicon + Intel) app, and publishes the release with the zip and its SHA-256. Release notes are generated from the merged pull requests. Installed copies pick the release up through their update popup.

The bundle identifier stays `com.alex.pdftutor` (from the app's first name), so data, keys and updates carry over between versions.

## Layout

```
src/
  App.tsx                 workbench layout (resizable panels), keyboard shortcuts, backend events
  components/ui/          coss ui components (Base UI on Tailwind)
  components/workbench/   title bar, activity bar, editor (PDF viewer), tutor, status bar,
                          settings, ⌘K palette, update popup
  components/views/       side bar views: map, search, notes, library
  lib/store.ts            app state (zustand) and actions
  lib/api.ts              typed Tauri commands
  lib/pdf.ts, viewer.ts   pdf.js loading, text and outline extraction, scheduled page rendering
  lib/perf.ts             local performance log
src-tauri/src/
  lib.rs                  Tauri commands and startup
  db.rs                   SQLite schema, ingest (outline → section tree), search
  claude.rs               Claude Messages API client (raw HTTP, streaming, usage and pricing)
  mapping.rs              map pipeline: leaves → parents → overview, with estimate and stop
  mcp.rs                  MCP server (Streamable HTTP)
  llm.rs, cursor.rs       model provider: Claude API, or Cursor via cursor-agent
  ocr.rs                  Qianfan-OCR in llama-server, installed and run on demand
  update.rs               updates from GitHub Releases
  secrets.rs              API keys (private file)
  state.rs                shared state
design/icon.svg           app icon source
```
