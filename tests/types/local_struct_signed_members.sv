// IEEE 1800-2023 §6.11.3, §6.12.1, §7.2: a signed member of an unpacked
// structure declared in a procedural block reads signed, also after a real is
// rounded into it; it read as an unsigned number.
module top;
  typedef struct { int i; byte b; shortint h; longint l; } SI;
  function automatic int f();
    SI fs;
    fs.i = -1.5;
    return fs.i;
  endfunction
  initial begin
    SI si;
    si.i = -1.5; si.b = -1.5; si.h = -1.5; si.l = -1.5;
    $display("T|real %0d %0d %0d %0d", si.i, si.b, si.h, si.l);
    si.i = -2; si.b = -2;
    $display("T|int %0d %0d", si.i, si.b);
    si.i = 8'd5; si.i = si.i - 10;
    $display("T|arith %0d %0d", si.i, si.i < 0);
    $display("T|function %0d", f());
  end
  initial begin : named
    SI sn;
    sn.i = -1.5;
    $display("T|named %0d", sn.i);
  end
endmodule
