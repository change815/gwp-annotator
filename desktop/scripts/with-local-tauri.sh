#!/usr/bin/env sh
set -eu

PROJECT_ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
RUSTUP_HOME="$PROJECT_ROOT/.local-rust/rustup"
CARGO_HOME="$PROJECT_ROOT/.local-rust/cargo"
TAURI_BIN="$PROJECT_ROOT/node_modules/.bin/tauri"

if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  printf '%s\n' "Run: sh scripts/with-project-npm.sh run rust:bootstrap" >&2
  exit 1
fi

if [ ! -x "$TAURI_BIN" ]; then
  printf '%s\n' "Run: sh scripts/with-project-npm.sh ci" >&2
  exit 1
fi

cd "$PROJECT_ROOT"
exec env \
  RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_HOME="$CARGO_HOME" \
  PATH="$PROJECT_ROOT/scripts:$CARGO_HOME/bin:$PROJECT_ROOT/node_modules/.bin:$PATH" \
  RUSTUP_TOOLCHAIN=1.97.1 \
  "$TAURI_BIN" "$@"
