// =====================================================================
// xezim issue 528 -- a call ACTUAL takes the callee FORMAL's width
//
// LRM 11.6.1 / 11.8.2 make an expression's operands context-determined up to
// the widest context that bears on them, and a subroutine call is one of those
// contexts: the DECLARED width of the FORMAL is the context of the ACTUAL.
// With 8-bit `a = 200` and `b = 100`:
//
//     function bit wide9(logic [8:0] v); return v === 300; endfunction
//     wide9(a + b);      // 9-bit context -> 300 -> 1'b1
//
// evaluated SELF-determined, the '+' keeps the operands' own 8 bits, wraps to
// 44, and the call answers 1'b0.  The AST interpreter bound every plain scalar
// formal by evaluating the actual self-determined and RESIZING it afterwards;
// resizing only zero-extends the value that already wrapped, so the carry is
// gone before the formal ever sees it.  The actual must be evaluated AT the
// formal's width in the first place.
//
// Every call here is made a handful of times, so each callee stays below the
// 1000-call tier that switches a method body to the bytecode executor: this
// file is the INTERPRETER half of the rule.  `bytecode_wider_formal_width.sv`
// is the compiled half, `class_member_operand_width.sv` the comparison half.
//
// Checks 1-14 reach the class-method binder, checks 15-16 the module-scope
// function binder -- the two places that bind a plain scalar formal:
//
//   01  wide9(a + b)          bare call, `this`-bounded actual
//   02  this.wide9(a + b)     explicit `this`
//   03  c.wide9(a + b)        RECEIVER form -- same binder, reached through a
//                             handle rather than bare
//   05  wide12(a + b)         a third width, to catch a hard-coded 9
//   06  wide32(a + b)         built-in `int` formal (IntegerAtom, not a
//                             literal-range vector)
//   07  mix98(a + b, a + b)   two formals of DIFFERENT widths: each actual is
//                             sized by ITS OWN formal, not the widest
//   08  wide11s(a + b)        SIGNED formal (300 fits 11-bit signed)
//   09  narrow8(a + b)        CONTROL: an 8-bit formal must NOT widen -- the
//                             sum stays 44
//   10  wide9sum(sum32(a,b))  a call actual whose OWN actuals are widened by
//                             ITS formals
//   12  t9(a + b)             a TASK formal; the verdict lands in a member.
//                             Entered from `run_task`, NOT from `run`: LRM
//                             13.4 forbids a function from calling a task.
//   13  wide9d()              an omitted actual still binds its declared
//                             default, unshifted by the new context
//   14  refecho(xr)           a `ref` formal keeps the self-determined path:
//                             an lvalue actual is the caller's own variable
//   15  fwide9(x + y)         a plain module-scope FUNCTION, pinning the
//                             free-function binder to the same rule
//   04  e.wide9(x + y)        receiver form called from MODULE scope
//   16  e.narrow8(x + y)      CONTROL from module scope, no widening
//   17  wide9(c.a + c.b)      the operands are class members reached through a
//                             HANDLE FORMAL, and the caller is itself a method
//                             entered through a 9-bit actual -- the one shape
//                             that survived the other three fixes
//
// Check 11 was dropped.  It asserted that an outer 9-bit context must not leak
// through a call into a NESTED callee's own 8-bit formals
// (`!wide9v(sum8(a, b))`).  Every reference simulator tested disagrees with
// that expectation and reports the nested sum as 300, so the check encoded an
// assumption about nested context propagation that this branch neither fixes
// nor needs.  The non-leak guarantee it was meant to protect is still covered,
// one level down and uncontroversially, by check 9 (`narrow8(a + b) == 44`) and
// check 16 (`e.narrow8(x + y) == 44`) -- an 8-bit formal does not get widened.
//
// Reference behaviour: the reference simulator prints `TAG_PASS` (0 failures).
// =====================================================================

