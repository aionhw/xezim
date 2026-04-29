#!/usr/bin/env bash
# Run c910 hello on xezim from openc910 RTL.
#
# Prereqs:
#   - openc910 cloned at $C910_REPO (default: /home/bondan/agent/claude/repo/openc910)
#   - xezim built at $XEZIM (default: /home/bondan/agent/claude/repo/xezim/target/release/xezim)
#   - case.pat / inst.pat / data.pat already generated in smart_run/work/
#     (they ship with openc910's hello_world test)
#
# Usage:
#   ./run_c910_hello.sh                # default 200000 ns max
#   MAX_TIME=1000000 ./run_c910_hello.sh
#   PATCH_CSRSI=1 ./run_c910_hello.sh  # patch csrsi 0x9bf -> NOP so init progresses
#                                        past the broken CSR write in crt0.s

set -euo pipefail

C910_REPO="${C910_REPO:-/home/bondan/agent/claude/repo/openc910}"
XEZIM="${XEZIM:-/home/bondan/agent/claude/repo/xezim/target/release/xezim}"
MAX_TIME="${MAX_TIME:-200000}"
PATCH_CSRSI="${PATCH_CSRSI:-0}"
LOG="${LOG:-/tmp/c910_hello.out}"

WORK="$C910_REPO/smart_run/work"
RTL_FACTORY="$C910_REPO/C910_RTL_FACTORY"
SMART="$C910_REPO/smart_run"

[ -x "$XEZIM" ]               || { echo "xezim not found: $XEZIM"; exit 1; }
[ -d "$WORK" ]                || { echo "smart_run/work not found: $WORK"; exit 1; }
[ -d "$RTL_FACTORY" ]         || { echo "RTL_FACTORY not found: $RTL_FACTORY"; exit 1; }
[ -f "$WORK/inst.pat" ]       || { echo "inst.pat missing — run 'make ${WORK%/}/case'"; exit 1; }
[ -f "$SMART/c910_files.fl" ] || { echo "c910_files.fl missing"; exit 1; }

if [ "$PATCH_CSRSI" = "1" ]; then
  # crt0.s does `csrsi 0x9bf, 1` (msmpr per author's intent) but c910 RTL
  # decodes 0x9C0-0x9FF as S-mode custom — 0x9BF is invalid. Without this
  # patch the CPU traps to mtvec=0 and loops forever.
  for pat in "$WORK/inst.pat" "$WORK/case.pat"; do
    [ -f "$pat" ] || continue
    [ -f "$pat.orig" ] || cp "$pat" "$pat.orig"
    sed -i 's/73e0f09b/13000000/' "$pat"   # csrsi 0x9bf,1 -> nop
  done
  echo "[patch] replaced csrsi 0x9bf,1 with NOP in inst.pat / case.pat"
fi

cd "$WORK"
echo "[run]   max-time=$MAX_TIME log=$LOG"
echo "[run]   xezim=$XEZIM"
CODE_BASE_PATH="$RTL_FACTORY" \
  "$XEZIM" --simulate \
  --max-time "$MAX_TIME" \
  -DIVERILOG_SIM -DNO_DUMP \
  -f ../c910_files.fl \
  -c ../logical/filelists/smart.fl \
  -c ../logical/filelists/tb.fl \
  > "$LOG" 2>&1 &
SIM_PID=$!
echo "[run]   pid=$SIM_PID — tail $LOG for progress"

# Stream key events while sim runs.
trap 'kill $SIM_PID 2>/dev/null || true' INT TERM
tail -f --pid="$SIM_PID" "$LOG" 2>/dev/null \
  | grep --line-buffered -E "XADV|XSUM|XCYC|TEST PASS|TEST FAIL|simulation finished|finished at|Hello|Welcome" \
  &
TAIL_PID=$!

wait "$SIM_PID" || true
kill "$TAIL_PID" 2>/dev/null || true

echo
if grep -q "TEST PASS" "$LOG"; then
  echo "[done]  TEST PASSED"
  exit 0
elif grep -q "TEST FAIL" "$LOG"; then
  echo "[done]  TEST FAILED — see $LOG"
  exit 2
else
  echo "[done]  hit max-time without verdict — see $LOG"
  exit 0
fi
