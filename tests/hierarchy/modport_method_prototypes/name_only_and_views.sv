// IEEE 1800-2023 §25.3 Syntax 25-1, §25.7: name-only imports and modport lists.
// §25.5.4: expression member; §14.3 / §25.5.5: clocking member.
// §4.7: one writer and delayed samples avoid initial-block ordering races.
interface bus_if(input bit clk);
  int value, extra, other;
  task put(input int n); value = n; endtask
  task tick(); extra = 9; endtask
  clocking cb @(posedge clk); input value; endclocking
  modport user(output extra, other, import put, tick, input value),
    observer(input value),
    expr_view(input .low(value[7:0])),
    clock_view(clocking cb);
endinterface
module client(bus_if.user b);
  initial begin b.put(37); b.tick(); b.other = 11; end
endmodule
module observer(bus_if.observer b);
  initial #1 $display("T|observe=%0d", b.value);
endmodule
module expr_client(bus_if.expr_view b);
  initial #2 $display("T|expr=%0d", b.low);
endmodule
module top;
  bit clk = 0;
  bus_if b(clk); client c(b); observer o(b); expr_client e(b);
  initial begin
    #3 $display("T|state=%0d %0d %0d", b.value, b.extra, b.other);
    $finish;
  end
endmodule
