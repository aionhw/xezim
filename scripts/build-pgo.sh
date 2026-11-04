#!/usr/bin/env bash
# Profile-guided release build: instrument -> train -> rebuild with the
# profile. Measured on a large SoC CoreMark run: simulation loop -15%,
# host instructions -11.7%, cycles -6.1%, output bit-exact.
#
#   usage: scripts/build-pgo.sh [training-command...]
#
# With no arguments the bundled trainer runs: the tests/perf shape designs,
# the scripts/pgo-train designs and the xezim-bench workloads, all against
# the instrumented binaries. Cheap (minutes) and always safe; measured
# about -0.5% host instructions on the SoC benchmarks and -12% on a
# loop-heavy DRAM model. A profile trained on the workload itself reaches
# -14.5% (see README), so pass a training command when you have one.
#
# A training command, when given, is run once against the instrumented
# binary instead and should resemble the workload that matters (a full
# benchmark iteration; an unrepresentative trainer can DEOPTIMIZE the
# paths you care about). Standalone xezim / xezim-bench path arguments are
# substituted with the instrumented builds.
# Requirements: rustup component llvm-tools (for the toolchain-matched
# llvm-profdata — the system one usually version-mismatches and the merge
# fails). The profile degrades gracefully as sources change; rebuild it
# when perf numbers matter.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."
# The profile format must match the compiler: use the ACTIVE toolchain's
# llvm-profdata (`rustup component add llvm-tools-preview`), never a system
# LLVM of a different version.
RUSTC_BIN=${RUSTC:-${CARGO_BUILD_RUSTC:-rustc}}
SYSROOT=$("$RUSTC_BIN" --print sysroot)
HOST=$("$RUSTC_BIN" -vV | sed -n 's/^host: //p')
PROFDATA="$SYSROOT/lib/rustlib/$HOST/bin/llvm-profdata"
[ -x "$PROFDATA" ] || { echo "llvm-profdata not found in $SYSROOT; rustup component add llvm-tools-preview" >&2; exit 2; }
# Feature set of the built binary: default none (interpreter + two-state
# path); PGO_FEATURES="--features jit" for the native-backend build.
FEATURES=${PGO_FEATURES:-}
read -r -a FEATURE_ARGS <<< "$FEATURES"
# Separate target dir so the instrumented and optimized artifacts never
# clobber an ordinary release build.
TDIR=${PGO_TARGET_DIR:-pgo-target}
mkdir -p -- "$TDIR"
TDIR=$(cd "$TDIR" && pwd -P)
PDIR=${PGO_PROFILE_DIR:-$TDIR/profile}
mkdir -p -- "$PDIR/raw" "$PDIR/training"
PDIR=$(cd "$PDIR" && pwd -P)
RAW_DIR="$PDIR/raw"

# Preserve Cargo's flag precedence and arguments. RUSTFLAGS is split on
# whitespace by Cargo; encoded flags already delimit arguments with US.
# Append PGO as one encoded argument, so profile paths can contain spaces.
BASE_FLAGS=${CARGO_ENCODED_RUSTFLAGS:-}
if [ "${CARGO_ENCODED_RUSTFLAGS+x}" != x ]; then
  FLAGS=()
  read -r -d '' -a FLAGS <<< "${RUSTFLAGS:-}" || true
  BASE_FLAGS=""
  for flag in "${FLAGS[@]}"; do
    BASE_FLAGS+="${BASE_FLAGS:+$'\x1f'}$flag"
  done
fi
build_with_profile() {
  local flag="$1"
  CARGO_ENCODED_RUSTFLAGS="${BASE_FLAGS}${BASE_FLAGS:+$'\x1f'}$flag" \
    ./scripts/cargo-local.sh build --release --bin xezim --bin xezim-bench \
      --target-dir "$TDIR" "${FEATURE_ARGS[@]}"
}
clear_raw_profiles() {
  # Only files named by our LLVM_PROFILE_FILE inside the dedicated raw
  # directory are ours to remove. Keep merged data and unrelated files.
  local raw
  for raw in "$RAW_DIR"/xezim-*.profraw; do
    [ ! -f "$raw" ] || rm -- "$raw"
  done
}
train() {
  local label="$1"; shift
  local log="$PDIR/training/$label.log"
  "$@" > "$log" 2>&1 || {
    echo "trainer failed: $label (log: $log)" >&2
    tail -40 "$log" >&2
    return 1
  }
}
{
  "$RUSTC_BIN" -vV
  cargo -V
  "$PROFDATA" --version
  printf 'PGO_TARGET_DIR=%q\nPGO_PROFILE_DIR=%q\nPGO_FEATURES=%q\n' "$TDIR" "$PDIR" "$FEATURES"
  printf 'RUSTC_BIN=%q\n' "$RUSTC_BIN"
  printf 'RUSTFLAGS=%q\nCARGO_ENCODED_RUSTFLAGS=%q\n' "${RUSTFLAGS-}" "${CARGO_ENCODED_RUSTFLAGS-}"
  printf 'effective_base_flags=%q\nXEZIM_NO_NATIVE_CPU=%q\n' "$BASE_FLAGS" "${XEZIM_NO_NATIVE_CPU-}"
  printf 'CARGO_PROFILE_RELEASE_LTO=%q\nCARGO_PROFILE_RELEASE_CODEGEN_UNITS=%q\n' "${CARGO_PROFILE_RELEASE_LTO-}" "${CARGO_PROFILE_RELEASE_CODEGEN_UNITS-}"
} > "$PDIR/build-info.txt"
export LLVM_PROFILE_FILE="$RAW_DIR/xezim-%m-%p.profraw"
clear_raw_profiles
echo "== instrumented build =="
build_with_profile "-Cprofile-generate=$RAW_DIR"
# Build scripts may also be instrumented; the profile should describe the
# simulator's training runs, so discard those build-time records first.
clear_raw_profiles
if [ $# -ge 1 ]; then
  echo "== training: $* =="
  ARGS=()
  for a in "$@"; do
    case "$a" in
      */xezim|xezim) ARGS+=("$TDIR/release/xezim");;
      */xezim-bench|xezim-bench) ARGS+=("$TDIR/release/xezim-bench");;
      *) ARGS+=("$a");;
    esac
  done
  train custom "${ARGS[@]}"
else
  echo "== training: bundled designs and workloads =="
  # The test bench is the LAST module in each of these files.
  for d in tests/perf/*.sv; do
    top=$(grep -E '^module' "$d" | tail -1 | sed -E 's/^module +([A-Za-z0-9_]+).*/\1/')
    train "perf-$(basename "$d" .sv)" "$TDIR/release/xezim" --simulate -s "$top" "$d" --no-cache --max-time 2000000
  done
  for d in scripts/pgo-train/*.sv; do
    train "bundled-$(basename "$d" .sv)" "$TDIR/release/xezim" --simulate -s tb "$d" --no-cache
  done
  train xezim-bench "$TDIR/release/xezim-bench" --cycles 200000
fi
shopt -s nullglob
PROFILES=("$RAW_DIR"/xezim-*.profraw)
[ ${#PROFILES[@]} -gt 0 ] || { echo "training produced no profiles in $RAW_DIR" >&2; exit 1; }
"$PROFDATA" merge -o "$PDIR/merged.profdata" "${PROFILES[@]}"
"$PROFDATA" show "$PDIR/merged.profdata" > "$PDIR/profile-summary.txt"
echo "== optimized build =="
build_with_profile "-Cprofile-use=$PDIR/merged.profdata"
echo "PGO build complete: $TDIR/release/xezim"
echo "Profile, raw data, training logs and build metadata retained in $PDIR"
