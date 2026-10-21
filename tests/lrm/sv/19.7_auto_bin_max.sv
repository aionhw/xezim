// top: c19_7_auto_bin_max
module c19_7_auto_bin_max;
  bit clk;
  bit [3:0] a;
  bit [1:0] b;
  bit [5:0] c;
  int i;

  // Test 1: covergroup with auto_bin_max = 4, two coverpoints
  // cp_a (4-bit, 16 values) gets 4 bins, cp_b (2-bit, 4 values) gets 0 bins
  covergroup cg1 @(posedge clk);
    option.auto_bin_max = 4;
    cp_a : coverpoint a;
    cp_b : coverpoint b;
  endgroup

  // Test 2: covergroup with auto_bin_max = 8, three coverpoints
  // cp_x (4-bit, 16 values) gets 8 bins, cp_y (2-bit, 4 values) gets 0 bins, cp_z (6-bit, 64 values) gets 0 bins
  covergroup cg2 @(posedge clk);
    option.auto_bin_max = 8;
    cp_x : coverpoint a;
    cp_y : coverpoint b;
    cp_z : coverpoint c;
  endgroup

  // Test 3: covergroup with auto_bin_max = 0, no automatic bins
  covergroup cg3 @(posedge clk);
    option.auto_bin_max = 0;
    cp_m : coverpoint a;
    cp_n : coverpoint b;
  endgroup

  // Test 4: coverpoint with own auto_bin_max overrides covergroup
  covergroup cg4 @(posedge clk);
    option.auto_bin_max = 2;
    cp_p : coverpoint a { option.auto_bin_max = 8; } // own limit 8
    cp_q : coverpoint b; // inherits covergroup limit 2
  endgroup

  cg1 g1 = new;
  cg2 g2 = new;
  cg3 g3 = new;
  cg4 g4 = new;

  always #5 clk = ~clk;

  initial begin
    for (i = 0; i < 64; i++) begin
      @(negedge clk)
      a = i[3:0];
      b = i[1:0];
      c = i[5:0];
      g1.sample();
      g2.sample();
      g3.sample();
      g4.sample();
    end
    @(negedge clk);

    // Test 1: cg1 auto_bin_max=4, cp_a gets 4 bins (all hit), cp_b gets 0 bins
    $display("T|19.7|cg1 cp_a=%0.2f cp_b=%0.2f tot=%0.2f",
             g1.cp_a.get_coverage(), g1.cp_b.get_coverage(), g1.get_coverage());

    // Test 2: cg2 auto_bin_max=8, cp_x gets 8 bins (all hit), cp_y gets 0, cp_z gets 0
    $display("T|19.7|cg2 cp_x=%0.2f cp_y=%0.2f cp_z=%0.2f tot=%0.2f",
             g2.cp_x.get_coverage(), g2.cp_y.get_coverage(), g2.cp_z.get_coverage(), g2.get_coverage());

    // Test 3: cg3 auto_bin_max=0, no automatic bins for cp_m or cp_n
    $display("T|19.7|cg3 cp_m=%0.2f cp_n=%0.2f tot=%0.2f",
             g3.cp_m.get_coverage(), g3.cp_n.get_coverage(), g3.get_coverage());

    // Test 4: cg4 auto_bin_max=2, cp_p has own auto_bin_max=8 (gets 8 bins), cp_q gets 2 bins
    $display("T|19.7|cg4 cp_p=%0.2f cp_q=%0.2f tot=%0.2f",
             g4.cp_p.get_coverage(), g4.cp_q.get_coverage(), g4.get_coverage());

    $finish;
  end
endmodule