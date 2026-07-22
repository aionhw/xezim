#!/bin/bash
# ==============================================================================
# resolve_run_all.sh — top-level orchestrator. Runs the three resolution-tier
# tests on every available reference simulator and on xezim, and prints a
# cross-simulator comparison matrix.
#
# Usage:
#   ./resolve_run_all.sh
#   ./resolve_run_all.sh --skip-xezim      # reference only
#   ./resolve_run_all.sh --tiers 37 39     # specific tiers
#   XEZIM=/path/to/xezim ./resolve_run_all.sh
#
# Exit code:
#   0  if every requested simulator passes every requested tier
#   1  if any simulator fails any tier (i.e. xezim still has the bug)
#   2  if a reference simulator failed (test infrastructure is broken)
# ==============================================================================

set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
XEZIM="${XEZIM:-${HERE}/../../../target/debug/xezim}"
SKIP_XEZIM=0
TIERS=(37 38 39 40)

while [ "$#" -gt 0 ]; do
  case "$1" in
    --skip-xezim) SKIP_XEZIM=1; shift ;;
    --tiers) shift; TIERS=("$@"); break ;;
    -h|--help)
      sed -n '2,/^# ====/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "Unknown arg: $1" >&2; exit 64 ;;
  esac
done

# ---------------------------------------------------------------------------
# Stage 1: Icarus (Tier 0 + Tier 1 only; Tier 2 unsupported)
# ---------------------------------------------------------------------------
IVL_TIERS=()
for t in "${TIERS[@]}"; do
  # Tiers 38 and 40 require user-defined nettypes; Icarus doesn't support them.
  if [ "$t" != "38" ] && [ "$t" != "40" ]; then
    IVL_TIERS+=("$t")
  fi
done

declare -A RESULT_IVL
if [ "${#IVL_TIERS[@]}" -gt 0 ] && command -v iverilog >/dev/null 2>&1; then
  echo "============================================================"
  IVL_VER="$(iverilog -V 2>&1 | sed -n '1s/Icarus Verilog //p')"
  echo "Reference: Icarus Verilog ${IVL_VER:-<unknown>}"
  echo "============================================================"
  OUT="$("${HERE}/iverilog_run_tiers.sh" "${IVL_TIERS[@]}" 2>&1)"
  echo "$OUT" | tail -30
  for t in "${IVL_TIERS[@]}"; do
    if echo "$OUT" | grep -qE "PASS  $t\$"; then
      RESULT_IVL[$t]=PASS
    else
      RESULT_IVL[$t]=FAIL
    fi
  done
elif [ "${#IVL_TIERS[@]}" -gt 0 ]; then
  echo "(iverilog not on PATH; skipping Icarus reference)"
fi

# ---------------------------------------------------------------------------
# Stage 2: Questa (all three tiers)
# ---------------------------------------------------------------------------
declare -A RESULT_QST
if [ -x "$(command -v vsim 2>/dev/null || true)" ] || [ -n "${VSIM_BIN:-}" ]; then
  echo
  echo "============================================================"
  echo "Reference: QuestaSim-64"
  echo "============================================================"
  OUT="$("${HERE}/questa_run_tiers.sh" "${TIERS[@]}" 2>&1)"
  echo "$OUT" | tail -30
  for t in "${TIERS[@]}"; do
    if echo "$OUT" | grep -qE "PASS  $t\$"; then
      RESULT_QST[$t]=PASS
    else
      RESULT_QST[$t]=FAIL
    fi
  done
else
  echo "(vsim not on PATH; skipping Questa reference)"
fi

# ---------------------------------------------------------------------------
# Stage 3: xezim (the DUT)
# ---------------------------------------------------------------------------
declare -A RESULT_XZ
if [ "$SKIP_XEZIM" -eq 0 ]; then
  if [ ! -x "$XEZIM" ]; then
    echo
    echo "(xezim not found at ${XEZIM}; set XEZIM=... or pass --skip-xezim)"
  else
    echo
    echo "============================================================"
    echo "DUT: xezim (${XEZIM})"
    echo "============================================================"
    cd "$HERE"
    ANY_XZ_FAIL=0
    for t in "${TIERS[@]}"; do
      FILE=""
      case "$t" in
        37) FILE="37_z_skip_resolution.sv" ;;
        38) FILE="38_resolver_dispatch.sv" ;;
        39) FILE="39_builtin_nettype_resolution.sv" ;;
        40) FILE="40_struct_nettype_resolution.sv" ;;
      esac
      [ -z "$FILE" ] && continue
      LOG="/tmp/xezim_resolve_${t}.log"
      "$XEZIM" -I ../common "$FILE" > "$LOG" 2>&1
      if grep -q "TEST_PASS" "$LOG"; then
        RESULT_XZ[$t]=PASS
        echo "  Tier $t: PASS"
      elif grep -q "TEST_FAIL" "$LOG"; then
        RESULT_XZ[$t]="FAIL($(grep -oE 'TEST_FAIL count=[0-9]+' "$LOG" | head -1 | cut -d= -f2))"
        echo "  Tier $t: FAIL — $(grep -oE 'TEST_FAIL count=[0-9]+' "$LOG" | head -1)"
        ANY_XZ_FAIL=1
      else
        RESULT_XZ[$t]=ERROR
        echo "  Tier $t: ERROR (no marker)"
        ANY_XZ_FAIL=1
      fi
    done
    XEZIM_FAILED=$ANY_XZ_FAIL
  fi
fi

# ---------------------------------------------------------------------------
# Summary matrix
# ---------------------------------------------------------------------------
echo
echo "============================================================"
echo "SUMMARY MATRIX"
echo "============================================================"
printf "%-6s | %-12s | %-12s | %-12s\n" "Tier" "Icarus" "Questa" "xezim"
printf "%-6s-+-%-12s-+-%-12s-+-%-12s\n" "------" "------------" "------------" "------------"
for t in "${TIERS[@]}"; do
  IVL="${RESULT_IVL[$t]:--}"
  QST="${RESULT_QST[$t]:--}"
  XZ="${RESULT_XZ[$t]:--}"
  printf "%-6s | %-12s | %-12s | %-12s\n" "$t" "$IVL" "$QST" "$XZ"
done

# Reference-side failure is a test infrastructure bug (exit 2)
for t in "${TIERS[@]}"; do
  [ "${RESULT_IVL[$t]:-}" = "FAIL" ] && exit 2
  [ "${RESULT_QST[$t]:-}" = "FAIL" ] && exit 2
done

# xezim-side failure is the expected state until Tiers 0–2 land (exit 1).
[ "${XEZIM_FAILED:-0}" -eq 1 ] && exit 1
exit 0