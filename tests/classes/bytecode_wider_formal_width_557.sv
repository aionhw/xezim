// =====================================================================
// xezim issue 528 -- the bytecode-compiled path re-derives widths on its own
//
// `class_member_operand_width.sv` covers the AST-interpreted half of
// issue 528.  A class method that is called often enough (>= 1000 times,
// see `method_tier_threshold`) stops being interpreted and runs from
// bytecode compiled by `BytecodeCompiler`, which does NOT share the
// simulator's width resolver -- it bakes its own context/self widths in
// `lrm_self_width` / `expr_max_width`.  So every shape fixed on the
// interpreted side has to be re-proved on the compiled side.
//
// The checks below are the issue-528 shapes that the bytecode compiler
// accepts (the rest bail to the AST interpreter with
// `[FALLBACK] compile-fail reason=Expr_MemberAccess` and would silently
// test the wrong path).  Each is evaluated TWICE:
//
//   cold -- one call, still on the AST interpreter
//   hot  -- after 1500 calls, from compiled bytecode
//
// and the two must agree with the reference result.  A cold-only pass
// proves nothing about the compiled path, and a hot-only pass proves
// nothing if the method silently declined to compile, so both are
// required.
//
// Confirm the method really compiled before trusting a green run:
//
//   XEZIM_FALLBACK_SITES=1 xezim bytecode_wider_formal_width.sv 2>&1 \
//       | grep FALLBACK
//
// every `E.probe*` scope must read `runs-compiled`, never `compile-fail`.
//
// Reference behaviour: the reference simulator prints `TAG_PASS` (0 failures).
// Before this change xezim printed `TAG_FAIL`: the wider-call-formal shape is evaluated at the
// 8-bit SELF width of `a + b` (44, so `wide9` sees 44 !== 300) instead of
// the 9-bit width of `wide9`'s formal.  Every other shape here already
// agrees with the reference on both tiers.
// =====================================================================

typedef struct packed { bit [7:0] sa, sb; } st_t;

class E;
  bit  [7:0] x, y, a, b, vec;
  bit  [7:0] arr [4];
  int  ia, ib;
  bit signed [7:0] sa, sb;

  function new();
    x = 200; y = 100; a = 200; b = 100; vec = 8'hC8;
    arr = '{200, 100, 200, 100};
    ia = 200; ib = 100;
    sa = 8'hC8; sb = 8'h64;
  endfunction

  function bit [7:0] f8a(); return 8'd200; endfunction
  function bit [7:0] f8b(); return 8'd100; endfunction

  // The formal is 9 bits, so `v`'s context must widen the 8-bit `+` that
  // the caller passes in.  This is the shape the compiler gets wrong.
  function bit wide9(logic [8:0] v); return v === 300; endfunction

  // One issue-528 shape per bit.  `s` is 9 bits and holds 300, so every
  // comparison below must be evaluated at 9 bits or wider.
  function int probe(logic [8:0] s);
    int n;
    logic [8:0] r;
    n = 0;
    if (!(s === x + y))                               n |= 1;
    if (!(s === this.x + this.y))                     n |= 2;
    if (!(s === arr[1] + arr[2]))                     n |= 4;
    if (!(s === (vec & 8'hff) + 8'd100))              n |= 8;
    if (!(s === ia + ib))                             n |= 16;
    if (!(s === f8a() + f8b()))                       n |= 32;
    if (!((a + b) >= 300))                            n |= 64;
    if (!((a + b) <= 300))                            n |= 128;
    if (!(((a + b) >= 300) && ((a + b) <= 300)))      n |= 256;
    if (!((-x) === 9'd312))                           n |= 2048;
    if (!((~x) === 9'd311))                           n |= 4096;
    if (!(((a + b) << 1) === 10'd600))                n |= 8192;
    if (!(s === $signed(a) + $signed(b)))             n |= 16384;
    if (!wide9(a + b))                                n |= 32768;
    if (!(s === sa + sb))                             n |= 65536;
    if (!(((1'b1 ? a : 8'd0) + (1'b1 ? b : 8'd0)) === s)) n |= 131072;
    r = a + b;
    if (!(r === 300))                                 n |= 262144;
    if (!({1'b0, a + b} === 9'd44))                   n |= 524288;
    return n;
  endfunction

  // Same shapes, reached through a second method so the compiler sees a
  // second body; guards against a per-method fix that does not generalise.
  function int probe2(logic [8:0] s);
    int n;
    n = 0;


    if (!(s === x + y))                               n |= 1;
    if (!(((a + b) >= 300) && ((a + b) <= 300)))      n |= 2;
    if (!wide9(a + b))                                n |= 4;
    if (!(s === $signed(a) + $signed(b)))             n |= 8;
    return n;
  endfunction
endclass

module top;
  logic [8:0] s;
  E e;
  int i, cold, hot, acc;
  int fails;

  initial begin
    fails = 0;
    s = 300;
    e = new;

    // ---- cold: one call, still on the AST interpreter ------------------
    // Informational only.  The interpreted half of the wider-formal shape is
    // tracked by tests/classes/call_actual_formal_width.sv; counting it
    // here would make this test fail for a reason the bytecode compiler does
    // not own.
    cold = e.probe(s);
    $display("  info cold probe (AST)      bad bits=%0d", cold);
    cold = e.probe2(s);
    $display("  info cold probe2 (AST)     bad bits=%0d", cold);

    // ---- hot: warm up PAST the compile threshold, then measure ---------
    // The warm-up is discarded: the tier counter only compiles a method
    // after 1000 genuine calls, so measuring from call 0 would fold the
    // interpreted calls (tracked by call_actual_formal_width.sv) into the
    // compiled result and report the wrong engine's failures.
    for (i = 0; i < 1500; i++) begin
      void'(e.probe(s));
      void'(e.probe2(s));
    end
    acc = 0;
    for (i = 0; i < 500; i++) begin
      acc |= e.probe(s);
      acc |= e.probe2(s);
    end
    hot = acc;
    if (hot != 0) begin
      fails++;
      $display("  FAIL %-4d hot compiled probe, bad bits=%0d", 1, hot);
    end else begin
      $display("  ok   %-4d hot compiled probe, bad bits=0", 1);
    end

    $display("TOTAL_FAILS=%0d", fails);
    if (fails == 0) $display("TAG_PASS");
    else            $display("TAG_FAIL count=%0d", fails);
    $finish;
  end
endmodule