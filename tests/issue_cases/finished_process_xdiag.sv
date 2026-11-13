// IEEE 1800-2023 §10.4.2 / §9.6.3: delayed writes survive child termination.
// Diagnostics that attribute a write to a fork child that has already ended
// (--x-warn, XEZIM_VALUE_TRACE, XEZIM_TRACE_SCHED), in a sub-instance.
module sub;
  logic [3:0] q = 4'd1, r = 4'd2, s = 4'd3;
  logic u;
  initial begin : main
    #2;
    fork
      begin : late q <= #5 {3'b0, u}; end           // child ends at t=2, X lands at t=7
      begin : now r = 4'bx01x; #1; end              // blocking X write
      begin : killed #1; s <= #9 4'bxxxx; #50; end  // killed at t=5, X lands at t=12
    join_none
    #3 disable fork;
    repeat (5) fork begin #1; end begin #2; end join
    #20;
    fork begin : again q = 4'd4; #1 q = 4'bxxxx; end join
    #1 $display("sub done q=%b r=%b s=%b t=%0t", q, r, s, $time);
  end
endmodule
module top;
  sub s0();
  initial begin #100; $finish; end
endmodule
