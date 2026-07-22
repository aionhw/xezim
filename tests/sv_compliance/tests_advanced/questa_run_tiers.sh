#!/bin/bash
# ==============================================================================
# questa_run_tiers.sh — runs the Tier 0 / 1 / 2 resolution test suite under
# Questa (Mentor Graphics / Siemens EDA) SystemVerilog simulator.
#
# Usage:
#   ./questa_run_tiers.sh                       # run all three tiers
#   ./questa_run_tiers.sh 37                    # run a single tier (37, 38, 39)
#   ./questa_run_tiers.sh 37 38                 # run multiple tiers
#   VSIM_BIN=vsim ./questa_run_tiers.sh         # point to a different binary
#
# Output:
#   Per-test transcript (TEST_PASS / TEST_FAIL count=N) followed by an
#   aggregate PASS/FAIL summary. Exit code 0 if every requested test
#   passed, non-zero otherwise (so this is CI-friendly).
#
# Prerequisites:
#   - Questa installed and `vsim` on PATH (or set VSIM_BIN).
#   - Run from this directory (tests_advanced/).
# ==============================================================================

set -u

# ---- Locate Questa ---------------------------------------------------------
VSIM_BIN="${VSIM_BIN:-$(command -v vsim 2>/dev/null || true)}"
if [ -z "${VSIM_BIN}" ]; then
  echo "ERROR: 'vsim' not found on PATH. Set VSIM_BIN or activate a Questa install." >&2
  exit 127
fi

# Resolve the absolute path to this script's directory so we can be robust to
# being invoked from anywhere.
HERE="$(cd "$(dirname "$0")" && pwd)"
COMMON="${HERE}/../common"
TESTS_DIR="${HERE}"

# ---- Tier-to-file mapping --------------------------------------------------
TIER_FILES=(
  "37|test_37_tier0_z_skip_resolution|37_z_skip_resolution.sv"
  "38|test_38_tier2_resolver_dispatch|38_resolver_dispatch.sv"
  "39|test_39_tier1_builtin_nettype_resolution|39_builtin_nettype_resolution.sv"
  "40|test_40_tier2_struct_nettype_resolution|40_struct_nettype_resolution.sv"
)

# Map argument -> list of indices to run.
TIER_ARG="$*"
RUN_LIST=()
if [ -z "${TIER_ARG}" ]; then
  for tier in 37 38 39 40; do RUN_LIST+=("${tier}"); done
else
  for arg in ${TIER_ARG}; do
    matched=0
    for tier in 37 38 39 40; do
      if [ "${arg}" = "${tier}" ]; then matched=1; break; fi
    done
    if [ "${matched}" -eq 0 ]; then
      echo "ERROR: unknown tier '${arg}' (valid: 37 38 39 40)" >&2
      exit 2
    fi
    RUN_LIST+=("${arg}")
  done
fi

# ---- Per-test run ---------------------------------------------------------
WORK_DIR="${HERE}/questa_work"
mkdir -p "${WORK_DIR}"
cd "${WORK_DIR}" || exit 1

PASS_TESTS=()
FAIL_TESTS=()

run_one() {
  local tier="$1"
  local top="$2"
  local src="$3"

  echo
  echo "============================================================"
  echo "Tier ${tier}: ${src}"
  echo "Top  : ${top}"
  echo "Cmd  : vsim -c ${top}"
  echo "============================================================"

  # Compile.
  rm -rf ./* 2>/dev/null
  vlib work >/dev/null 2>&1
  if ! vlog -sv -mfcu +incdir+"${COMMON}" \
           -work work "${TESTS_DIR}/${src}" 2>&1 | tee compile.log; then
    echo "  COMPILE_FAILED"
    FAIL_TESTS+=("${tier}")
    return 1
  fi

  # Run.
  rm -f transcript run.log
  set +e
  vsim -c -voptargs="+acc" \
        -do "add wave -r /test_bench/*; run -all; quit -f" \
        -l run.log "${top}" 2>&1 | tee transcript
  vsim_status=$?
  set -e
  echo "vsim exit status: ${vsim_status}"

  # Check log for pass/fail markers.
  if grep -q "TEST_PASS" run.log; then
    echo "  TEST_RESULT: PASS"
    PASS_TESTS+=("${tier}")
    return 0
  else
    echo "  TEST_RESULT: FAIL"
    echo "  Run-log tail:"
    tail -n 30 run.log | sed 's/^/    /'
    FAIL_TESTS+=("${tier}")
    return 1
  fi
}

# Run the requested tiers.
for tier in "${RUN_LIST[@]}"; do
  for entry in "${TIER_FILES[@]}"; do
    IFS='|' read -r t top src <<<"${entry}"
    if [ "${tier}" = "${t}" ]; then
      run_one "${t}" "${top}" "${src}" || true
      break
    fi
  done
done

# ---- Aggregate summary -----------------------------------------------------
echo
echo "============================================================"
echo "SUMMARY"
echo "============================================================"
echo "Passed: ${#PASS_TESTS[@]}"
for t in "${PASS_TESTS[@]:-}"; do echo "  PASS  ${t}"; done
echo "Failed: ${#FAIL_TESTS[@]}"
for t in "${FAIL_TESTS[@]:-}"; do echo "  FAIL  ${t}"; done
echo "============================================================"

if [ "${#FAIL_TESTS[@]}" -gt 0 ]; then
  exit 1
fi
exit 0
# vim: ts=2 sw=2 et

