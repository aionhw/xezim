# run_tier0.do — Questa batch-mode driver for Tier 0 (z-skip).
#
# Launch from Questa with:
#   vsim -c -do run_tier0.do
#
# Or from the GUI's Transcript pane:
#   do run_tier0.do
#
# Exit code is 0 on TEST_PASS, non-zero on any failure.

if { ![file isdirectory work] } { vlib work }
vlog -sv -mfcu -work work +incdir+../common 37_z_skip_resolution.sv
vsim -voptargs="+acc" -c -do "run -all; quit -f" -l run_tier0.log test_37_tier0_z_skip_resolution

set pass_count [regexp -inline {TEST_PASS} run_tier0.log]
if {[llength $pass_count] > 0} {
  echo "TIER0_RESULT: PASS"
  quit -f
} else {
  echo "TIER0_RESULT: FAIL"
  echo "--- run_tier0.log tail ---"
  exec tail -n 30 run_tier0.log
  quit -code 1
}
