// SPDX-License-Identifier: MIT
//
// IEEE 1800-2023 Clause 21.4–21.5 — Memory file load and write
//   $writememb, $writememh (memory dump to file)
//   Round-trip: write → read → compare
//   $readmemd (decimal load, new in 2023) — DIAGNOSTIC only (older reference
//   tools do not implement it)
//
// Self-checking contract:
//   Seeds a memory, dumps it in hex/binary, clears, reloads, and verifies
//   the round-tripped values match.
//
// Prints "TEST_PASS" on success.

`timescale 1ns/1ps
`include "../common/svtest_defs.svh"

module test_memory_file;
  `SVTEST_INIT
  logic [7:0] mem [0:3];

  initial begin
    mem[0] = 8'hDE;
    mem[1] = 8'hAD;
    mem[2] = 8'hBE;
    mem[3] = 8'hEF;

    // ---- $writememh → $readmemh round-trip ----
    $writememh("ch21_memdump.hex", mem);
    mem[0]=0; mem[1]=0; mem[2]=0; mem[3]=0;
    $readmemh("ch21_memdump.hex", mem);
    `SVTEST_CHECK(mem[0] == 8'hDE, "writememh->readmemh [0]")
    `SVTEST_CHECK(mem[1] == 8'hAD, "writememh->readmemh [1]")
    `SVTEST_CHECK(mem[2] == 8'hBE, "writememh->readmemh [2]")
    `SVTEST_CHECK(mem[3] == 8'hEF, "writememh->readmemh [3]")

    // ---- $writememb → $readmemb round-trip ----
    $writememb("ch21_memdump.bin", mem);
    mem[0]=0; mem[1]=0; mem[2]=0; mem[3]=0;
    $readmemb("ch21_memdump.bin", mem);
    `SVTEST_CHECK(mem[0] == 8'hDE, "writememb->readmemb [0]")
    `SVTEST_CHECK(mem[3] == 8'hEF, "writememb->readmemb [3]")

    // ---- $readmemd: DIAGNOSTIC (2023 feature, not in older tools) ----
    // Create a decimal memory file, then try to load it. Report status
    // rather than hard-asserting since older simulator releases do not
    // implement it.
    begin
      integer fd;
      fd = $fopen("ch21_memdump.dec", "w");
      $fdisplay(fd, "@0 222");
      $fdisplay(fd, "@1 173");
      $fdisplay(fd, "@2 190");
      $fdisplay(fd, "@3 239");
      $fclose(fd);
    end
    mem[0]=0; mem[1]=0; mem[2]=0; mem[3]=0;
    $readmemd("ch21_memdump.dec", mem);
    if (mem[0] == 8'd222 && mem[3] == 8'd239)
      $display("INFO: $readmemd works (values loaded correctly)");
    else
      $display("INFO: $readmemd not supported by this tool (2023 feature)");

    $system("rm -f ch21_memdump.hex ch21_memdump.bin ch21_memdump.dec");
    `SVTEST_PASSFAIL
  end
endmodule
