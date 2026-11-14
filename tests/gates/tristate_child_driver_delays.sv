`timescale 1ns/1ps
// §23.3.3: the inout connects both primitive outputs to the same net.
// §28.5/§28.7/§28.16: falls at t26 and t30, hence x at t29 (§28.12).
// §4.7/§6.8: initialize at t1 and observe away from driver updates.
module child(inout wire w, input wire a, en);
  nmos #(2,9) g(w,a,en);
endmodule
module top;
  logic a,en;
  wire w;
  buf #(1,5) g(w,a);
  child u(w,a,en);
  initial begin
    #1 a=1; en=1;
    #20 a=0;
    #8 if (w === 1'bx) $display("PASS child fall pending");
       else $display("FAIL child fall pending got=%b",w);
    #7 if (w === 1'b0) $display("PASS child fall settled");
       else $display("FAIL child fall settled got=%b",w);
    $finish;
  end
endmodule
