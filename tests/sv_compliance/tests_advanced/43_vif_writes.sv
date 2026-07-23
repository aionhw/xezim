// Compliance test: virtual-interface write forms propagate to the bound instance.
//
// Five write forms are tested through a virtual interface handle:
//   (1) blocking scalar     vif.a = x
//   (2) NBA scalar          vif.b <= x
//   (3) blocking bit-select vif.v[i] = x
//   (4) NBA bit-select      vif.w[i] <= x
//   (5) NBA whole vector    vif.u <= '1
//
// Before the fix, forms (3)-(5) were silently dropped because the write
// target was resolved at NBA-schedule time against a stale this_stack, or
// because the bit-select resolver did not propagate back through the vif
// binding.
interface probe_if(input logic clk);
  logic       a, b;
  logic [4:0] v, w, u;
endinterface

module top;
  logic clk = 0;
  always #5 clk = ~clk;

  probe_if pif(.clk(clk));

  class writer;
    virtual probe_if vif;
    task run();
      @(posedge vif.clk);
      vif.a    = 1'b1;
      vif.b   <= 1'b1;
      vif.v[1] = 1'b1;
      vif.w[1] <= 1'b1;
      vif.u   <= '1;
    endtask
  endclass

  initial begin
    automatic writer wr = new;
    wr.vif = pif;
    wr.run();
  end

  initial begin
    #20;
    if (pif.a     !== 1'b1)        $display("TEST_FAIL: blk scalar a=%b (expect 1)", pif.a);
    else if (pif.b !== 1'b1)       $display("TEST_FAIL: nba scalar b=%b (expect 1)", pif.b);
    else if (pif.v[1] !== 1'b1)    $display("TEST_FAIL: blk bit-select v[1]=%b (expect 1)", pif.v[1]);
    else if (pif.w[1] !== 1'b1)    $display("TEST_FAIL: nba bit-select w[1]=%b (expect 1)", pif.w[1]);
    else if (pif.u !== 5'b11111)   $display("TEST_FAIL: nba vector u=%b (expect 11111)", pif.u);
    else                           $display("TEST_PASS");
    $finish;
  end
endmodule
