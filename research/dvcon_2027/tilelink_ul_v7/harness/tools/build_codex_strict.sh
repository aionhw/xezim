#!/usr/bin/env bash
# Project-local build; pre-extracted development packages and a recorded local
# lockfile are required. Preserve each build log under a unique supplied name.
set -euo pipefail
cd "$(dirname "$0")/.."
study_root="$PWD"
build_log="${1:?supply a new results log path}"
if [ -e "$build_log" ]; then echo 'refusing to overwrite build log' >&2; exit 1; fi
export OPENSSL_DIR="$study_root/.deps/build-dev/root/usr"
export OPENSSL_LIB_DIR="$OPENSSL_DIR/lib/x86_64-linux-gnu"
export OPENSSL_INCLUDE_DIR="$OPENSSL_DIR/include"
export PKG_CONFIG_PATH="$OPENSSL_LIB_DIR/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$study_root/.deps/build-dev/root"
export PROTOC="$OPENSSL_DIR/bin/protoc"
export LD_LIBRARY_PATH="$OPENSSL_LIB_DIR"
export CFLAGS="-I$OPENSSL_INCLUDE_DIR/x86_64-linux-gnu"
if [ "${2:-standard}" = "fast-final" ]; then
  cargo rustc --release --locked --manifest-path .deps/codex-protocol-source/codex-rs/Cargo.toml -p codex-cli --bin codex --target-dir .deps/codex_strict_target -- -C opt-level=0 > "$build_log" 2>&1
else
  cargo build --release --locked --manifest-path .deps/codex-protocol-source/codex-rs/Cargo.toml -p codex-cli --bin codex --target-dir .deps/codex_strict_target > "$build_log" 2>&1
fi
