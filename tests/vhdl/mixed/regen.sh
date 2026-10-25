#!/usr/bin/env bash
# Regenerate the golden output of the mixed-language designs here with
# Questa (vcom -2008, vlog -sv, vsim): <name>.golden is the simulation's
# transcript without Questa's own lines. Needs vlib/vcom/vlog/vsim
# (tested with Questa 2021.2). `./regen.sh <name>` regenerates one design.
set -u
cd "$(dirname "$0")"
here=$PWD
for sv in ${1:-*}.sv; do
  n=${sv%.sv}
  work=$(mktemp -d)
  (cd "$work" && vlib work >/dev/null &&
    vcom -2008 -quiet "$here/$n.vhd" &&
    vlog -sv -quiet "$here/$n.sv" &&
    vsim -c -quiet -voptargs=+acc -do "run -all; quit -f" "$n") > "$work/log" 2>&1 ||
    { echo "Questa failed on $n"; cat "$work/log"; rm -rf "$work"; continue; }
  # The transcript after `run -all`, without Questa's notes on $finish, its
  # end-of-run summary and the echo of `quit`.
  sed -n '/^# run -all/,$p' "$work/log" | sed '1d' |
    grep -v -E '^# (\*\* Note: \$finish|   Time: |End time: |Errors: [0-9]+, Warnings: )' |
    sed 's/^# //' | grep -v -E '^ *quit -f$' > "$n.golden"
  rm -rf "$work"
done
