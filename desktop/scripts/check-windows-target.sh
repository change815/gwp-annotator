#!/usr/bin/env sh
set -eu

PROJECT_ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
RUSTUP_HOME="$PROJECT_ROOT/.local-rust/rustup"
CARGO_HOME="$PROJECT_ROOT/.local-rust/cargo"
RUST_VERSION=1.97.1
TARGET=x86_64-pc-windows-msvc
RUSTUP_DIST_SERVER=https://rsproxy.cn
RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup

if [ ! -x "$CARGO_HOME/bin/rustup" ] || [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  printf '%s\n' "Run: sh scripts/with-project-npm.sh run rust:bootstrap" >&2
  exit 1
fi

if ! env \
  RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_HOME="$CARGO_HOME" \
  "$CARGO_HOME/bin/rustup" target list --installed --toolchain "$RUST_VERSION" \
  | grep -Fx "$TARGET" >/dev/null; then
  env \
    RUSTUP_HOME="$RUSTUP_HOME" \
    CARGO_HOME="$CARGO_HOME" \
    RUSTUP_DIST_SERVER="$RUSTUP_DIST_SERVER" \
    RUSTUP_UPDATE_ROOT="$RUSTUP_UPDATE_ROOT" \
    "$CARGO_HOME/bin/rustup" target add --toolchain "$RUST_VERSION" "$TARGET"
fi

cd "$PROJECT_ROOT"
exec env \
  RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_HOME="$CARGO_HOME" \
  PATH="$CARGO_HOME/bin:$PATH" \
  RUSTUP_TOOLCHAIN="$RUST_VERSION" \
  "$CARGO_HOME/bin/cargo" \
  --config "$PROJECT_ROOT/src-tauri/.cargo/config.toml" \
  check \
  --manifest-path src-tauri/Cargo.toml \
  --target "$TARGET"
