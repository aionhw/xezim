#!/usr/bin/env bash
# Verify a design is deterministic: run it twice with the same seed and fail
# if the outputs differ. Identical inputs must give byte-identical output.
#
#   scripts/dev/check-determinism.sh design.sv
#   scripts/dev/check-determinism.sh design.sv +seed=7 --max-time 200
#
# Pass +seed=<n> (default is seed 1). Use +seed=random only if you want to
# confirm your analysis ignores entropy — two random runs SHOULD differ.
set -uo pipefail

if [ "$#" -lt 1 ] || [ ! -f "$1" ]; then
  echo "usage: $(basename "$0") <design.sv> [xezim args...]" >&2
  exit 2
fi
design="$1"
shift

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="$repo/target/release/xezim"
if [ ! -x "$bin" ]; then
  echo "release binary missing — run: cargo build --release" >&2
  exit 2
fi

r1="$(mktemp)"; r2="$(mktemp)"
trap 'rm -f "$r1" "$r2"' EXIT

"$bin" "$design" "$@" 2>&1 >"$r1"; s1=$?
"$bin" "$design" "$@" 2>&1 >"$r2"; s2=$?

if [ "$s1" -ne "$s2" ]; then
  echo "DETERMINISM FAILURE: exit codes differ ($s1 vs $s2)" >&2
  exit 1
fi
if ! diff -q "$r1" "$r2" >/dev/null; then
  echo "DETERMINISM FAILURE: two identical runs produced different output" >&2
  diff -u "$r1" "$r2" >&2
  exit 1
fi
echo "DETERMINISM OK: byte-identical (exit $s1)"
