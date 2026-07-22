// SPDX-License-Identifier: MIT
//
// probe_questa_resolution.sv — a one-shot diagnostic that prints the actual
// value Questa assigns to every built-in net type and nettype-after-resolver
// configuration we care about. Run this once, copy the output into the
// tier tests so the assertions match Questa's actual behavior, and we'll
// have a defensible baseline.

module probe;
  // ---- single driver ----
  wire  w_one_0;  assign w_one_0  = 1'b0;
  wire  w_one_1;  assign w_one_1  = 1'b1;
  tri   t_one_0;  assign t_one_0  = 1'b0;
  tri   t_one_1;  assign t_one_1  = 1'b1;

  // ---- 2 drivers, no z ----
  wire  w00;      assign w00 = 1'b0; assign w00 = 1'b0;
  wire  w01;      assign w01 = 1'b0; assign w01 = 1'b1;
  wire  w11;      assign w11 = 1'b1; assign w11 = 1'b1;
  tri   t00;      assign t00 = 1'b0; assign t00 = 1'b0;
  tri   t01;      assign t01 = 1'b0; assign t01 = 1'b1;
  tri   t11;      assign t11 = 1'b1; assign t11 = 1'b1;

  // ---- 2 drivers, with z ----
  tri   t_0z;     assign t_0z = 1'b0; assign t_0z = 1'bz;
  tri   t_1z;     assign t_1z = 1'b1; assign t_1z = 1'bz;

  // ---- 3 drivers, including z ----
  tri   t_01z;    assign t_01z = 1'b0; assign t_01z = 1'b1; assign t_01z = 1'bz;

  // ---- all-z ----
  tri   t_zz;     assign t_zz = 1'bz; assign t_zz = 1'bz;

  // ---- wand / triand ----
  wand  wand_00;  assign wand_00 = 1'b0; assign wand_00 = 1'b0;
  wand  wand_01;  assign wand_01 = 1'b0; assign wand_01 = 1'b1;
  wand  wand_11;  assign wand_11 = 1'b1; assign wand_11 = 1'b1;
  triand triand_01; assign triand_01 = 1'b0; assign triand_01 = 1'b1;
  wand  wand_0z;  assign wand_0z = 1'b0; assign wand_0z = 1'bz;
  wand  wand_1z;  assign wand_1z = 1'b1; assign wand_1z = 1'bz;

  // ---- wor / trior ----
  wor   wor_00;   assign wor_00 = 1'b0; assign wor_00 = 1'b0;
  wor   wor_01;   assign wor_01 = 1'b0; assign wor_01 = 1'b1;
  wor   wor_11;   assign wor_11 = 1'b1; assign wor_11 = 1'b1;
  trior trior_01; assign trior_01 = 1'b0; assign trior_01 = 1'b1;

  // ---- tri0 / tri1 ----
  tri0  tri0_no;       // no drivers -> default value
  tri0  tri0_0;        assign tri0_0 = 1'b0;
  tri0  tri0_1;        assign tri0_1 = 1'b1;
  tri0  tri0_01;       assign tri0_01 = 1'b0; assign tri0_01 = 1'b1;
  tri1  tri1_no;       // no drivers -> default value
  tri1  tri1_0;        assign tri1_0 = 1'b0;
  tri1  tri1_1;        assign tri1_1 = 1'b1;
  tri1  tri1_01;       assign tri1_01 = 1'b0; assign tri1_01 = 1'b1;

  // ---- supply ----
  supply0 s0_no;  // no driver
  supply1 s1_no;  // no driver

  initial begin
    $display("== PROBE BEGIN: Questa resolution values ==");
    #0;
    $display("single-driver:                       w_one_0=%b w_one_1=%b t_one_0=%b t_one_1=%b",
             w_one_0, w_one_1, t_one_0, t_one_1);
    $display("2-driver same value:                 w00=%b w11=%b t00=%b t11=%b",
             w00, w11, t00, t11);
    $display("2-driver different (no z):           w01=%b t01=%b",
             w01, t01);
    $display("2-driver with z:                     t_0z=%b t_1z=%b",
             t_0z, t_1z);
    $display("3-driver [0,1,z]:                    t_01z=%b",  t_01z);
    $display("all-z drivers:                       t_zz=%b",   t_zz);
    $display("wand all-zero:                       wand_00=%b", wand_00);
    $display("wand mixed:                          wand_01=%b wand_11=%b", wand_01, wand_11);
    $display("triand mixed:                        triand_01=%b", triand_01);
    $display("wand with z:                         wand_0z=%b wand_1z=%b", wand_0z, wand_1z);
    $display("wor mixed:                           wor_01=%b wor_11=%b",  wor_01, wor_11);
    $display("trior mixed:                         trior_01=%b", trior_01);
    $display("tri0 default (no drivers):           tri0_no=%b", tri0_no);
    $display("tri0 [0]:                            tri0_0=%b",  tri0_0);
    $display("tri0 [1]:                            tri0_1=%b",  tri0_1);
    $display("tri0 [0,1]:                          tri0_01=%b", tri0_01);
    $display("tri1 default (no drivers):           tri1_no=%b", tri1_no);
    $display("tri1 [0]:                            tri1_0=%b",  tri1_0);
    $display("tri1 [1]:                            tri1_1=%b",  tri1_1);
    $display("tri1 [0,1]:                          tri1_01=%b", tri1_01);
    $display("supply0 no drivers:                  s0_no=%b",   s0_no);
    $display("supply1 no drivers:                  s1_no=%b",   s1_no);
    $display("== PROBE END ==");
    $finish;
  end
endmodule
