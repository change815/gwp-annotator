#!/usr/bin/env sh
set -eu

PROJECT_ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
RUSTUP_HOME="$PROJECT_ROOT/.local-rust/rustup"
CARGO_HOME="$PROJECT_ROOT/.local-rust/cargo"

if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  printf '%s\n' "Run: sh scripts/with-project-npm.sh run rust:bootstrap" >&2
  exit 1
fi

if [ "${1:-}" != "cargo" ]; then
  printf '%s\n' "Usage: with-local-rust.sh cargo <cargo arguments>" >&2
  exit 1
fi
shift

exec env \
  RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_HOME="$CARGO_HOME" \
  PATH="$CARGO_HOME/bin:$PATH" \
  RUSTUP_TOOLCHAIN=1.97.1 \
  "$CARGO_HOME/bin/cargo" \
  --config "$PROJECT_ROOT/src-tauri/.cargo/config.toml" \
  "$@"
