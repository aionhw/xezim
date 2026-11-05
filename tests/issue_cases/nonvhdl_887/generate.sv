// The hiding class is declared in a generate block (27.5).
class base; int x = 5; endclass
module top;
  if (1) begin : g
    class base; int x = 7; endclass
    class derived extends base;
      function int get(); return x; endfunction
    endclass
    function automatic int val(); derived d = new(); return d.get(); endfunction
  end
  task automatic chk(string what, int got, int exp);
    if (got === exp) $display("PASS %s", what);
    else $display("FAIL %s: got %0d, expected %0d", what, got, exp);
  endtask
  initial begin
    chk("class in a generate-if block", g.val(), 7);
  end
endmodule
