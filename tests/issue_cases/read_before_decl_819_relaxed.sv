// §6.5: strict mode rejects forward module-data reads in calls, events and
// assertions. Relaxed mode preserves this reference simulator trace.
module t;
  logic y_f, y_or;
  int n_ev;
  int q[$];
  string s;
  function automatic logic f(input logic a); return a; endfunction
  task automatic put(input int v); q.push_back(v); endtask

  initial begin
    #1;
    y_f = f(w);
    put(k);
    s = $sformatf("%b", w);
    $display("T|call %b %0d %s %0d", y_f, q[0], s, $size(arr));
  end
  always @(w or k) n_ev++;
  initial #2 assert (w) $display("T|assert pass");
  assign y_or = t.w;
  initial begin : blk
    int w;
    w = 5;
    for (int k = 0; k < 3; k++) w += k;
    $display("T|local %0d", w);
  end
  function automatic int g(int w);
    return w + 1;
  endfunction
  initial #3 $display("T|formal %0d", g(41));

  logic w = 1'b1;
  int k = 7;
  int arr[4];

  initial begin
    #4 k = 8;
    #1 $display("T|hier %b events %0d", y_or, n_ev);
    $finish;
  end
endmodule
