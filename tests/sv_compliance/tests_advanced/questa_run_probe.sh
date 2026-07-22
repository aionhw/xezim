#!/bin/bash
# ==============================================================================
# questa_run_probe.sh — runs the resolution probe once on Questa and prints
# the actual values Questa assigns to every built-in net-type configuration.
#
# Usage:
#   ./questa_run_probe.sh
#
# Output:
#   run_probe.log in questa_work/, plus a stdout pass/fail banner. After
#   running, paste the `PROBE BEGIN / PROBE END` block from run_probe.log
#   to ground the tier tests in Questa's actual behavior.
# ==============================================================================

set -u

VSIM_BIN="${VSIM_BIN:-$(command -v vsim 2>/dev/null || true)}"
if [ -z "${VSIM_BIN}" ]; then
  echo "ERROR: 'vsim' not found on PATH. Set VSIM_BIN or activate Questa." >&2
  exit 127
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
COMMON="${HERE}/../common"
WORK_DIR="${HERE}/questa_work"
mkdir -p "${WORK_DIR}"
cd "${WORK_DIR}" || exit 1

echo "============================================================"
echo "Resolution Probe"
echo "Src: ${HERE}/probe_questa_resolution.sv"
echo "============================================================"

rm -rf ./* 2>/dev/null
vlib work >/dev/null 2>&1
vlog -sv -mfcu +incdir+"${COMMON}" \
     -work work "${HERE}/probe_questa_resolution.sv" 2>&1 | grep -iE "error|warn" || true

rm -f transcript run_probe.log
set +e
vsim -voptargs="+acc" -c \
     -do "run -all; quit -f" -l run_probe.log probe 2>&1 | tail -30
set -e

echo
echo "============================================================"
echo "PROBE LOG run_probe.log"
echo "============================================================"
grep -E "==|=|single-driver|2-driver|3-driver|all-z|wand|wor|trior|tri0|tri1|supply" \
     run_probe.log | grep -v "^#" || true

echo
echo "============================================================"
echo "Full log: ${WORK_DIR}/run_probe.log"
echo "============================================================"

if grep -q "PROBE END" run_probe.log; then
  echo "PROBE_RESULT: OK"
  exit 0
else
  echo "PROBE_RESULT: FAIL"
  exit 1
fi
