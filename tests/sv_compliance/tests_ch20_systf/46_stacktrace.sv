// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 20.17.2 — $stacktrace
//   $stacktrace                       // task form: displays the call stack
//   string s = $stacktrace;           // function form: returns a string
//
// DEDICATED, SEPARATE FILE: $stacktrace was added to the LRM later than the
// other Clause-20 system tasks, so older simulator releases may not implement
// it yet (they reject the design at load time). Run this test with a NEWER
// simulator version that supports the $stacktrace feature to validate the
// reference behaviour, then use the same test against the xezim implementation.
//
// Per LRM 20.17.2 the CONTENT of the call stack information is
// IMPLEMENTATION-DEPENDENT, so this test only asserts:
//   - function form returns a value assignable to a `string` variable;
//   - execution CONTINUES past both the function and task forms;
//   - the returned string is non-empty (reported as INFO; this is a
//     reasonableness check, not an LRM requirement — left as INFO in case a
//     tool legitimately returns an empty string).
// It also exercises $stacktrace from inside a nested call chain so there is a
// real stack to trace, and prints a sample of the returned string for
// cross-tool capture.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_stacktrace;
  `SVTEST_INIT

  bit reached_fn_top;   // continuation sentinel: function form at top level
  bit reached_fn_nested;// continuation sentinel: function form from a sub-call
  bit reached_tk;       // continuation sentinel: task form
  string tr_top;        // function-form return, called from the top level
  string tr_nested;     // function-form return, called from a nested task

  // A nested subroutine so the function-form call site has a real call stack.
  task automatic capture_nested();
    tr_nested = $stacktrace;             // function form, nested context
    reached_fn_nested = 1'b1;
  endtask

  initial begin
    // ---- function form, top-level call ----
    reached_fn_top = 1'b0;
    tr_top = $stacktrace;                // assignable to `string` => string-typed
    reached_fn_top = 1'b1;
    `SVTEST_CHECK(reached_fn_top == 1'b1,
                 "$stacktrace function form (top) did not continue")

    // ---- function form, nested call (exercises a real call stack) ----
    reached_fn_nested = 1'b0;
    capture_nested();
    `SVTEST_CHECK(reached_fn_nested == 1'b1,
                 "$stacktrace function form (nested) did not continue")

    // ---- task form: displays the stack, execution continues ----
    reached_tk = 1'b0;
    $stacktrace;
    reached_tk = 1'b1;
    `SVTEST_CHECK(reached_tk == 1'b1,
                 "$stacktrace task form did not continue")

    // ---- reasonableness (INFO only, not LRM-required): non-empty strings ----
    $display("INFO: top-level  $stacktrace len=%0d", tr_top.len());
    $display("INFO: nested      $stacktrace len=%0d", tr_nested.len());
    if (tr_top.len()    > 0) $display("INFO: top stacktrace sample: %s ...", tr_top.substr(0, 39));
    if (tr_nested.len() > 0) $display("INFO: nst stacktrace sample: %s ...", tr_nested.substr(0, 39));

    `SVTEST_PASSFAIL
  end
endmodule
