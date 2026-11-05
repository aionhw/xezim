// §27.4, §23.9: subroutines declared in a generate-for block, called from
// the block, a nested block, a continuous assign and from outside the loop.
// The T| lines were checked with the reference simulator.
module top;
  localparam int W = 8;
  int y [2];
  for (genvar i = 0; i < 2; i++) begin : g
    logic [7:0] x = 8'(10 * i);
    function automatic int f(); return W + i; endfunction
    function automatic int old_style(); old_style = 100 + i; endfunction
    function automatic int fact(int n); return n <= 1 ? 1 : n * fact(n - 1); endfunction
    function automatic int rd(); return x; endfunction
    task automatic t(); $display("T|task %0d", i); endtask
    assign y[i] = f();
    initial begin
      #1;
      $display("T|blk %0d f=%0d old=%0d fact=%0d rd=%0d", i, f(), old_style(), fact(i + 3), rd());
      t();
    end
    if (1) begin : h
      initial #2 $display("T|nested %0d f=%0d", i, f());
    end
  end
  if (1) begin : c
    function automatic int cf(); return 7; endfunction
    initial #3 $display("T|cond cf=%0d", cf());
  end
  initial begin
    #4;
    $display("T|assign %0d %0d", y[0], y[1]);
    $display("T|outside %0d %0d %0d", g[0].f(), g[1].f(), c.cf());
    $finish;
  end
endmodule
