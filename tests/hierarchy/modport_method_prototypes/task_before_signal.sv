// IEEE 1800-2023 §25.3 Syntax 25-1, §25.7: an imported task prototype can precede a signal item.
interface bus_if;
  int value;
  task put(input int n); value=n; endtask
  modport user(import task put(input int n),input value);
endinterface
module client(bus_if.user b);
  initial b.put(37);
endmodule
module top;
  bus_if b(); client c(b);
  initial begin
    #1;
    $display("value=%0d",b.value);
    if(b.value!=37) $fatal(1,"FAIL");
    $display("PASS");$finish;
  end
  initial #100 $finish;
endmodule
