// Use: a constraint of a derived class names a member it inherits from the module's base (18.5, 8.15).
// The unit class of the same name has no member `r`.
class base; int x = 5; endclass
module top;
  class base; rand int r; endclass
  class derived extends base;
    constraint c { r > 10; r < 20; }
  endclass
  task automatic chk(string what, int got, int exp);
    if (got === exp) $display("PASS %s", what);
    else $display("FAIL %s: got %0d, expected %0d", what, got, exp);
  endtask
  initial begin
    derived d = new();
    chk("randomize() succeeds", d.randomize(), 1);
    chk("the constraint on the inherited r holds", (d.r > 10 && d.r < 20), 1);
  end
endmodule
