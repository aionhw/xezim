// §8.25: bind class expressions before calling another specialization and
// distinguish the same caller class instantiated with different arguments.
class store #(int N=0);
  static int value=0;
  static function void put(int v); value=v; endfunction
  static function int get(); return value; endfunction
endclass

class agent #(int WIDTH=8);
  function void put_first(int value); store#(WIDTH+1)::put(value); endfunction
  function int get_first(); return store#(WIDTH+1)::get(); endfunction
  function void put_second(int value); store#(WIDTH+2)::put(value); endfunction
  function int get_second(); return store#(WIDTH+2)::get(); endfunction
endclass

module top;
  localparam int WIDTH=4;
  initial begin
    agent#(8) first;
    agent#(12) second;
    first=new;
    second=new;
    store#(WIDTH)::put(17);
    $display("T|module|%0d",store#(4)::get());
    first.put_first(33);
    second.put_first(77);
    store#(WIDTH+1)::put(22);
    store#(WIDTH+2)::put(44);
    second.put_second(88);
    first.put_second(55);
    $display("T|cache|%0d|%0d|%0d|%0d|%0d|%0d|%0d|%0d|%0d|%0d",first.get_first(),second.get_first(),store#(5)::get(),store#(9)::get(),store#(13)::get(),first.get_second(),second.get_second(),store#(6)::get(),store#(10)::get(),store#(14)::get());
    $finish;
  end
endmodule
