// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 21.7.1 — VCD dump control tasks
//   $dumpall, $dumplimit, $dumpflush
//
// Self-checking contract:
//   Exercises the three control tasks for the 4-state VCD writer. Since VCD
//   content format is tool-specific, this test only asserts the tasks execute
//   without error.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_vcd_misc;
  `SVTEST_INIT
  logic clk = 0;
  logic [7:0] count = 0;

  initial begin
    $dumpfile("ch21_vcd_misc.vcd");
    $dumpvars;

    repeat (3) begin
      #5 clk = ~clk;
      count = count + 1;
    end

    // §21.7.1.4 $dumpall: checkpoint all current values
    $dumpall;

    // §21.7.1.5 $dumplimit: set a size cap (best-effort)
    $dumplimit(1000000);

    repeat (2) begin
      #5 clk = ~clk;
      count = count + 1;
    end

    // §21.7.1.6 $dumpflush: flush to disk
    $dumpflush;

    #1;
    `SVTEST_PASSFAIL
  end
endmodule
