// §7.4.6 / §7.5.1: out-of-range `d[i] = new[n()]` discards the row sizing but
// still evaluates the size expression.
module rdob;
  int d[][];
  int n_calls;
  function int n();
    n_calls++;
    return 2;
  endfunction
  initial begin
    d = new[1];
    d[1] = new[n()];
    $display("T|7.5.1row_oob|%0d %0d", n_calls, d.size());
  end
endmodule