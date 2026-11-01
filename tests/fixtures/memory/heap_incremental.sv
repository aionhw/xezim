class Node;
  int id;
  Node next;
  function new(int i); id = i; endfunction
endclass

module top;
  Node keep, scratch;
  int sum;
  initial begin
    keep = new(1);
    for (int t = 0; t < 8; t++) begin
      // Churn: enough allocations per slot to trigger a collection.
      for (int i = 0; i < 21000; i++) scratch = new(1000 + i);
      if (t == 1) keep.next = new(7);          // survives a collection via an edge
      if (t == 3) keep.next.next = new(8);     // allocated after it, linked to a survivor
      if (t == 5) keep.next = keep.next.next;  // 7 becomes unreachable
      #1;
    end
    sum = keep.id + keep.next.id;
    $display("GC_INCR sum=%0d next_null=%0d", sum, keep.next.next == null);
    $finish;
  end
endmodule
