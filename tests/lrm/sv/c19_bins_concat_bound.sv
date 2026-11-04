// §11.4.12, §19.5: a bin range bounded by a constant concatenation or
// replication, in a module and in a parameterized class.
class enc_cov #(int Width = 4);
  logic [Width-1:0] value;
  covergroup cg;
    coverpoint value { bins all[Width] = {[0:{Width{1'b1}}]}; }
  endgroup
  function new(); cg = new; endfunction
  function void sample(logic [Width-1:0] v); value = v; cg.sample(); endfunction
endclass
module c19_bins_concat_bound;
  localparam int W = 4;
  logic [W-1:0] v;
  covergroup cg;
    coverpoint v {
      bins lo[2] = {[0:{1'b0, {(W-1){1'b1}}}]};
      bins hi = {[{1'b1, {(W-1){1'b0}}}:{W{1'b1}}]};
    }
  endgroup
  cg c;
  enc_cov e4;
  enc_cov #(8) e8;
  initial begin
    c = new;
    v = 1; c.sample();
    v = 9; c.sample();
    $display("T|module|%0.2f", c.get_inst_coverage());
    v = 5; c.sample();
    $display("T|module_all|%0.2f", c.get_inst_coverage());
    e4 = new;
    e4.sample(1); e4.sample(5); e4.sample(9); e4.sample(13);
    $display("T|class4|%0.2f", e4.cg.get_inst_coverage());
    e8 = new;
    e8.sample(8'h00); e8.sample(8'hff);
    $display("T|class8|%0.2f", e8.cg.get_inst_coverage());
  end
endmodule
