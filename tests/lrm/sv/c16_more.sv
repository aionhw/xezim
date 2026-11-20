// top: c16_more
module c16_more;
  // The trace records nonvacuous matches and failures.
  initial $assertvacuousoff(0);
  bit clk, a, b, c; int k;
  always #5 clk = ~clk;
  bit [3:0] va [0:9] = '{4'b0001,4'b0010,4'b0100,4'b0001,4'b0011,4'b0110,4'b0000,4'b0001,4'b0000,4'b0000};
  initial begin for (int i = 0; i < 10; i++) begin @(negedge clk); {c,b,a} = va[i][2:0]; end $finish; end
  function void note(string s); $display("T|16.11|match %s t=%0t", s, $time); endfunction
  c_sub: cover property (@(posedge clk) (a, note("a")) ##1 (b, note("b")));
  property p_rec(x); @(posedge clk) x and (1'b1 |=> p_rec(x)); endproperty
  // recursive property: always-like; checks !(a&&c) forever
  a_rec: assert property (p_rec(!(a && c))) else $display("T|16.12.17|rec F t=%0t", $time);
  clocking cb @(posedge clk);
    property p_cb; a |=> b; endproperty
  endclocking
  a_cb: assert property (cb.p_cb) $display("T|16.18|cb P t=%0t", $time); else $display("T|16.18|cb F t=%0t", $time);
  property p_typed(int n, logic s); @(posedge clk) s |-> ##n c; endproperty
  a_typed: assert property (p_typed(2, a)) $display("T|16.12.18|typed P t=%0t", $time); else $display("T|16.12.18|typed F t=%0t", $time);
  always @(posedge clk) $display("T|16.9.3|sampled t=%0t a=%0b s=%0b", $time, a, $sampled(a));
  a_dflt: assert property (@(posedge clk) a |=> c);
  always @(a or b) begin : blk a_dbg: assert #0 (!(a && b)) else $display("T|16.4.4|deferred F t=%0t", $time); end
endmodule
