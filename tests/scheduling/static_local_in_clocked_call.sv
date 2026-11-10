module top;
  logic clk = 0; always #5 clk = ~clk;
  int n = 0, t = 0, a = 0, f = 0, g = 0, w = 0, blk = 0;
  function void bump(); int k; k = k + 1; n = k; endfunction
  task tk(); int j; j += 2; t = j; endtask
  function automatic void ab(); static int s = 5; int c; c++; s++; a = s * 10 + c; endfunction
  function int cnt(); int c; c = c + 3; return c; endfunction
  function automatic int acnt(input int x); static int s; s += x; return s; endfunction
  function void wr(input int x); int tmp; tmp = x * 2; w = w + tmp; endfunction
  function void nest(); if (clk) begin int q; q = q + 1; blk = q; end endfunction
  always @(posedge clk) begin bump(); tk(); ab(); f = cnt(); g <= acnt(2); wr(3); nest(); end
  initial begin
    repeat (4) @(posedge clk);
    #1 $display("T|n=%0d t=%0d a=%0d f=%0d g=%0d w=%0d blk=%0d", n, t, a, f, g, w, blk);
    $finish;
  end
endmodule
