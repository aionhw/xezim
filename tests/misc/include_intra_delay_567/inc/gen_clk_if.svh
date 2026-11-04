// Pulled in with `include; the delays below are intra-assignment delays
// (IEEE 1800-2017 9.4.5).
interface gen_clk_if;
  localparam HALF = 5;
  bit c;
  logic q = 0;
  always c = #(HALF) ~c;          // free-running clock
  always @(posedge c) q <= #3 ~q; // nonblocking update 3 ticks after each posedge
endinterface
