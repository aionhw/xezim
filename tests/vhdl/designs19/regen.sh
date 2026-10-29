#!/usr/bin/env bash
# Regenerate the golden output of every VHDL-2019 design here with nvc (the
# VHDL-2019 reference simulator; GHDL 4.1 as packaged has no 2019
# libraries): <name>.golden is nvc's stdout (the designs write their results
# with std.textio; nvc prints report messages on stderr), <name>.exit its
# exit status. Needs `nvc` on PATH or in $NVC (tested with nvc 1.23.1).
# A design nvc cannot run has a golden written from the LRM and a
# <name>.lrm file saying why; it is skipped here.
set -u
cd "$(dirname "$0")"
nvc=${NVC:-nvc}
work=$(mktemp -d)
for f in *.vhd; do
  n=${f%.vhd}
  [ -f "$n.lrm" ] && { echo "skipping $f (LRM golden)"; continue; }
  # Analyse, elaborate and run in the work directory: a design may create files.
  cp "$f" "$work/"
  (cd "$work" && "$nvc" --std=2019 -a "$f" -e "$n" -r) > "$n.golden" 2>/dev/null
  echo $? > "$n.exit"
done
rm -rf "$work"
