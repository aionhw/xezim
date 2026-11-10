// The module's own class pc is not parameterized and hides the unit's parameterized one, so `pc::v` names
// the module's class: it is legal (8.25.1 restricts `::` on a name that is a parameterized class).
class pc #(int N = 1); static int v = N; endclass
module top;
  class pc; static int v = 9; endclass
  task automatic chk(string what, int got, int exp);
    if (got === exp) $display("PASS %s", what);
    else $display("FAIL %s: got %0d, expected %0d", what, got, exp);
  endtask
  initial begin
    chk("pc::v", pc::v, 9);
  end
endmodule
