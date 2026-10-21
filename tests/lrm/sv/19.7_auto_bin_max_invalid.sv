// top: c19_7_auto_bin_max_invalid
module c19_7_auto_bin_max_invalid;
  bit clk;
  bit [3:0] a;
  int i;

  // Test negative auto_bin_max (should be treated as 0 per LRM? Or error?)
  // LRM says "A value of 0 means no automatic bins are created."
  // Negative values are not specified. We'll test that negative is treated as 0.
  covergroup cg_neg @(posedge clk);
    option.auto_bin_max = -1;
    cp_a : coverpoint a;
  endgroup

  // Test auto_bin_max as a non-constant (should use default or error)
  // Using a variable - this might not be allowed in covergroup body
  // covergroup cg_var @(posedge clk);
  //   option.auto_bin_max = i; // i is not constant
  //   cp_b : coverpoint a;
  // endgroup

  cg_neg gn = new;

  always #5 clk = ~clk;

  initial begin
    for (i = 0; i < 16; i++) begin
      @(negedge clk) a = i;
      gn.sample();
    end
    @(negedge clk);

    // Negative auto_bin_max should result in no automatic bins (0 coverage)
    $display("T|19.7|neg cp_a=%0.2f tot=%0.2f", gn.cp_a.get_coverage(), gn.get_coverage());

    $finish;
  end
endmodule