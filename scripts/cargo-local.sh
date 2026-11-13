#!/usr/bin/env bash
# Run cargo with the xezim-core git dependency patched to the sibling
# ../xezim-core checkout WHEN one exists; plain cargo (git fetch) otherwise.
# No persistent local-core override file needed.
#
# Usage:
#   ./scripts/cargo-local.sh build --release
#   ./scripts/cargo-local.sh test --features jit
#   ./scripts/cargo-local.sh            # defaults to `build`
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."

ARGS=("$@")
[ ${#ARGS[@]} -eq 0 ] && ARGS=(build)

# Local builds tune for this machine (CI's plain `cargo` stays portable).
# Prepend so a caller's RUSTFLAGS (e.g. frame pointers for profiling) wins
# on conflicts. XEZIM_NO_NATIVE_CPU=1 opts out.
if [ -z "${XEZIM_NO_NATIVE_CPU:-}" ]; then
  if [ "${CARGO_ENCODED_RUSTFLAGS+x}" = x ]; then
    # Cargo gives encoded flags precedence, even when they are empty.
    # Keep each encoded argument intact and let later caller flags win.
    export CARGO_ENCODED_RUSTFLAGS="-Ctarget-cpu=native${CARGO_ENCODED_RUSTFLAGS:+$'\x1f'$CARGO_ENCODED_RUSTFLAGS}"
  else
    export RUSTFLAGS="-C target-cpu=native ${RUSTFLAGS:-}"
  fi
fi

if [ -f ../xezim-core/Cargo.toml ]; then
  exec cargo \
    --config 'patch."https://github.com/aionhw/xezim-core.git".xezim-core.path="../xezim-core"' \
    --config 'patch."https://github.com/aionhw/xezim-core.git".sv-parser.path="../xezim-core/xezim-parser"' \
    "${ARGS[@]}"
else
  exec cargo "${ARGS[@]}"
fi
