// IEEE 1800-2023 §28.5: buf maps z data to x before delaying it.
// §11.4.6: x === z is false. Samples avoid updates (§4.7).
`timescale 1ns/1ps
module top;
  logic a,en;
  wire #(2,3) w,conditional,m;
  buf #(0,5) g(w,a);
  // A buf input and a MOS can lower to the same conditional ending in z;
  // keep that ambiguous form's main-correct comparisons unchanged.
  buf #(0,5) gc(conditional,en ? a : 'z);
  nmos #(0,5) gm(m,a,1'b1);
  initial begin
    #1 a=1; en=1; #20 a=1'bx; #20 a=1'bz; #20 a=0;
    #5.5 $display("T|truth=%b is_z=%b conditional_is_z=%b mos_is_x=%b",w,w===1'bz,conditional===1'bz,m===1'bx);
    #3 $display("T|settled=%b",w);
    $finish;
  end
endmodule
