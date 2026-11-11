// IEEE 1800-2023 §6.6.5-6, §6.7.1: undriven and supply defaults.
// §6.8, §10.3.2-3: a real variable has one continuous driver, no initializer.
// Sample at t1/t3, away from time-zero settlement and the delayed t2 update.
`timescale 1ns/1ps
module top;
  wire undriven;
  tri0 low_pull;
  tri1 high_pull;
  supply0 ground_net;
  supply1 power_net;
  real real_out;
  wire immediate;
  assign #2 ground_net = 1'b0;
  assign #2 power_net = 1'b1;
  assign #2 real_out = 1.25;
  assign immediate = 1'b1;
  initial begin
    #1 $display("T|t1 undriven=%b tri0=%b tri1=%b supply=%b%b real=%0.2f immediate=%b",
                undriven, low_pull, high_pull, ground_net, power_net, real_out, immediate);
    #2 $display("T|t3 undriven=%b tri0=%b tri1=%b supply=%b%b real=%0.2f immediate=%b",
                undriven, low_pull, high_pull, ground_net, power_net, real_out, immediate);
    $finish;
  end
endmodule
