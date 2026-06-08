#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PICORV32_DIR="${PICORV32_DIR:-/home/bondan/agent/claude/repo/picorv32}"
PORT="${PORT:-6972}"
TB_SRC="${PICORV32_DIR}/testbench_ez.v"
PICORV32_V="${PICORV32_DIR}/picorv32.v"
TB_TMP="${TB_TMP:-/tmp/testbench_ez_xezim_surfer.v}"
VCD_OUT="${VCD_OUT:-/tmp/picorv32_test_ez_xezim_surfer.vcd}"
LOG="${LOG:-/tmp/xezim_surfer_plugin_picorv32_ez_${PORT}.log}"

if [[ ! -f "$TB_SRC" ]]; then
  echo "missing PicoRV32 testbench: $TB_SRC" >&2
  exit 1
fi

if [[ ! -f "$PICORV32_V" ]]; then
  echo "missing PicoRV32 RTL: $PICORV32_V" >&2
  exit 1
fi

cd "$ROOT"

echo "[build] xezim-surfer-plugin and synthetic-surfer"
cargo build --bin xezim-surfer-plugin --bin synthetic-surfer

echo "[prepare] temporary xezim-compatible EZ testbench: $TB_TMP"
awk -v vcd="$VCD_OUT" '
  /\$dumpfile\("testbench\.vcd"\);/ {
    print "\t\t\t$dumpfile(\"" vcd "\");";
    next;
  }
  /\$dumpvars\(0, testbench\);/ {
    print "\t\t\t$dumpvars;";
    next;
  }
  { print }
' "$TB_SRC" > "$TB_TMP"

rm -f "$LOG"

echo "[plugin] starting on 127.0.0.1:${PORT}"
target/debug/xezim-surfer-plugin \
  --port "$PORT" \
  -s testbench \
  "$TB_TMP" \
  "$PICORV32_V" \
  +vcd \
  >"$LOG" 2>&1 &
PLUGIN_PID=$!

cleanup() {
  kill "$PLUGIN_PID" 2>/dev/null || true
  wait "$PLUGIN_PID" 2>/dev/null || true
}
trap cleanup EXIT

for _ in $(seq 1 100); do
  if grep -q "listening on 127.0.0.1:${PORT}" "$LOG"; then
    break
  fi
  if ! kill -0 "$PLUGIN_PID" 2>/dev/null; then
    echo "plugin exited before listening" >&2
    cat "$LOG" >&2
    exit 1
  fi
  sleep 0.1
done

if ! grep -q "listening on 127.0.0.1:${PORT}" "$LOG"; then
  echo "plugin did not start listening on port $PORT" >&2
  cat "$LOG" >&2
  exit 1
fi

echo "[synthetic-surfer] run PicoRV32 EZ to completion"
target/debug/synthetic-surfer --addr "127.0.0.1:${PORT}" ez

echo
echo "[plugin log] $LOG"
tail -n 20 "$LOG"
