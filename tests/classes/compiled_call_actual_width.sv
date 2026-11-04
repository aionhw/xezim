// IEEE 1800-2023 §13.5.1 / §11.6.1: on the bytecode-compiled tier every
// call lowering gives an actual the context width of the callee's declared
// formal: `super.wide9(a + b)` and a free function `free9(a + b)` evaluate
// `a + b` at 9 bits (300), as the bare in-class call already did. Each probe
// is run 1500x to clear the compile threshold; under XEZIM_FALLBACK_SITES=1
// the `E.probe` scope must report `runs-compiled`.
class B;
  function bit wide9(logic [8:0] v); return v === 9'd300; endfunction
endclass
function bit free9(logic [8:0] v); return v === 9'd300; endfunction
function bit free_n8(logic [7:0] v); return v === 8'd44; endfunction
class E extends B;
  logic [7:0] a = 8'd200, b = 8'd100;
  function int probe();
    int bad = 0;
    if (!super.wide9(a + b)) bad |= 1;
    if (!wide9(a + b)) bad |= 2;
    if (!free9(a + b)) bad |= 4;
    if (!free_n8(a + b)) bad |= 8;
    return bad;
  endfunction
endclass
module top;
  E e;
  int i, cold, hot, fails;
  initial begin
    e = new;
    fails = 0;
    cold = e.probe();
    $display("  info cold probe (AST) bad bits=%0d", cold);
    if (cold != 0) begin fails++; $display("  FAIL cold bad bits=%0d", cold); end
    for (i = 0; i < 1500; i++) void'(e.probe());
    hot = 0;
    for (i = 0; i < 500; i++) hot |= e.probe();
    if (hot != 0) begin fails++; $display("  FAIL hot compiled probe, bad bits=%0d", hot); end
    else $display("  ok   hot compiled probe, bad bits=0");
    if (fails == 0) $display("TAG_PASS");
    else            $display("TAG_FAIL count=%0d", fails);
    $finish;
  end
endmodule
