#!/usr/bin/env bash
# Build `uvm.so` against xezim's include/ headers.
#
#   Outputs: ./uvm.so (in the directory you invoke this from, by default)
#
# Why this script exists:
#   - uvm-core/src/dpi/uvm_dpi.cc unconditionally includes uvm_hdl.c,
#     which has VCS/Questa/Xcelium-only branches requiring proprietary
#     vendor headers (vcsuser.h, etc.). xezim can't ship those.
#   - uvm_dpi_xezim.cc (shipped in xezim/include/) is a from-scratch
#     drop-in: same include chain as uvm_dpi.cc, but with uvm_hdl.c
#     skipped and an in-tree uvm_hdl_* implementation built against
#     standard IEEE 1800 VPI.
#   - uvm_hdl_polling.c has a long-standing `-Wformat-security`
#     warning that every commercial simulator's UVM build suppresses.
#
# Usage:
#   ./scripts/build_uvm_so.sh                       # builds ./uvm.so here
#   UVM=/path/to/uvm-core ./scripts/build_uvm_so.sh  # custom UVM checkout
#   OUT=/path/to/uvm.so  ./scripts/build_uvm_so.sh  # custom output path

set -euo pipefail

XEZIM_INCLUDE=${XEZIM_INCLUDE:-$(dirname "$(readlink -f "$0")")/../include}
UVM=${UVM:-$(dirname "$(readlink -f "$0")")/../../uvm-core/src/dpi}
OUT=${OUT:-./uvm.so}

if [[ ! -f "$XEZIM_INCLUDE/uvm_dpi_xezim.cc" ]]; then
    echo "error: $XEZIM_INCLUDE/uvm_dpi_xezim.cc not found" >&2
    echo "  set XEZIM_INCLUDE=/path/to/xezim/include" >&2
    exit 1
fi

if [[ ! -f "$UVM/uvm_dpi.h" ]]; then
    echo "error: $UVM/uvm_dpi.h not found" >&2
    echo "  set UVM=/path/to/uvm-core/src/dpi" >&2
    exit 1
fi

echo "xezim include : $XEZIM_INCLUDE"
echo "uvm source    : $UVM"
echo "output        : $OUT"

g++ -shared -fPIC -std=c++17 -Wno-format-security \
    -I "$XEZIM_INCLUDE" -I "$UVM" \
    "$XEZIM_INCLUDE/uvm_dpi_xezim.cc" \
    -o "$OUT"

echo
echo "OK: $OUT built successfully"
echo "    verify with: nm -D $OUT | grep uvm_hdl_check_path"
