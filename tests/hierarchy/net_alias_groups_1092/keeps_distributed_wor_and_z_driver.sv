// IEEE 1800-2023 §10.11: "same physical net"; §6.6.3 wor is 1 when any driver is 1; §6.6.1 z yields to 1.
// Distributed drivers must preserve the already-correct driven name; omit main-wrong a/c.
`timescale 1ns/1ps
module top;
  wire low=0, high=1, floating=1'bz;
  wor a,b; alias a=b; assign a=low; assign b=high;
  wire c,d; alias c=d; assign d=high; assign c=floating;
  initial begin #1; $display("T|protected %b %b",b,d); $finish; end
endmodule
