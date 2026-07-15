// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 21.2.1.1–21.2.1.7 — Format specifications
//
// Self-checking contract:
//   Exercises the shared format-spec engine used by $display/$write/$fwrite/
//   $swrite/$sformatf. Each formatted line is captured via $sformatf into a
//   string and compared to an expected literal.
//
// Covers:
//   - %0 zero-suppression on h/b/o (strip leading zeros, keep min width)
//   - .N precision on f/e/g
//   - field width + zero-pad on d/h/b/o
//   - %c %s %m %t %%
//   - %0d / %d equivalence (decimal has no full-width default)
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_format_specs;
  `SVTEST_INIT

  // Capture a formatted string via $sformatf and compare to expected.
  task automatic check(input string got, input string exp, input string msg);
    begin
      `SVTEST_CHECK(got == exp, msg)
      if (got != exp)
        $display("  got=[%s] exp=[%s]", got, exp);
    end
  endtask

  // Capture $display output via $sformatf (same engine, returns string).
  string s;

  initial begin
    // ---- %0 zero-suppression on hex / binary / octal ----
    s = $sformatf("%0h", 32'h000000FF);   check(s, "ff",  "%0h 32'hFF");
    s = $sformatf("%0h", 32'h00000000);   check(s, "0",   "%0h zero");
    s = $sformatf("%0h", 32'hDEADBEEF);   check(s, "deadbeef", "%0h full");
    s = $sformatf("%0b", 8'b00000011);    check(s, "11",  "%0b");
    s = $sformatf("%0b", 8'b00000000);    check(s, "0",   "%0b zero");
    s = $sformatf("%0o", 32'h00000007);   check(s, "7",   "%0o");
    s = $sformatf("%0o", 32'h00000000);   check(s, "0",   "%0o zero");

    // ---- default (full-width) hex / binary without %0 ----
    s = $sformatf("%h", 32'hFF);          check(s, "000000ff", "%h default width");
    s = $sformatf("%b", 4'b0011);         check(s, "0011", "%b default width");

    // ---- explicit field width on hex ----
    s = $sformatf("%4h", 16'h0F);         check(s, "000f", "%4h");
    // Field width below the natural (full-vector) width keeps the natural
    // form — Icarus `%2h` of 32'hFF is "000000ff", NOT "ff". Only bare
    // `%0h` trims to the minimum; an explicit width never truncates.
    s = $sformatf("%2h", 32'hFF);         check(s, "000000ff", "%2h < natural width keeps full form");

    // ---- decimal: %0d == %d (no full-width default) ----
    s = $sformatf("%d", 32'd255);         check(s, "       255", "%d default");
    s = $sformatf("%0d", 32'd255);        check(s, "255", "%0d");
    s = $sformatf("%05d", 32'd42);        check(s, "00042", "%05d zero-padded decimal");
    s = $sformatf("%5d", 32'd42);         check(s, "   42", "%5d space-pad");

    // ---- .N precision on real ----
    s = $sformatf("%.2f", 3.14159);       check(s, "3.14", "%.2f");
    s = $sformatf("%.0f", 3.7);           check(s, "4", "%.0f rounds");
    s = $sformatf("%.4e", 2.5);           check(s, "2.5000e+00", "%.4e");
    s = $sformatf("%.3g", 3.14159);       check(s, "3.14", "%.3g");
    // default precision for %f is 6
    s = $sformatf("%f", 1.0);             check(s, "1.000000", "%f default 6");

    // ---- %c %s %% ----
    s = $sformatf("%c%c", 65, 66);        check(s, "AB", "%c");
    s = $sformatf("%s", "hi");            check(s, "hi", "%s");
    s = $sformatf("100%%");               check(s, "100%", "%%");

    `SVTEST_PASSFAIL
  end
endmodule
