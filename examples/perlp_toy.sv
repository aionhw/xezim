// PDES toy: two clocked counters + 1 shared signal. Used as the human-
// readable reference for the synthetic 2-LP test in
// `xezim/src/multikernel/tests.rs::two_counters_with_shared_signal_via_pdes`.
//
// The Rust test stubs the blocks as closures rather than parsing this
// file, because the perlp-experiment branch is wire-up scaffolding only
// — it does not yet integrate with the SV parser / elaborator.
//
// Topology:
//   LP-A owns count_a (8-bit reg). Increments on every posedge clk_a.
//   LP-B owns count_b (8-bit reg). Each posedge clk_b: count_b += count_a.
//   count_a is the single boundary signal (produced by LP-A, consumed by LP-B).
//   Both clocks have identical period (5 ns half-period → 10 ns full cycle).
//
// Expected after 10 cycles (sim_time = 100 ns):
//   count_a = 10
//   count_b = sum(0..=9) = 45   (LP-B reads count_a's value from the
//                                 previous cycle, by CMB lookahead-1)

module counter_a(
    input  wire clk_a,
    output reg [7:0] count_a
);
    initial count_a = 0;
    always @(posedge clk_a) count_a <= count_a + 1;
endmodule

module counter_b(
    input  wire clk_b,
    input  wire [7:0] shared,
    output reg [7:0] count_b
);
    initial count_b = 0;
    always @(posedge clk_b) count_b <= count_b + shared;
endmodule

module tb;
    reg clk_a = 1'b0;
    reg clk_b = 1'b0;
    wire [7:0] count_a;
    wire [7:0] count_b;

    counter_a a (.clk_a(clk_a), .count_a(count_a));
    counter_b b (.clk_b(clk_b), .shared(count_a), .count_b(count_b));

    always #5 clk_a = ~clk_a;
    always #5 clk_b = ~clk_b;

    initial begin
        #100;
        $display("count_a = %0d", count_a);
        $display("count_b = %0d", count_b);
        $finish;
    end
endmodule
