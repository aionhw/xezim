#!/usr/bin/env bash
# Regenerate the golden output of every design here with GHDL (the VHDL
# reference simulator): <name>.golden is GHDL's stdout, <name>.exit its exit
# status. Run from any directory; needs `ghdl` (tested with GHDL 4.1).
set -u
cd "$(dirname "$0")"
work=$(mktemp -d)
for f in *.vhd; do
  n=${f%.vhd}
  ghdl -a --std=08 --workdir="$work" "$f" && ghdl -e --std=08 --workdir="$work" "$n" || { echo "GHDL failed on $f"; continue; }
  ghdl -r --std=08 --workdir="$work" "$n" > "$n.golden" 2>/dev/null
  echo $? > "$n.exit"
done
rm -rf "$work"
