// §6.5/§8.3/§18.7: class enum and randomized-object names are not uses of
// the later module variable. The getter tests elaboration without a call.
module t;
  class EnumReader;
    enum {later = 3} value;
    function int get(); return later; endfunction
  endclass
  class RandItem;
    rand int later;
  endclass
  RandItem obj;
  initial begin
    int ok;
    obj = new;
    ok = obj.randomize() with { later == 5; };
    if (!ok || obj.later != 5) $display("FAIL object constraint scope");
    else $display("PASS object constraint scope");
    $finish;
  end
  int later = 99;
endmodule
