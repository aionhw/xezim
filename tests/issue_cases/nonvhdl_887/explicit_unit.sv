// An explicit `$unit::` qualifier names the compilation-unit class, also where the module declares one of
// the same name (IEEE 1800-2023 3.12.1: `$unit` is the name of the compilation-unit scope).
class base;
  static int sv = 50;
  static function int sf(); return 500; endfunction
endclass
module top;
  class base;
    static int sv = 70;
    static function int sf(); return 700; endfunction
  endclass
  task automatic chk(string what, int got, int exp);
    if (got === exp) $display("PASS %s", what);
    else $display("FAIL %s: got %0d, expected %0d", what, got, exp);
  endtask
  initial begin
    chk("base::sv", base::sv, 70);
    chk("base::sf()", base::sf(), 700);
    chk("$unit::base::sv", $unit::base::sv, 50);
    chk("$unit::base::sf()", $unit::base::sf(), 500);
  end
endmodule
