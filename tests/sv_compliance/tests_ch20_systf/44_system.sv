// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 20.17.1 — $system
//   $system ( [ "terminal_command_line" ] )   // task OR function (returns int)
//
// Self-checking contract:
//   $system : writes known bytes to a file via the host shell, then reads them
//             back with $fopen/$fgetc and verifies them.
//             The function form's return value is checked to be 0 on success.
//             POSIX (sh + printf) is assumed on the host running the simulator.
//   $stacktrace (20.17.2) is tested separately in 46_stacktrace.sv — it is a
//   newer LRM feature that older simulator releases do not implement yet.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_system;
  `SVTEST_INIT

  localparam string OUT_FILE = "xezim_systf_out.txt";
  localparam int    EXPECTED_BYTE = "Z";   // ASCII 'Z' = 8'h5A = 90

  int rc;          // $system function return
  int fd;          // file handle
  int got_byte;    // $fgetc result

  function automatic void cleanup();
    // Best-effort removal of the scratch file (ignore result).
    rc = $system({"rm -f ", OUT_FILE});
  endfunction

  initial begin
    cleanup();                              // ensure a clean start

    // ---- $system as a FUNCTION: host writes byte 'Z' to OUT_FILE ----
    rc = $system({"printf 'Z' > ", OUT_FILE});
    `SVTEST_CHECK(rc == 0, "$system(\"printf ...\") should return 0 on success")

    // ---- $system as a TASK (return discarded) also works ----
    $system({"printf 'Z' >> ", OUT_FILE});  // append a second 'Z'

    // ---- $system with NO argument is legal (calls C system(NULL)) ----
    // Per 20.17.1: "If $system is called with no string argument, the C
    // function system() will be called with the NULL string." Just exercise it.
    $system();

    // ---- read it back and verify the first two bytes ----
    fd = $fopen(OUT_FILE, "r");
    `SVTEST_CHECK(fd != 0, "$fopen of $system-produced file failed")
    if (fd != 0) begin
      got_byte = $fgetc(fd);
      `SVTEST_CHECK(got_byte == EXPECTED_BYTE,
                   "$system-written byte should be 'Z' (90)")
      // a second byte exists from the task-form append
      got_byte = $fgetc(fd);
      `SVTEST_CHECK(got_byte == EXPECTED_BYTE,
                   "second $system-written byte should also be 'Z'")
      $fclose(fd);
    end

    cleanup();                              // remove scratch file

    `SVTEST_PASSFAIL
  end
endmodule
