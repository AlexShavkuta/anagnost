#!/usr/bin/env bash
# Release a new version: bump it everywhere, commit, tag and push. CI builds and publishes it.
# Usage: npm run release -- 0.3.0
set -euo pipefail
cd "$(dirname "$0")/.."

fail() { printf '\033[31mError:\033[0m %s\n' "$*" >&2; exit 1; }
version="${1:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "Give a version like 0.3.0"
[[ "$(git branch --show-current)" == "main" ]] || fail "Release from main."
[[ -z "$(git status --porcelain)" ]] || fail "Commit or stash your changes first."
git fetch -q origin main
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || fail "main isn't in sync with origin/main."
git rev-parse -q --verify "refs/tags/v$version" >/dev/null && fail "v$version already exists."

current=$(node -p "require('./src-tauri/tauri.conf.json').version")
echo "Releasing $current -> $version"

npm version "$version" --no-git-tag-version >/dev/null
node -e '
  const fs = require("fs"), f = "src-tauri/tauri.conf.json";
  const c = JSON.parse(fs.readFileSync(f, "utf8")); c.version = process.argv[1];
  fs.writeFileSync(f, JSON.stringify(c, null, 2) + "\n");
' "$version"
perl -pi -e 'if (!$done && s/^version = "[^"]*"/version = "'"$version"'"/) { $done = 1 }' src-tauri/Cargo.toml
cargo update -q --manifest-path src-tauri/Cargo.toml -p pdf-tutor

git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -qm "Release v$version"
git tag -a "v$version" -m "PDF Tutor $version"
git push -q origin main "v$version"
echo "Pushed v$version. GitHub Actions is building it: https://github.com/$(git remote get-url origin | sed -E 's#(git@github.com:|https://github.com/)##; s#\.git$##')/actions"
