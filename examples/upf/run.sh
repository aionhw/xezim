#!/bin/sh
# Simulate the power-aware example with its UPF power intent.
cd "$(dirname "$0")/../.." || exit 1
exec cargo run --release -- --simulate -s pwr_demo_tb --no-cache \
    --upf examples/upf/pwr_demo.upf \
    examples/upf/pwr_demo.sv examples/upf/pwr_demo_tb.sv "$@"
