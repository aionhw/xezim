# run_probe.do — runs the resolution probe once and prints Questa's actual values.
#
# Launch with:
#   vsim -c -do run_probe.do

if { ![file isdirectory work] } { vlib work }
vlog -sv -mfcu -work work +incdir+../common probe_questa_resolution.sv
vsim -voptargs="+acc" -c -do "run -all; quit -f" -l run_probe.log probe

set pass_count [regexp -inline {PROBE END} run_probe.log]
if {[llength $pass_count] > 0} {
  echo "PROBE_RESULT: OK"
  quit -f
} else {
  echo "PROBE_RESULT: FAIL"
  echo "--- run_probe.log tail ---"
  exec tail -n 40 run_probe.log
  quit -code 1
}
