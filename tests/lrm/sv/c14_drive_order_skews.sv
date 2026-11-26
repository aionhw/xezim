// top: c14do
`timescale 1ns/1ps
module c14do;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] za = 0, zb = 0, zc = 0, zd = 0, ye = 0, yf = 0, yg = 0, yh = 0;
  clocking cz @(posedge clk);
    default output #0;
    output za, zb, zc, zd;
  endclocking
  clocking cy @(posedge clk);
    default output #2;
    output ye, yf, yg, yh;
  endclocking
  initial begin
    @(cz);
    cz.za[0] <= 1'b1;    cz.za <= 4'b1010;
    cz.zb <= 4'b1010;    cz.zb[0] <= 1'b1;
    cy.yg <= 4'b1010;    cy.yg[0] <= 1'b1;
    cy.yh[3:2] <= 2'b11; cy.yh <= 4'b0001; cy.yh[1] <= 1'b1;
    cy.ye <= ##1 4'b1010; cy.ye[0] <= ##1 1'b1;
    #1;
    cz.zc[0] <= 1'b1;    cz.zc <= 4'b1010;
    cz.zd <= 4'b1010;    cz.zd[0] <= 1'b1;
    cy.yf[0] <= 1'b1;    cy.yf <= 4'b1010;
    @(cz); @(cz); @(cz);
    #1 $finish;
  end
  always @(za) $display("T|za|t=%0t za=%b", $time, za);
  always @(zb) $display("T|zb|t=%0t zb=%b", $time, zb);
  always @(zc) $display("T|zc|t=%0t zc=%b", $time, zc);
  always @(zd) $display("T|zd|t=%0t zd=%b", $time, zd);
  always @(ye) $display("T|ye|t=%0t ye=%b", $time, ye);
  always @(yf) $display("T|yf|t=%0t yf=%b", $time, yf);
  always @(yg) $display("T|yg|t=%0t yg=%b", $time, yg);
  always @(yh) $display("T|yh|t=%0t yh=%b", $time, yh);
endmodule
