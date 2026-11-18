// Issue #1241: a clocking block input bound to an EXPRESSION (`input sum = a + 1;`)
// samples X instead of the expression's value. IEEE 1800-2023 §14.5 permits an
// input-clockvar default (expression), and §14.13 says the value is the operand
// signature sampled at the clocking event, matched against the reference
// simulator: a=3 sum=4 cat=...110 slc=...10 sx=5 mux=10 ci=42 all PASS.
module top;
  bit clk = 0;
  int  a = 3;
  int  b = 6;
  logic [7:0] bus = 8'b1010_1010;
  always #5 clk = ~clk;

  clocking cb @(posedge clk);
    input sum = a + 1;             // arithmetic
    input cat = {a, 1'b0};         // concatenation
    input slc = bus[3:0];          // part-select
    input sx  = $signed(a) + 2;    // signed arithmetic
    input mux = a > 1 ? 10 : 20;   // conditional (int condition)
    input ci  = 42;                // constant
  endclocking

  initial begin
    @(cb); #1;
    $display("a=%0d sum=%0d cat=%b slc=%b sx=%0d mux=%0d ci=%0d",
             a, cb.sum, cb.cat, cb.slc, cb.sx, cb.mux, cb.ci);
    if (cb.sum !== 4) $display("FAIL sum got %0d want 4", cb.sum);
    if (cb.cat !== {a, 1'b0}) $display("FAIL cat");
    if (cb.slc !== 4'b1010) $display("FAIL slc");
    if (cb.sx !== 5) $display("FAIL sx");
    if (cb.mux !== 10) $display("FAIL mux want 10 got %0d", cb.mux);
    if (cb.ci !== 42) $display("FAIL ci");
    if (cb.sum === 4 && cb.cat === {a, 1'b0} && cb.slc === 4'b1010
        && cb.sx === 5 && cb.mux === 10 && cb.ci === 42)
        $display("PASS");
    $finish;
  end
endmodule