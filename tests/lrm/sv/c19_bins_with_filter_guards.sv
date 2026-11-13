// Regression: bin-filter evaluation must short-circuit `&&`/`||` like sampling,
// and wide filtered ranges must not vanish when shape building hits its scan cap.
module c19_bins_with_filter_guards;
  logic [31:0] v;
  logic [3:0] narrow;
  covergroup div0_cg;
    coverpoint narrow {
      bins b[] = {[0:7]} with (item != 0 && 8 / item > 1);
    }
  endgroup
  covergroup wide_cg;
    coverpoint v {
      bins even[4] = {[0:1048576]} with (item % 2 == 0);
    }
  endgroup
  div0_cg dg;
  wide_cg wg;
  initial begin
    dg = new;
    narrow = 1; dg.sample();
    narrow = 2; dg.sample();
    narrow = 3; dg.sample();
    narrow = 4; dg.sample();
    narrow = 5; dg.sample();
    narrow = 6; dg.sample();
    narrow = 7; dg.sample();
    $display("T|div0|%0.2f", dg.get_inst_coverage());
    wg = new;
    v = 0; wg.sample();
    v = 2; wg.sample();
    $display("T|wide|%0.2f", wg.get_inst_coverage());
  end
endmodule
