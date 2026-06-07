#!/usr/bin/env bash
# Run the cv32e40p (core-v-verif / OpenHW Group) UVM testbench under xezim.
#
# Prerequisites (paths are the defaults used during bring-up):
#   /home/bondan/repo/sv2023/core-v-verif    (OpenHW core-v-verif checkout)
#   /home/bondan/repo/sv2023/cv32e40p        (OpenHW cv32e40p RTL checkout)
#   /home/bondan/repo/sv2023/uvm-1.2         (Accellera UVM 1.2 reference)
#   /home/bondan/repo/riscv/rtl/include      (for `riscv_config.sv`)
#
# Usage:
#   ./scripts/run_cv32e40p_uvm.sh                       # default base test
#   ./scripts/run_cv32e40p_uvm.sh +UVM_TESTNAME=...     # any UVM test
#
# Mode: --compile (parse + elaborate) by default. Pass --simulate as the
# first arg to actually start UVM phases.

set -u
MODE="--compile"
if [[ "${1:-}" == "--simulate" || "${1:-}" == "-sim" ]]; then
    MODE="--simulate"
    shift
fi

XEZIM=${XEZIM:-$(dirname "$(readlink -f "$0")")/../target/release/xezim}
CVV=${CVV:-/home/bondan/repo/sv2023/core-v-verif}
UVM=${UVM:-/home/bondan/repo/sv2023/uvm-1.2}
CV32=${CV32:-/home/bondan/repo/sv2023/cv32e40p}
RVCFG=${RVCFG:-/home/bondan/repo/riscv/rtl/include}
CV_CORE_LC=${CV_CORE_LC:-cv32e40p}

# 25 export vars expected by core-v-verif filelists.
export CORE_V_VERIF=$CVV
export CV_CORE=cv32e40p
export CV_CORE_LC
export TBSRC_HOME=$CVV/$CV_CORE_LC/tb
export DESIGN_RTL_DIR=$CV32/rtl
export DV_UVMT_PATH=$CVV/$CV_CORE_LC/tb/uvmt
export DV_UVME_PATH=$CVV/$CV_CORE_LC/env/uvme
export DV_UVML_HRTBT_PATH=$CVV/lib/uvm_libs/uvml_hrtbt
export DV_UVMA_CORE_CNTRL_PATH=$CVV/lib/uvm_agents/uvma_core_cntrl
export DV_UVMA_ISACOV_PATH=$CVV/lib/uvm_agents/uvma_isacov
export DV_UVMA_RVFI_PATH=$CVV/lib/uvm_agents/uvma_rvfi
export DV_UVMA_RVVI_PATH=$CVV/lib/uvm_agents/uvma_rvvi
export DV_UVMA_RVVI_OVPSIM_PATH=$CVV/lib/uvm_agents/uvma_rvvi_ovpsim
export DV_UVMA_CLKNRST_PATH=$CVV/lib/uvm_agents/uvma_clknrst
export DV_UVMA_INTERRUPT_PATH=$CVV/lib/uvm_agents/uvma_interrupt
export DV_UVMA_DEBUG_PATH=$CVV/lib/uvm_agents/uvma_debug
export DV_UVMA_PMA_PATH=$CVV/lib/uvm_agents/uvma_pma
export DV_UVMA_OBI_MEMORY_PATH=$CVV/lib/uvm_agents/uvma_obi_memory
export DV_UVMA_FENCEI_PATH=$CVV/lib/uvm_agents/uvma_fencei
export DV_UVML_TRN_PATH=$CVV/lib/uvm_libs/uvml_trn
export DV_UVML_LOGS_PATH=$CVV/lib/uvm_libs/uvml_logs
export DV_UVML_SB_PATH=$CVV/lib/uvm_libs/uvml_sb
export DV_UVML_MEM_PATH=$CVV/lib/uvm_libs/uvml_mem
export DV_UVMC_RVFI_SCOREBOARD_PATH=$CVV/lib/uvm_components/uvmc_rvfi_scoreboard
export DV_UVMC_RVFI_REFERENCE_MODEL_PATH=$CVV/lib/uvm_components/uvmc_rvfi_reference_model
export DV_SVLIB_PATH=$CVV/$CV_CORE_LC/vendor_lib/verilab
export DV_OVPM_HOME=$CVV/vendor_lib/imperas
export DV_OVPM_DESIGN=$DV_OVPM_HOME/design
export DV_OVPM_MODEL=$DV_OVPM_HOME/imperas_DV_COREV

UVM_TEST_ARG="+UVM_TESTNAME=uvmt_cv32e40p_base_test_c"
EXTRA_ARGS=()
for arg in "$@"; do
    if [[ "$arg" == +UVM_TESTNAME=* ]]; then
        UVM_TEST_ARG="$arg"
    else
        EXTRA_ARGS+=("$arg")
    fi
done

echo "Mode      : $MODE"
echo "Test      : $UVM_TEST_ARG"
echo "Binary    : $XEZIM"

exec "$XEZIM" $MODE \
    --max-time 20000000 \
    -s uvmt_cv32e40p_tb \
    -DUVM_NO_DPI \
    -DUVM \
    -I $UVM/src \
    -I $DV_UVMT_PATH \
    -I $RVCFG \
    -I $DESIGN_RTL_DIR/include \
    -f $DV_UVMT_PATH/uvmt_cv32e40p.flist \
    -f $DV_UVMT_PATH/imperas_iss.flist \
    -f $CV32/cv32e40p_manifest.flist \
    $CV32/scripts/lint/cv32e40p_wrapper.sv \
    $UVM/src/uvm_pkg.sv \
    $DV_UVMT_PATH/uvmt_cv32e40p_tb.sv \
    "$UVM_TEST_ARG" \
    "${EXTRA_ARGS[@]}"
