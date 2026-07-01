#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

VERSION="$1"

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid semver version: $VERSION" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

jq --arg version "$VERSION" '.version = $version' "$ROOT/package.json" >"$ROOT/package.json.tmp"
mv "$ROOT/package.json.tmp" "$ROOT/package.json"

jq --arg version "$VERSION" '.version = $version' "$ROOT/src-tauri/tauri.conf.json" >"$ROOT/src-tauri/tauri.conf.json.tmp"
mv "$ROOT/src-tauri/tauri.conf.json.tmp" "$ROOT/src-tauri/tauri.conf.json"

sed -i "s/^version = \".*\"/version = \"$VERSION\"/" "$ROOT/src-tauri/Cargo.toml"

echo "Set project version to $VERSION"
