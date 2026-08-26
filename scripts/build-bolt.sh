#!/usr/bin/env bash
# Post-link-optimize the release binary with LLVM BOLT, LBR-profiled on a
# training command. Measured on the c906 CoreMark it1 benchmark: -1.1%
# instructions, -1.6% cycles, -1.8% simulation loop vs the same build
# un-bolted (interleaved A/B, PMU-judged); output gate-verified bit-exact.
#
# DO NOT stack on a PGO build: measured NET NEGATIVE there (+1.4% cycles,
# +1.2% simulation loop — BOLT's re-layout fights PGO's own profile-driven
# placement). PGO alone (scripts/build-pgo.sh, -17.7%) is the perf build;
# use BOLT only when PGO is not in the pipeline.
#
# Usage:
#   ./scripts/build-bolt.sh <training-command...>
# e.g.
#   ./scripts/build-bolt.sh ./target/release/xezim --simulate design.sv
#
# The training command should run the REAL workload shape; the binary path
# inside it is substituted automatically with the relocation build.
#
# Requirements:
#   * llvm-bolt + perf2bolt on PATH, or BOLT_BIN=/path/to/llvm-20/bin
#     (no root needed: `apt-get download bolt-20 && dpkg -x` works).
#   * perf with LBR (`perf record -j any,u` must produce branch stacks).
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."

BOLT_BIN="${BOLT_BIN:-}"
find_tool() {
  local t="$1"
  if [ -n "$BOLT_BIN" ] && [ -x "$BOLT_BIN/$t" ]; then echo "$BOLT_BIN/$t"; return; fi
  command -v "$t" || { echo "error: $t not found (set BOLT_BIN)" >&2; exit 1; }
}
LLVM_BOLT=$(find_tool llvm-bolt)
PERF2BOLT=$(find_tool perf2bolt)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

echo "== build with relocations preserved (-Wl,-q) =="
RUSTFLAGS="-C link-arg=-Wl,-q ${RUSTFLAGS:-}" ./scripts/cargo-local.sh build --release --features jit
cp target/release/xezim "$WORK/xezim.base"

echo "== LBR training run =="
# Substitute any argument that IS the xezim binary with the reloc build.
ARGS=()
for a in "$@"; do
  case "$a" in
    */xezim|xezim) ARGS+=("$WORK/xezim.base");;
    *) ARGS+=("$a");;
  esac
done
perf record -e cycles:u -j any,u -F 400 -o "$WORK/perf.data" -- "${ARGS[@]}"

echo "== perf2bolt =="
"$PERF2BOLT" -p "$WORK/perf.data" -o "$WORK/prof.fdata" "$WORK/xezim.base"

echo "== llvm-bolt =="
"$LLVM_BOLT" "$WORK/xezim.base" -o target/release/xezim.bolt \
  -data="$WORK/prof.fdata" \
  -reorder-blocks=ext-tsp -reorder-functions=cdsort \
  -split-functions -split-all-cold -split-eh -icf=all -dyno-stats

echo "== done: target/release/xezim.bolt =="
echo "   (debug info is stripped in the bolted binary; keep xezim for debugging)"
