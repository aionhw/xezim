// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 21.3.4–21.3.8 — File reading and status
//   $fgets, $feof, $ferror, $fflush
//
// Self-checking contract:
//   Creates a test file via $fdisplay, then reads it back with $fgets and
//   verifies content. Checks $feof returns nonzero at end-of-file, and
//   $fflush completes without error.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_file_read;
  `SVTEST_INIT
  integer fd;
  string line;
  int count;
  string tmpfile = "ch21_read_test.txt";

  initial begin
    // ---- Write a test file ----
    fd = $fopen(tmpfile, "w");
    `SVTEST_CHECK(fd != 0, "$fopen write")
    $fdisplay(fd, "Hello World");
    $fdisplay(fd, "123");
    $fclose(fd);

    // ---- Read it back line by line ----
    fd = $fopen(tmpfile, "r");
    `SVTEST_CHECK(fd != 0, "$fopen read")

    // Line 1: "Hello World\n"
    count = $fgets(line, fd);
    `SVTEST_CHECK(count > 0, "fgets line 1 returned >0")
    // $fgets includes the newline; strip it for comparison
    `SVTEST_CHECK(line.substr(0, line.len()-2) == "Hello World", "fgets content 1")

    // Line 2: "123\n"
    count = $fgets(line, fd);
    `SVTEST_CHECK(count > 0, "fgets line 2 returned >0")
    `SVTEST_CHECK(line.substr(0, line.len()-2) == "123", "fgets content 2")

    // Third call should hit EOF
    count = $fgets(line, fd);
    `SVTEST_CHECK(count == 0, "fgets at EOF returns 0")

    // $feof should now return nonzero
    `SVTEST_CHECK($feof(fd) != 0, "$feof returns nonzero at EOF")

    $fclose(fd);

    // ---- $ferror on a valid handle returns 0 ----
    fd = $fopen(tmpfile, "r");
    `SVTEST_CHECK($ferror(fd, line) == 0, "$ferror returns 0 on valid handle")
    $fclose(fd);

    // ---- $fflush ----
    fd = $fopen(tmpfile, "w");
    $fdisplay(fd, "test");
    $fflush(fd);
    $fclose(fd);

    $system("rm -f ch21_read_test.txt");
    `SVTEST_PASSFAIL
  end
endmodule
