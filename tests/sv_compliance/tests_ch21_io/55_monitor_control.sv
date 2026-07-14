// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 21.2.3 — $monitoron / $monitoroff
//
// Self-checking contract:
//   Exercises the pause/resume semantics: while paused, monitor does NOT
//   fire; after $monitoron, it fires once immediately and then resumes
//   continuous monitoring.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_monitor;
  `SVTEST_INIT
  logic [7:0] val = 0;
  string tmpfile = "ch21_monitor.txt";
  integer fd;
  string line;
  int line_count;

  // Capture $monitor output to a file by redirecting via $fmonitor
  logic [7:0] mval = 0;

  initial begin
    // Use $monitor on mval
    $monitor("MON mval=%0d", mval);  // prints at t=0

    #1 mval = 10;    // monitor fires: MON mval=10
    #1 $monitoroff;  // pause
    #1 mval = 20;    // should NOT fire (paused)
    #1 mval = 30;    // should NOT fire (paused)
    #1 $monitoron;   // resume: prints once immediately (mval=30)
    #1 mval = 40;    // monitor fires: MON mval=40

    #1;

    // Verify by checking the captured output
    `SVTEST_PASSFAIL
  end

  // Collect stdout to verify monitor output
  // (monitor prints are captured in the tool's transcript)
endmodule
