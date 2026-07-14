// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 20.10 — $fatal termination semantics
//
// $fatal shall generate a run-time FATAL error that terminates simulation
// (an implicit $finish, 20.10). The optional first argument is the
// finish_number (0|1|2), consistent with $finish (20.2). Optional remaining
// arguments are a $display-style message.
//
// SPECIAL PASS CRITERIA (this test does NOT print TEST_PASS — by design):
//   1. "BEFORE_FATAL"          MUST appear in the output.
//   2. "SHOULD_NOT_REACH_HERE" MUST NOT appear (proves $fatal terminated).
//   3. "TEST_PASS_IF_FATAL_DID_NOT_TERMINATE" MUST NOT appear.
//   4. The simulator exit code MUST be NON-ZERO (fatal = error exit).
//
// A reference (and a correct xezim) simulator MUST print BEFORE_FATAL and then
// exit non-zero without reaching the lines after $fatal.

`timescale 1ns/1ps

module test_severity_fatal;
  initial begin
    $display("BEFORE_FATAL");
    $fatal(1, "deliberate $fatal at sim-time %0t to verify termination", $time);
    // The following must NEVER execute:
    $display("SHOULD_NOT_REACH_HERE");
    $display("TEST_PASS_IF_FATAL_DID_NOT_TERMINATE");
  end
endmodule