class E;
  bit [7:0] a, b;
  bit       r9;              // written by the task in check 12

  function new();
    a = 200; b = 100;
    r9 = 0;
  endfunction

  // The formal is wider than either operand, so only the formal's context can
  // keep the carry out of `a + b`.
  function bit wide9   (logic [8:0]         v); return v === 300; endfunction
  function bit wide12  (logic [11:0]        v); return v === 300; endfunction
  function bit wide32  (int                 v); return v === 300; endfunction
  function bit wide11s (logic signed [10:0] v); return v === 300; endfunction
  function bit narrow8 (logic [7:0]         v); return v === 44;  endfunction

  // Two formals, two widths: each actual is sized by ITS OWN formal.
  function bit mix98(logic [8:0] v, logic [7:0] w);
    return v === 300 && w === 44;
  endfunction

  // Check 10: a call as the actual. Its arguments are sized by ITS OWN
  // formals -- sum32's 9-bit formals are what keep the carry, not the outer
  // comparison. (The mirror-image control, an 8-bit formal behind a 9-bit
  // context, is check 9; see the note by check 11 in the header for why the
  // nested variant of that control was dropped.)
  function int sum32(logic [8:0] x, logic [8:0] y); return x + y; endfunction
  function bit wide9sum(logic [8:0] v); return v === 300; endfunction

  // Check 13: an omitted actual binds this default, which must still be read
  // as the 300 the declaration says.
  function bit wide9d(logic [8:0] v = 300); return v === 300; endfunction

  // Check 14: a `ref` formal is an lvalue, not an expression; the actual is
  // the caller's own variable and keeps its declared width.
  function bit refecho(ref logic [7:0] x); return x === 8'd44; endfunction

  // Check 12: a task has no return value, so the verdict lands in a member.
  task t9(logic [8:0] v);
    r9 = (v === 300);
  endtask

  function void one(int r, bit g, string nm, ref int fails);
    if (!g) begin
      fails++;
      $display("  FAIL %-4d %s", r, nm);
    end else begin
      $display("  ok   %-4d %s", r, nm);
    end
  endfunction


  // Checks 1-14 run inside a method, where the caller sits in a class scope
  // and `this` is live.  Check 12 lives in `run_task` instead: LRM 13.4
  // forbids a function from calling a task.
  function void run(E c, ref int fails);
    logic [7:0] xr;
    one( 1, wide9(a + b),             "bare      wide9(a+b)",           fails);
    one( 2, this.wide9(a + b),        "this.     wide9(a+b)",           fails);
    one( 3, c.wide9(a + b),           "receiver  c.wide9(a+b)",         fails);
    one( 5, wide12(a + b),            "bare      wide12(a+b)",          fails);
    one( 6, wide32(a + b),            "bare      wide32(a+b) int",      fails);
    one( 7, mix98(a + b, a + b),      "bare      mix98(a+b,a+b)",       fails);
    one( 8, wide11s(a + b),           "bare      wide11s(a+b) signed",  fails);
    one( 9, narrow8(a + b),           "CONTROL   narrow8(a+b)==44",     fails);
    one(10, wide9sum(sum32(a, b)),    "nested    wide9sum(sum32(a,b))", fails);
    one(13, wide9d(),                 "default   wide9d()",             fails);
    xr = 44;
    one(14, refecho(xr),              "ref       refecho(xr)",          fails);
  endfunction

  // Check 12: a task has no return value, so the verdict lands in a member.
  // The task call cannot sit in `run`: LRM 13.4 says a function may not call a
  // task, so this is a task of its own, entered from the initial block.
  task run_task(E c, ref int fails);
    t9(a + b);
    one(12, r9,                       "task      t9(a+b)",              fails);
  endtask

  // Check 17: the operands are class members reached through a HANDLE FORMAL
  // (`c`, not `this`) while the caller itself sits in a class scope and was
  // entered through a 9-bit actual.  Only the callee's declared formal width
  // can keep the carry out of `c.a + c.b`, so this is the one shape that
  // survived the member-width, bytecode and plain-call fixes.
  //
  // (The direct comparison `s === c.a + c.b` in the same method is ALSO wrong,
  // but it is wrong identically on main and on every branch here -- a separate
  // defect, so it is left to a separate test instead of hiding a
  // red check inside this MWE.)
  function void run_formal_handle(logic [8:0] s, E c, ref int fails);
    one(17, wide9(c.a + c.b), "formal-handle wide9(c.a+c.b)", fails);
  endfunction
endclass

module top;
  int fails;
  logic [7:0] x, y;
  logic [7:0] xr;
  E e;

  function automatic void mchk(int r, bit g, string nm, ref int n);
    if (!g) begin n++; $display("  FAIL %-4d %s", r, nm); end
    else    begin      $display("  ok   %-4d %s", r, nm); end
  endfunction

  // Check 15's callee at module scope: a plain function, not a class method,
  // so this pins the free-function binder to the same rule.
  function automatic bit fwide9(logic [8:0] v); return v === 300; endfunction

  initial begin
    fails = 0;
    x = 200; y = 100;
    e = new;

    e.run(e, fails);
    e.run_task(e, fails);
    e.run_formal_handle(9'd300, e, fails);

    mchk( 4, e.wide9(x + y),    "module     e.wide9(x+y)",       fails);
    mchk(15, fwide9(x + y),     "module     fwide9(x+y) fn",     fails);
    xr = 44;
    mchk(16, e.narrow8(x + y),  "CONTROL    e.narrow8(x+y)==44", fails);

    $display("TOTAL_FAILS=%0d", fails);
    if (fails == 0) $display("TAG_PASS");
    else            $display("TAG_FAIL count=%0d", fails);
    $finish;
  end
endmodule