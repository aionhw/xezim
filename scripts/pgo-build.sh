#!/bin/bash
# Profile-Guided Optimization build for xezim.
#
# Usage:   scripts/pgo-build.sh <PROFILE_CMD>
# Example: scripts/pgo-build.sh "./target/release/xezim --simulate -f /tmp/c906_abs.fl -s tb -I .../src --max-time 700000000"
#
# PGO adds 5-10% wall-time speedup on the workload it was profiled with.
# The optimization is workload-specific — a c906-profiled binary helps c906
# more than c910, etc. For mixed workloads, profile multiple runs back-to-
# back and they accumulate into the same /tmp/pgo-data directory.
#
# Measured impact on c906 hello:
#   thin LTO no PGO:   38.4 s
#   thin LTO + PGO:    34.9 s   (-9%)
#   fat LTO + PGO:     35.9 s   (no benefit over thin+PGO; fat LTO alone regressed)
#
# Requires `llvm-tools-preview` rustup component:
#   rustup component add llvm-tools-preview
set -euo pipefail

PROFILE_CMD="${1:-}"
if [ -z "$PROFILE_CMD" ]; then
    echo "usage: $0 '<command to run with the instrumented binary>'" >&2
    echo "example: $0 './target/release/xezim --simulate -f /tmp/c906_abs.fl -s tb ...'" >&2
    exit 2
fi

PGO_DATA=${PGO_DATA:-/tmp/xezim-pgo-data}
LLVM_PROFDATA=$(find ~/.rustup -name 'llvm-profdata' 2>/dev/null | head -1)
if [ -z "$LLVM_PROFDATA" ]; then
    SYSROOT=$(rustc --print sysroot)
    HOST=$(rustc -vV | sed -n 's/host: //p')
    if [ -f "$SYSROOT/lib/rustlib/$HOST/bin/llvm-profdata" ]; then
        LLVM_PROFDATA="$SYSROOT/lib/rustlib/$HOST/bin/llvm-profdata"
    fi
fi
[ -z "$LLVM_PROFDATA" ] && {
    echo "error: llvm-profdata not found. Install via: rustup component add llvm-tools-preview" >&2
    exit 1
}

cd "$(dirname "$0")/.."

echo "[pgo] (1/4) clean profile dir: $PGO_DATA"
rm -rf "$PGO_DATA" && mkdir -p "$PGO_DATA"

echo "[pgo] (2/4) build instrumented binary"
touch src/compiler/simulator.rs   # force re-link with new RUSTFLAGS
RUSTFLAGS="-Cprofile-generate=$PGO_DATA" cargo build --release

echo "[pgo] (3/4) profile run: $PROFILE_CMD"
eval "$PROFILE_CMD" >/dev/null 2>&1 || {
    echo "[pgo] profile run exited non-zero, but continuing (profile data may still be partial)"
}
echo "[pgo]   -> $(ls "$PGO_DATA" | wc -l) profile files, $(du -sh "$PGO_DATA" | cut -f1)"

echo "[pgo] (4/4) merge profiles + rebuild PGO-optimized"
"$LLVM_PROFDATA" merge -o "$PGO_DATA/merged.profdata" "$PGO_DATA"
touch src/compiler/simulator.rs
RUSTFLAGS="-Cprofile-use=$PGO_DATA/merged.profdata" cargo build --release

echo "[pgo] done. PGO-optimized binary at: target/release/xezim"
