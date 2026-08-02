#!/usr/bin/env sh
set -eu

PROJECT_ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)
RUST_VERSION=1.97.1
RUSTUP_HOME="$PROJECT_ROOT/.local-rust/rustup"
CARGO_HOME="$PROJECT_ROOT/.local-rust/cargo"
RUSTUP_DIST_SERVER=https://rsproxy.cn
RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup
CARGO_SPARSE_CONFIG=https://rsproxy.cn/index/config.json

case "$(uname -s):$(uname -m)" in
  Darwin:arm64) HOST_TRIPLE=aarch64-apple-darwin ;;
  Darwin:x86_64) HOST_TRIPLE=x86_64-apple-darwin ;;
  Linux:x86_64) HOST_TRIPLE=x86_64-unknown-linux-gnu ;;
  *)
    printf '%s\n' "Unsupported host for the project-local Rust bootstrap." >&2
    exit 1
    ;;
esac

INSTALL_ROOT="$PROJECT_ROOT/.local-rust"
RUSTUP_INIT="$INSTALL_ROOT/rustup-init-$HOST_TRIPLE"
mkdir -p "$INSTALL_ROOT"

# Verify the mainland-China mirror endpoint before downloading any installer bytes.
curl --fail --silent --show-error --head --max-time 30 \
  "$RUSTUP_DIST_SERVER/rustup/dist/$HOST_TRIPLE/rustup-init" >/dev/null
curl --fail --silent --show-error --head --max-time 30 \
  "$CARGO_SPARSE_CONFIG" >/dev/null
curl --fail --silent --show-error --location --max-time 120 \
  "$RUSTUP_DIST_SERVER/rustup/dist/$HOST_TRIPLE/rustup-init" \
  --output "$RUSTUP_INIT"
chmod +x "$RUSTUP_INIT"

env \
  RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_HOME="$CARGO_HOME" \
  RUSTUP_DIST_SERVER="$RUSTUP_DIST_SERVER" \
  RUSTUP_UPDATE_ROOT="$RUSTUP_UPDATE_ROOT" \
  "$RUSTUP_INIT" -y --no-modify-path --profile minimal --default-toolchain "$RUST_VERSION"

env \
  RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_HOME="$CARGO_HOME" \
  RUSTUP_DIST_SERVER="$RUSTUP_DIST_SERVER" \
  RUSTUP_UPDATE_ROOT="$RUSTUP_UPDATE_ROOT" \
  "$CARGO_HOME/bin/rustup" component add --toolchain "$RUST_VERSION" rustfmt clippy

printf '%s\n' "Project-local Rust $RUST_VERSION is ready."
