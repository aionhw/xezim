#!/usr/bin/env bash
# Run the XuanTie-C910 hello_world test on xezim with XTrace dumping
# restricted to CPU core 0 (the x_ct_top_0 instance).
#
# Output (left in $WORK_DIR):
#   c910_hello_cpu0.xt    XTrace dump — only signals under cpu0
#   c910_hello_cpu0.log   full xezim stdout + PROF stats
#   c910.fl               generated absolute-path filelist
#
# Exit 0 on TEST PASSED, 1 otherwise.
#
# Usage:
#   ./run_c910_hello_xtrace.sh
#   XEZIM=/path/to/xezim ./run_c910_hello_xtrace.sh
#   WORK_DIR=/tmp/c910x ./run_c910_hello_xtrace.sh
#
set -euo pipefail

# ---- Configuration (override via env) ---------------------------------------
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null && pwd -P)"
DEPS_DIR="${DEPS_DIR:-${SCRIPT_DIR}/deps}"

XEZIM="${XEZIM:-${DEPS_DIR}/xezim/target/release/xezim}"
DESIGN_DIR="${DESIGN_DIR:-${DEPS_DIR}/rtlmeter/designs/XuanTie-C910}"
WORK_DIR="${WORK_DIR:-${SCRIPT_DIR}/work}"
MAX_TIME="${MAX_TIME:-80000000}"     # ns of sim time before xezim aborts

# Hierarchical scope of CPU core 0 inside the C910 SoC:
#   tb -> x_soc -> x_cpu_sub_system_axi -> x_rv_integration_platform
#      -> x_cpu_top (openC910) -> x_ct_top_0   <-- core 0   (x_ct_top_1 = core 1)
# xezim's --xtrace-scope keeps a signal when its dotted name equals this
# string or begins with "<scope>." — so this captures cpu0's whole subtree.
CPU0_SCOPE="${CPU0_SCOPE:-x_soc.x_cpu_sub_system_axi.x_rv_integration_platform.x_cpu_top.x_ct_top_0}"

# ---- Sanity checks ----------------------------------------------------------
[[ -x "${XEZIM}" ]] || {
  echo "xezim binary not found / not executable: ${XEZIM}" >&2
  echo "Run ./setup.sh first, or set XEZIM=/path/to/xezim." >&2
  exit 2; }
[[ -d "${DESIGN_DIR}/src" ]]        || { echo "C910 src not found: ${DESIGN_DIR}/src" >&2; exit 2; }
[[ -d "${DESIGN_DIR}/tests/hello" ]] || { echo "hello test dir not found in ${DESIGN_DIR}" >&2; exit 2; }

# Resolve symlinks so the absolute paths written into the filelist stay valid
# even when DESIGN_DIR is reached through a symlink.
DESIGN_DIR="$(cd "${DESIGN_DIR}" && pwd -P)"

# ---- Stage the work dir -----------------------------------------------------
mkdir -p "${WORK_DIR}"
cd "${WORK_DIR}"

cp -f "${DESIGN_DIR}/tests/hello/data.pat" .
cp -f "${DESIGN_DIR}/tests/hello/inst.pat" .

# --- The one modification: empty __rtlmeter_top_include.vh stub --------------
# tb.v contains `include "__rtlmeter_top_include.vh"`. The real include (in
# rtlmeter/rtl/) instantiates __rtlmeter_utils, whose
#     longint unsigned max_cycles = '1;
# xezim evaluates to 0 — so the module's `cycles >= max_cycles` check fires
# $finish at t=0, before the CPU runs. An empty stub here (found first because
# WORK_DIR is the first -I entry, and rtlmeter/rtl/ is deliberately NOT on -I)
# lets tb.v compile without pulling in that module.
: > __rtlmeter_top_include.vh

# ---- Generate the Verilog filelist from descriptor.yaml ---------------------
python3 - "${DESIGN_DIR}" > c910.fl <<'PY'
import yaml, os, sys
d = sys.argv[1]
descr = yaml.safe_load(open(os.path.join(d, "descriptor.yaml")))
for f in descr["compile"]["verilogSourceFiles"]:
    print(os.path.join(d, f))
PY

# ---- Run xezim --------------------------------------------------------------
LOG="${WORK_DIR}/c910_hello_cpu0.log"
XTRACE="${WORK_DIR}/c910_hello_cpu0.xt"
rm -f "${LOG}" "${XTRACE}"

echo "==> xezim --simulate --xtrace  (scope restricted to cpu0)"
echo "    xezim    : ${XEZIM}"
echo "    design   : ${DESIGN_DIR}"
echo "    scope    : ${CPU0_SCOPE}"
echo "    work dir : ${WORK_DIR}"
echo "    log file : ${LOG}"

# stdbuf keeps the log line-buffered so progress is visible while running.
stdbuf -oL "${XEZIM}" \
  --simulate \
  --max-time "${MAX_TIME}" \
  -s tb \
  --xtrace "${XTRACE}" \
  --xtrace-scope "${CPU0_SCOPE}" \
  -I "${WORK_DIR}" \
  -I "${DESIGN_DIR}/src" \
  -f c910.fl \
  &> "${LOG}"

# ---- Report -----------------------------------------------------------------
echo "==> result:"
grep -E "XTrace\] dumping|Hello|TEST|simulation finished|finished at time" "${LOG}" \
  | sed 's/^/    /'

if grep -q "TEST PASSED" "${LOG}"; then
  echo
  echo "==> xtrace file: $(ls -la "${XTRACE}" | awk '{printf "%s bytes  %s\n", $5, $NF}')"
  exit 0
fi

echo "FAILED — full log at ${LOG}" >&2
exit 1
