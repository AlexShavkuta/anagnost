#!/usr/bin/env bash
# Build PDF Tutor from source and install it into /Applications.
# Usage: ./scripts/install-macos.sh   (or: npm run install-app)
set -euo pipefail

cd "$(dirname "$0")/.."
APP_NAME="PDF Tutor"
BUNDLE="src-tauri/target/release/bundle/macos/$APP_NAME.app"

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname)" == "Darwin" ]] || fail "This script is for macOS."
xcode-select -p >/dev/null 2>&1 || fail "Xcode Command Line Tools are missing. Run: xcode-select --install"
command -v cargo >/dev/null || fail "Rust is missing. Install it from https://rustup.rs, then open a new terminal."
command -v npm >/dev/null || fail "Node.js is missing. Install it from https://nodejs.org (or: brew install node)."

say "Installing JavaScript dependencies"
npm ci --no-audit --no-fund

say "Building the app (the first build compiles Rust and takes a few minutes)"
npm run tauri build -- --bundles app

[[ -d "$BUNDLE" ]] || fail "Build finished but $BUNDLE wasn't found."

DEST="/Applications"
[[ -w "$DEST" ]] || DEST="$HOME/Applications"
mkdir -p "$DEST"

if pgrep -f "$DEST/$APP_NAME.app/Contents/MacOS/" >/dev/null; then
  say "Quitting the running copy"
  osascript -e "quit app \"$APP_NAME\"" || true
  sleep 2
fi

say "Installing to $DEST"
rm -rf "$DEST/$APP_NAME.app"
ditto "$BUNDLE" "$DEST/$APP_NAME.app"

say "Installed: $DEST/$APP_NAME.app"
echo
command -v llama-server >/dev/null && echo "  ✓ llama.cpp found (text recognition for scanned PDFs)" \
  || echo "  · Optional, for scanned PDFs: brew install llama.cpp"
{ command -v cursor-agent >/dev/null || [[ -x "$HOME/.local/bin/cursor-agent" ]]; } && echo "  ✓ cursor-agent found (use your Cursor key for the tutor)" \
  || echo "  · Optional, to use a Cursor key: curl https://cursor.com/install -fsS | bash"
echo
echo "Open it from Launchpad or Spotlight, then add an Anthropic or Cursor API key in Settings."
