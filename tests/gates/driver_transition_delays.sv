`timescale 1ns/1ps
// IEEE 1800-2023 §28.16, §10.3.2/§10.3.3: each driver delays its own
// transitions before §28.12 resolution. Net delays then delay that result.
module top;
  logic a, assign_enable, gate_zero;
  wire mixed, reversed, pair, gate_only, assign_only, equal;
  wand wired_and; // §6.6.3, §28.12.4
  wor wired_or;   // §6.6.3, §28.12.4
  wire #2 net_delay; // §6.7.1, §28.16
  wire turn_off;
  buf #(1,5) gm(mixed,a);
  assign #(1,1) mixed = a;
  assign #(1,1) reversed = a;
  buf #(1,5) gr(reversed,a);
  buf #(1,5) gp1(pair,a);
  buf #(3,2) gp2(pair,a);
  buf #(1,5) gg(gate_only,a);
  assign #(1,2,7) assign_only = a;
  buf #(1,1) ge(equal,a);
  assign #(1,5) equal = a;
  buf #(1,5) ga(wired_and,a);
  assign #(1,1) wired_and = a;
  buf #(1,5) go(wired_or,a);
  assign #(1,1) wired_or = a;
  buf #(1,5) gn(net_delay,a);
  assign #(1,1) net_delay = a;
  // §10.3.3/§28.16: the assignment's turn-off is 7, independently
  // of the gate's fall delay of 5. The gate keeps driving 0 throughout.
  buf #(6,5) gt(turn_off,gate_zero);
  assign #(1,2,7) turn_off = assign_enable ? 1'b1 : 1'bz;
  initial begin
    // §4.7/§6.8: initialize after time zero, sample away from update times.
    #1 a=1; assign_enable=0; gate_zero=0;
    #10 a=0;
    // §9.4.1: sample between the integer driver-update times.
    #1.5 $display("early pair=%b",pair);
    #1.5 $display("fall mixed=%b reversed=%b pair=%b gate=%b assign=%b equal=%b and=%b or=%b",
                mixed,reversed,pair,gate_only,assign_only,equal,wired_and,wired_or);
    #1 $display("net fall=%b",net_delay);
    #4 $display("fallen mixed=%b pair=%b gate=%b net=%b",mixed,pair,gate_only,net_delay);
    #2 a=1;
    #2 $display("rise mixed=%b pair=%b gate=%b assign=%b net=%b",mixed,pair,gate_only,assign_only,net_delay);
    #3 $display("risen mixed=%b pair=%b net=%b",mixed,pair,net_delay);
    #5 assign_enable=1;
    #3 $display("assign on=%b",turn_off);
    #7 assign_enable=0;
    #3 $display("assign off pending=%b",turn_off);
    #6 $display("off settled=%b",turn_off);
    $finish;
  end
endmodule
