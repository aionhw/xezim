// IEEE 1800-2023 §27.5: only the selected generate block is instantiated.
`timescale 1ns/1ps
module top; typedef struct {int a,b;} S;
if(0) begin: unused S s; initial s='{3{7}}; end
initial begin #1; $display("PASS inactive"); $finish; end endmodule
