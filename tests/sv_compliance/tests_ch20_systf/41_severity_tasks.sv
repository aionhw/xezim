// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 20.10 — Severity system tasks
//   $info, $warning, $error   (shall NOT terminate simulation)
//   $fatal                     (terminates — tested separately in 42_severity_fatal.sv)
//
// Self-checking contract (run on a reference simulator):
//   - Each of $info / $warning / $error emits a message and execution CONTINUES.
//   - A sentinel set AFTER each call proves continuation.
//   - $display-style format arguments are accepted (string + values).
//   - Calls with NO arguments are accepted.
//   - The module prints exactly "TEST_PASS" on success and exits normally.
//
// What is intentionally NOT asserted: the exact tool-specific message text
// (file/line/hier/sim-time preamble is implementation-defined per 20.10).
//
// Tool-default caveat: per LRM 20.10 ONLY $fatal terminates; $error/$warning/$info
// must allow execution to continue. Commercial simulators conform by default.
// NOTE: Verilator non-conformantly aborts on $error by default, so this test
// should be validated against a conformant simulator (not Verilator).

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_severity_tasks;
  `SVTEST_INIT

  bit after_info    = 1'b0;
  bit after_warning = 1'b0;
  bit after_error   = 1'b0;
  bit after_noarg   = 1'b0;
  int counter       = 0;

  initial begin
    // ---- $info : message of no specific severity, shall continue ----
    $info("info message number %0d", 1);
    after_info = 1'b1;
    counter = counter + 1;

    // ---- $warning : shall continue ----
    $warning("warning at counter=%0d", counter);
    after_warning = 1'b1;
    counter = counter + 1;

    // ---- $error : shall continue (does NOT terminate) ----
    $error("error at counter=%0d", counter);
    after_error = 1'b1;
    counter = counter + 1;

    // ---- no-argument forms are legal ----
    $info;
    $warning();
    $error();
    after_noarg = 1'b1;
    counter = counter + 1;

    // ---- continuation assertions ----
    `SVTEST_CHECK(after_info    == 1'b1, "$info did not continue execution")
    `SVTEST_CHECK(after_warning == 1'b1, "$warning did not continue execution")
    `SVTEST_CHECK(after_error   == 1'b1, "$error did not continue execution")
    `SVTEST_CHECK(after_noarg   == 1'b1, "no-arg severity calls did not continue")
    `SVTEST_CHECK(counter       == 4,    "counter not incremented past $error")

    `SVTEST_PASSFAIL
  end
endmodule
