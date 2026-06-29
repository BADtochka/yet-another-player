#!/usr/bin/env bash
set -euo pipefail

sudo apt-get update
sudo apt-get install -y clang lld llvm nsis

rustup target add x86_64-pc-windows-msvc

if ! command -v cargo-xwin >/dev/null 2>&1; then
  cargo install cargo-xwin
fi
