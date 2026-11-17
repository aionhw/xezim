// IEEE 1800-2023 §25.3 Syntax 25-1, §25.7: prototypes and names share a list.
// §13.5: packed formals and defaults; §4.7: sample after the caller completes.
interface bus_if;
  typedef logic [7:0] byte_t;
  int value, count;
  task put(input int n, input logic [7:0] mask = (8'(1 << 4) | 8'd3));
    value = n + int'(mask);
  endtask
  task tick(); count = 9; endtask
  function byte_t read_value(input int bias, input int extra);
    return byte_t'(value + bias + extra);
  endfunction
  function void clear(); value = 0; endfunction
  modport user(import tick, task put(input int n,
      input logic [(4+4)-1:0] mask),
      function byte_t read_value(input int bias, input int extra),
      function void clear(), input value, count),
    observer(input value, count),
    final_user(input value, import function byte_t read_value(input int bias, input int extra));
endinterface
module client(bus_if.user b);
  initial begin
    b.clear();
    b.put(37, 8'd19);
    b.tick();
    $display("T|call=%0d", b.read_value(2, 3));
  end
endmodule
module observer(bus_if.observer b);
  initial #1 $display("T|observe=%0d count=%0d", b.value, b.count);
endmodule
module last_client(bus_if.final_user b);
  initial #2 $display("T|last=%0d", b.read_value(1, 2));
endmodule
module top;
  bus_if b(); client c(b); observer o(b); last_client l(b);
  initial #3 $finish;
endmodule
