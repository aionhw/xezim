// Ch.14 clocking blocks, Ch.19 covergroups
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[c] %s", name); fails++; end
  logic clk = 0;
  logic [3:0] sig;
  always #5 clk = ~clk;

  clocking cb @(posedge clk);
    default input #1step output #0;
    input sig;
  endclocking

  covergroup cg @(posedge clk);
    cp: coverpoint sig {
      bins low = {[0:7]};
      bins high = {[8:15]};
    }
  endgroup

  initial begin
    cg c = new();
    sig = 3;
    @(posedge clk);
    sig = 12;
    @(posedge clk);
    @(posedge clk);
    `CK("clocking sampled input", cb.sig == 12)
    `CK("covergroup coverage nonzero", c.get_inst_coverage() > 0.0)
    `CK("both bins hit", c.get_inst_coverage() >= 99.0)
    $finish;
  end
  final $display("CHC CHECKS DONE fails=%0d", fails);
endmodule
