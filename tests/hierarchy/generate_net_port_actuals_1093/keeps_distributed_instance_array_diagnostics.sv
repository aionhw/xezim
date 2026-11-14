// IEEE 1800-2023 §23.3.3.5: "each instance shall get a part-select of the port connection"; four one-bit input ports receive bits 3,2,1,0 of 0000.
// Distributed packed actuals on instance arrays need per-instance width checks.
`timescale 1ns/1ps
module reader(input wire p);endmodule
module top;
 for(genvar k=0;k<2;k++)begin:g wire [3:0] w=4'b0000;end
 reader u[3:0](g[1].w);
 initial begin #1;$display("T|array %b%b%b%b",u[3].p,u[2].p,u[1].p,u[0].p);$finish;end
endmodule
