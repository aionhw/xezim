#!/bin/bash
# ==============================================================================
# iverilog_run_tiers.sh — runs Tier 0 and Tier 1 tests on Icarus Verilog 12 as
# a second cross-check. (Icarus does not implement user-defined nettypes with
# resolvers, so Tier 2 cannot be run here.)
#
# Usage:
#   ./iverilog_run_tiers.sh            # run tier 0 and tier 1
#   ./iverilog_run_tiers.sh 37 39      # run only the listed tier numbers
#
# Each tier is its own vvp run; the script prints PASS/FAIL per tier and exits
# non-zero if any tier fails.
# ==============================================================================

set -u

IVL_BIN="${IVL_BIN:-$(command -v iverilog 2>/dev/null || true)}"
VVP_BIN="${VVP_BIN:-$(command -v vvp 2>/dev/null || true)}"
if [ -z "${IVL_BIN}" ] || [ -z "${VVP_BIN}" ]; then
  echo "ERROR: iverilog/vvp not on PATH." >&2
  exit 127
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
COMMON="${HERE}/../common"
WORK_DIR="${HERE}/ivl_work"
mkdir -p "${WORK_DIR}"

# Map tier number -> {file, top-module, label}
declare -A TIERS
TIERS[37]="37_z_skip_resolution.sv|test_37_tier0_z_skip_resolution|Tier 0 (z-skip)"
TIERS[39]="39_builtin_nettype_resolution.sv|test_39_tier1_builtin_nettype_resolution|Tier 1 (per-NetType)"

# Tiers 38 and 40 require `nettype` (user-defined nettypes) which Icarus 12
# does not implement. They are intentionally omitted from this runner.

# If the user passed tier numbers as args, use only those; otherwise run all.
if [ "$#" -gt 0 ]; then
  REQUESTED=("$@")
else
  REQUESTED=(37 39)
fi

PASS_TIERS=()
FAIL_TIERS=()

for tier in "${REQUESTED[@]}"; do
  if [ -z "${TIERS[$tier]:-}" ]; then
    echo "SKIP  tier ${tier}: not Icarus-runnable (likely needs nettype)"
    continue
  fi
  IFS='|' read -r SRC TOP LABEL <<< "${TIERS[$tier]}"

  echo "============================================================"
  echo "Tier ${tier}: ${SRC}  (${LABEL})"
  echo "Top  : ${TOP}"
  echo "============================================================"

  cd "${WORK_DIR}" || exit 1
  rm -f "tier${tier}.vvp" "tier${tier}.log"

  if ! "${IVL_BIN}" -g2012 -o "tier${tier}.vvp" \
        -I "${COMMON}" "${HERE}/${SRC}" 2> "tier${tier}.compile.log"; then
    echo "  COMPILE FAILED:"
    sed 's/^/    /' "tier${tier}.compile.log" | tail -20
    FAIL_TIERS+=("${tier}")
    cd "${HERE}"
    continue
  fi
  cd "${HERE}"

  # vvp 12 doesn't reliably write to a -l log file; route everything through
  # a tee so we have a stable copy regardless.
  : > "${WORK_DIR}/tier${tier}.log"
  if ! "${VVP_BIN}" "${WORK_DIR}/tier${tier}.vvp" 2>&1 \
       | tee -a "${WORK_DIR}/tier${tier}.log" > "${WORK_DIR}/tier${tier}.stdout"; then
    :  # vvp returns nonzero on $finish-with-error; we'll judge by the marker instead
  fi

  # Pull only the SVTEST pass/fail lines.
  grep -E "TEST_(PASS|FAIL)" "${WORK_DIR}/tier${tier}.log" \
      "${WORK_DIR}/tier${tier}.stdout" 2>/dev/null \
    || echo "  (no TEST_PASS/TEST_FAIL markers in output)"

  if grep -q "TEST_PASS" "${WORK_DIR}/tier${tier}.log" \
     "${WORK_DIR}/tier${tier}.stdout" 2>/dev/null; then
    echo "  TEST_RESULT: PASS"
    PASS_TIERS+=("${tier}")
  else
    echo "  TEST_RESULT: FAIL"
    FAIL_TIERS+=("${tier}")
    echo "  Last 25 lines of run log:"
    tail -25 "${WORK_DIR}/tier${tier}.log" "${WORK_DIR}/tier${tier}.stdout" 2>/dev/null \
      | sed 's/^/    /'
  fi
done

echo
echo "============================================================"
echo "Icarus SUMMARY"
echo "============================================================"
echo "Passed: ${#PASS_TIERS[@]}"
for t in "${PASS_TIERS[@]}"; do echo "  PASS  ${t}"; done
echo "Failed: ${#FAIL_TIERS[@]}"
for t in "${FAIL_TIERS[@]}"; do echo "  FAIL  ${t}"; done

if [ "${#FAIL_TIERS[@]}" -gt 0 ]; then
  exit 1
fi
exit 0