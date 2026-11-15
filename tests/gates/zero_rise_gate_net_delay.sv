// IEEE 1800-2023 §28.4–§28.9, §28.16, §10.3.3.
// Fall at t=21+5+3=29; rise at t=41+0+2=43.
// Enabled gate outputs all equal a. Stimulus and samples do not race (§4.7).
`timescale 1ns/1ps
module top;
  logic a,en;
  wire #(2,3) w_buf; buf #(0,5) g_buf(w_buf,a);
  wire #(2,3) w_not; not #(0,5) g_not(w_not,~a);
  wire #(2,3) w_and; and #(0,5) g_and(w_and,a,1'b1);
  wire #(2,3) w_nand; nand #(0,5) g_nand(w_nand,~a,1'b1);
  wire #(2,3) w_or; or #(0,5) g_or(w_or,a,1'b0);
  wire #(2,3) w_nor; nor #(0,5) g_nor(w_nor,~a,1'b0);
  wire #(2,3) w_xor; xor #(0,5) g_xor(w_xor,a,1'b0);
  wire #(2,3) w_xnor; xnor #(0,5) g_xnor(w_xnor,~a,1'b0);
  wire #(2,3) w_bufif0; bufif0 #(0,5) g_bufif0(w_bufif0,a,~en);
  wire #(2,3) w_bufif1; bufif1 #(0,5) g_bufif1(w_bufif1,a,en);
  wire #(2,3) w_notif0; notif0 #(0,5) g_notif0(w_notif0,~a,~en);
  wire #(2,3) w_notif1; notif1 #(0,5) g_notif1(w_notif1,~a,en);
  wire #(2,3) w_nmos; nmos #(0,5) g_nmos(w_nmos,a,en);
  wire #(2,3) w_pmos; pmos #(0,5) g_pmos(w_pmos,a,~en);
  wire #(2,3) w_rnmos; rnmos #(0,5) g_rnmos(w_rnmos,a,en);
  wire #(2,3) w_rpmos; rpmos #(0,5) g_rpmos(w_rpmos,a,~en);
  wire #(2,3) w_cmos; cmos #(0,5) g_cmos(w_cmos,a,en,~en);
  wire #(2,3) w_rcmos; rcmos #(0,5) g_rcmos(w_rcmos,a,en,~en);

  initial begin
    #1 a=1; en=1;
    #20 a=0;
    #20 a=1;
    #20 en=0;
  end
  initial begin
    #24.5 $display("T|t=24.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    #2 $display("T|t=26.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor});
    #2 $display("T|t=28.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor});
    #1 $display("T|t=29.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    #12 $display("T|t=41.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    #1 $display("T|t=42.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    #1 $display("T|t=43.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    #19 $display("T|t=62.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    #1 $display("T|t=63.5 gates=%b", {w_buf, w_not, w_and, w_nand, w_or, w_nor, w_xor, w_xnor, w_bufif0, w_bufif1, w_notif0, w_notif1, w_nmos, w_pmos, w_rnmos, w_rpmos, w_cmos, w_rcmos});
    $finish;
  end
endmodule
