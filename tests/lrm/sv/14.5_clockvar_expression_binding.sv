// top: c145e
// §14.5, §14.13: a clocking input bound to an expression samples the value
// the expression takes from its operands' samples, at every input skew, at
// the expression's own width; a variable outside the block named like the
// clockvar is not sampled instead.
`timescale 1ns/1ns
module c145e;
  bit clk = 0;
  always #5 clk = ~clk;
  int a = 3;
  logic [3:0] op = 4'h5, ra = 4'ha;
  logic [7:0] rb = 8'b0000_1110;
  logic signed [7:0] sg = -8'sd4;
  logic [79:0] w = 80'hffff_ffff_ffff_ffff_ffff;
  logic [3:0] xv = 4'b10x1;
  logic c = 1;
  int sum = 99;
  clocking cb @(posedge clk);
    input sum = a + 1;
    input ins = {op, ra, rb[3:1]};
    input cat = {a[3:0], 1'b0};
    input #0 s0 = a + 10;
    input #2 s2 = a + 20;
    input ng = 0 - sg;
    input sgx = sg >>> 1;
    input wide = w + 1;
    input xx = xv + 1;
    input cnd = c ? op : ra;
    input un = op ^ 4'hf;
    input red = ra == 4'hf;
  endclocking
  initial begin
    @(cb); #1;
    $display("T|a|sum=%0d ins=%h cat=%b s0=%0d s2=%0d", cb.sum, cb.ins, cb.cat, cb.s0, cb.s2);
    $display("T|b|ng=%0d sgx=%0d wide=%h xx=%b cnd=%h un=%h red=%b",
             cb.ng, cb.sgx, cb.wide, cb.xx, cb.cnd, cb.un, cb.red);
    $display("T|c|bits sum=%0d ins=%0d cat=%0d", $bits(cb.sum), $bits(cb.ins), $bits(cb.cat));
    // `a` changes 2 before the next edge (seen by `#2`) and at the edge
    // itself (seen by `#0` only).
    #7 a = 7;
    #2 a = 8;
    @(cb); #1;
    $display("T|d|t=%0t sum=%0d s0=%0d s2=%0d", $time, cb.sum, cb.s0, cb.s2);
    $finish;
  end
  always @(cb.sum) $display("T|ev|t=%0t sum=%0d", $time, cb.sum);
endmodule
