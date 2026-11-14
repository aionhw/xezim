#!/usr/bin/env bash
# Keep LLD selected even when callers supply their own RUSTFLAGS.
set -euo pipefail
target_libdir=$("${RUSTC:-rustc}" --print target-libdir --target x86_64-unknown-linux-gnu)
bundled_lld="$(dirname "$target_libdir")/bin/gcc-ld/ld.lld"
if [ -x "$bundled_lld" ]; then
  exec clang "-fuse-ld=$bundled_lld" "$@"
fi
exec clang -fuse-ld=lld "$@"
