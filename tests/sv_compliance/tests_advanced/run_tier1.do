# run_tier1.do — Questa batch-mode driver for Tier 1 (per-NetType fold).
#
# Launch with:
#   vsim -c -do run_tier1.do

if { ![file isdirectory work] } { vlib work }
vlog -sv -mfcu -work work +incdir+../common 39_builtin_nettype_resolution.sv
vsim -voptargs="+acc" -c -do "run -all; quit -f" -l run_tier1.log test_39_tier1_builtin_nettype_resolution

set pass_count [regexp -inline {TEST_PASS} run_tier1.log]
if {[llength $pass_count] > 0} {
  echo "TIER1_RESULT: PASS"
  quit -f
} else {
  echo "TIER1_RESULT: FAIL"
  echo "--- run_tier1.log tail ---"
  exec tail -n 30 run_tier1.log
  quit -code 1
}
