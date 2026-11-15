// IEEE 1800-2023 §27.5: only the selected generate block is instantiated.
`timescale 1ns/1ps
module top; parameter ENABLE=0; typedef struct {int a,b;} S;
if(ENABLE) begin: unused S s; initial s='{3{7}}; end
initial begin #1; $display("PASS inactive parameter"); $finish; end endmodule
