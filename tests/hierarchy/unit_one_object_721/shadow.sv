int g = 5;
int h = 50;
int arr [2] = '{1, 2};
function automatic void addg(int by); g += by; endfunction
function automatic int geth(int h); return h + g; endfunction
task automatic setarr(int v); arr[1] = v; endtask
class K;
  static function void sset(int v); g = v; endfunction
  function int get(); return g + arr[1]; endfunction
endclass
module sh;  // module-level shadow
  int g = 100;
  int arr [2] = '{7, 8};
  K k = new;
  initial begin
    #1; $display("T|sh local=%0d unit=%0d arr=%0d uarr=%0d", g, $unit::g, arr[1], $unit::arr[1]);
    addg(1); $display("T|sh after addg local=%0d unit=%0d", g, $unit::g);
    setarr(30); $display("T|sh after setarr arr=%0d uarr=%0d", arr[1], $unit::arr[1]);
    $display("T|sh k.get=%0d geth=%0d", k.get(), geth(1));
    K::sset(60); $display("T|sh after sset local=%0d unit=%0d", g, $unit::g);
    $unit::g = $unit::g + 1; g = g + 1;
    $display("T|sh end local=%0d unit=%0d", g, $unit::g);
  end
endmodule
module blk;  // block-local and argument shadows
  function automatic int f(int g); return g * 2; endfunction
  initial begin
    #2;
    begin : b
      int g;
      g = 7; $display("T|blk block=%0d unit=%0d f=%0d", g, $unit::g, f(4));
    end
    $display("T|blk outer=%0d", g);
  end
endmodule
module child; initial begin #3; $display("T|child g=%0d arr=%0d", g, arr[1]); g = g + 1000; end endmodule
module par; int g = -1; child c(); genvar i;
  for (i = 0; i < 2; i++) begin : gb int h = i; initial begin #4; $display("T|par.gb%0d h=%0d uh=%0d g=%0d ug=%0d", i, h, $unit::h, g, $unit::g); end end
endmodule
module top;
  sh s(); blk b(); par p();
  initial begin #5; $display("T|top g=%0d h=%0d arr=%0d p.g=%0d s.g=%0d", g, h, arr[1], p.g, s.g); $finish; end
endmodule
