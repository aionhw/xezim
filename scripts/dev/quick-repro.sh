#!/usr/bin/env bash
# Simulate a one-off SystemVerilog snippet without writing a repo file.
#
#   scripts/dev/quick-repro.sh 'module tb; initial $display("hi"); endmodule'
#   scripts/dev/quick-repro.sh 'module tb; ... endmodule' --max-time 50 +seed=2
#
# The snippet is written to a temp .sv file, simulated with the release
# binary, and the temp file is removed on exit. Exit code = xezim's.
set -uo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $(basename "$0") '<sv-snippet>' [xezim args...]" >&2
  exit 2
fi
snippet="$1"
shift

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="$repo/target/release/xezim"
if [ ! -x "$bin" ]; then
  echo "release binary missing — run: cargo build --release" >&2
  exit 2
fi

tmp="$(mktemp "${TMPDIR:-/tmp}/xz-repro-XXXXXX.sv")"
trap 'rm -f "$tmp"' EXIT
printf '%s\n' "$snippet" >"$tmp"
# Note: no `exec` here — the EXIT trap would never fire once the shell is
# replaced, leaking the temp file. The bin's exit status is preserved because
# it is the script's last command, and the trap runs after it exits.
"$bin" "$tmp" "$@"
