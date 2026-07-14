// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 21.3.2 — File output radix variants
//   $fdisplayb/h/o, $fwriteb/h/o
//
// Self-checking contract:
//   Writes values to a temp file using each radix variant (UNFORMATTED args —
//   the task name's radix only applies to arguments without a %specifier),
//   reads the file back, and compares to the expected formatted string.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_file_radix_output;
  `SVTEST_INIT
  integer fd;
  string line;
  int i, rc;
  string lines[4];
  string tmpfile = "ch21_radix_out.txt";

  // Helper: strip trailing newline (and possible \r) from a string
  function automatic string rstrip(input string s);
    int len = s.len();
    while (len > 0 && (s.substr(len-1, len-1) == "\n" || s.substr(len-1, len-1) == "\r"))
      len--;
    return s.substr(0, len-1);
  endfunction

  initial begin
    fd = $fopen(tmpfile, "w");
    `SVTEST_CHECK(fd != 0, "$fopen succeeded")

    // UNFORMATTED args: the task name determines the radix.
    // 8-bit values get full-width output in the task's radix.
    $fdisplayb(fd, 8'd5);     // binary  → 00000101
    $fdisplayh(fd, 8'hAB);    // hex     → ab
    $fdisplayo(fd, 8'd64);    // octal   → 100

    // fwrite variants (no trailing newline) — add manually
    $fwriteh(fd, 8'd170);     // hex     → aa
    $fdisplay(fd, "");        // newline

    $fclose(fd);

    // Read back and verify
    fd = $fopen(tmpfile, "r");
    i = 0;
    while (i < 4) begin
      line = "";
      rc = $fgets(line, fd);
      if (rc == 0) break;
      if (line.len() > 0) begin
        lines[i] = rstrip(line);
        i++;
      end
    end
    $fclose(fd);
    $system("rm -f ch21_radix_out.txt");

    `SVTEST_CHECK(lines[0] == "00000101", "fdisplayb")
    `SVTEST_CHECK(lines[1] == "ab",       "fdisplayh")
    `SVTEST_CHECK(lines[2] == "100",      "fdisplayo")
    `SVTEST_CHECK(lines[3] == "aa",       "fwriteh")

    `SVTEST_PASSFAIL
  end
endmodule
