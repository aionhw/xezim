#!/usr/bin/env bash
# Profile-guided release build: instrument -> train -> rebuild with the
# profile. Measured on a large SoC CoreMark run: simulation loop -15%,
# host instructions -11.7%, cycles -6.1%, output bit-exact.
#
#   usage: scripts/build-pgo.sh <training-command...>
#
# The training command is run once against the instrumented binary and
# should resemble the workload that matters (a full benchmark iteration;
# an unrepresentative trainer can DEOPTIMIZE the paths you care about).
# Requirements: rustup component llvm-tools (for the toolchain-matched
# llvm-profdata — the system one usually version-mismatches and the merge
# fails). The profile degrades gracefully as sources change; rebuild it
# when perf numbers matter.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
[ $# -ge 1 ] || { echo "usage: $0 <training-command...>" >&2; exit 2; }
PROFDATA=$(ls "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/llvm-profdata 2>/dev/null | head -1)
[ -n "$PROFDATA" ] || { echo "llvm-profdata not found; rustup component add llvm-tools" >&2; exit 2; }
PDIR=$(mktemp -d)
trap 'rm -rf "$PDIR"' EXIT
echo "== instrumented build =="
RUSTFLAGS="-Cprofile-generate=$PDIR" ./scripts/cargo-local.sh build --release --features jit
echo "== training: $* =="
"$@"
"$PROFDATA" merge -o "$PDIR/merged.profdata" "$PDIR"/*.profraw
echo "== optimized build =="
RUSTFLAGS="-Cprofile-use=$PDIR/merged.profdata" ./scripts/cargo-local.sh build --release --features jit
echo "PGO build complete: target/release/xezim"
