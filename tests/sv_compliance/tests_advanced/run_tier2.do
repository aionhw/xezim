# run_tier2.do — Questa batch-mode driver for Tier 2 (resolver dispatch).
#
# This tier needs `nettype ... with <fn>`, which Questa supports.
#
# Launch with:
#   vsim -c -do run_tier2.do

if { ![file isdirectory work] } { vlib work }
vlog -sv -mfcu -work work +incdir+../common 38_resolver_dispatch.sv
vsim -voptargs="+acc" -c -do "run -all; quit -f" -l run_tier2.log test_38_tier2_resolver_dispatch

set pass_count [regexp -inline {TEST_PASS} run_tier2.log]
if {[llength $pass_count] > 0} {
  echo "TIER2_RESULT: PASS"
  quit -f
} else {
  echo "TIER2_RESULT: FAIL"
  echo "--- run_tier2.log tail ---"
  exec tail -n 30 run_tier2.log
  quit -code 1
}
