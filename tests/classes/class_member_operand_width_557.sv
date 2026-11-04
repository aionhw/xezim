// =====================================================================
// xezim issue 528 -- class-member operands lose their comparison context
//
// LRM 11.6.1 / 11.8.2: the operands of a relational or equality operator
// are CONTEXT-determined.  In `s === c.a + c.b` the 9-bit `s` must widen
// the 8-bit operands of `+` to 9 bits, so the sum is 300.  xezim evaluated
// `c.a + c.b` at its SELF width (8 bits -> 44) whenever an operand's
// static width was not already known, and that happened for every
// member / handle-select-shaped operand inside a class method.
//
// This is the RESOLVED half of the issue 528 matrix -- the 64 checks that
// the declaration-based width resolution in
// `Simulator::declared_select_width` (src/compiler/simulator/jit_runtime.rs)
// brings into line with the reference simulators.  The shapes that still fall
// through are isolated in a separate test (not part of this change); the wider-formal
// call-actual shape and its bytecode-compiled variant are in
// `call_actual_formal_width.sv` and `bytecode_wider_formal_width.sv`, and the
// owner-qualified shape likewise (not part of this change).
//
// Reference behaviour: the reference simulator prints `TAG_PASS` (0 failures).
//
// Build note: keep every class-member reference inside procedural code --
// The reference simulator rejects assigning class members outside a
// procedural block.
// =====================================================================

package p528_pkg;
  bit [7:0] pa, pb;
endpackage

interface add_if;
  logic [7:0] q1, q2;
  logic [8:0] sum;
endinterface

typedef struct packed { bit [7:0] sa, sb; } st_t;

// ---- innermost class: reached through a handle ----------------------
class CC;
  bit [7:0] a, b;
endclass

// ---- class whose methods are the failing scope ----------------------
class E;
  bit  [7:0] x, y;
  bit  [7:0] a, b;                // read through a handle (c.a, arg.a)
  bit  [7:0] vla, vlb;
  bit  [7:0] arr [4];
  bit  [7:0] vec;
  CC   cc;
  st_t st;
  virtual add_if vif;
  int  ia, ib;                    // 32-bit members (never truncate)
  bit signed [7:0] sa, sb;        // signed 8-bit members
  bit tb_ok;
  E c_g;                          // used by the class-task check
  E    c2;                        // second handle, used by check 4
  static bit [7:0] ta, tb;        // class-static properties

  function new();
    cc = new;
    arr = '{200, 100, 200, 100};
    cc.a = 200; cc.b = 100;
    x = 200; y = 100; a = 200; b = 100;
    vla = 200; vlb = 100;
    vec = 8'hC8;
    st.sa = 200; st.sb = 100;
    ia = 200; ib = 100;
    sa = 8'hC8; sb = 8'h64;
  endfunction

  function bit [7:0] f8a(); return 8'd200; endfunction
  function bit [7:0] f8b(); return 8'd100; endfunction
  function bit t_chk(logic [8:0] s); return s === x + y; endfunction

  // Report one check.  `g` is the boolean under test.
  function void one(int r, bit g, string nm, ref int fails);
    if (!g) begin
      fails++;
      $display("  FAIL %-4d %s", r, nm);
    end else begin
      $display("  ok   %-4d %s", r, nm);
    end
  endfunction

  // `s` is 9 bits and holds 300; `c`/`arg` are same-class handles.
  function void run(logic [8:0] s, E c, E arg, CC d, ref int fails);
    // ---- 1. the original 528 shapes --------------------------------
    one(  1, s === x + y,               "bare x + y", fails);
    one(  2, s === this.x + this.y,     "this.x + this.y", fails);
    one(  3, s === c.a + c.b,           "c.a + c.b (handle prop)", fails);
    one(  4, s === this.c2.x + this.c2.y, "this.c2.x + this.c2.y", fails);
    one(  5, s === arg.a + arg.b,       "arg.a + arg.b (formal)", fails);
    one(  6, s === d.a + d.b,           "d.a + d.b (2-level handle)", fails);
    // ---- 2. other aggregate shapes ---------------------------------
    one(  7, s === st.sa + st.sb,       "packed struct st.sa+st.sb", fails);
    one(  8, s === c.st.sa + c.st.sb,   "c.st.sa + c.st.sb", fails);
    one(  9, s === vla + vlb,           "member vla + vlb", fails);
    one( 10, s === arr[1] + arr[2],     "arr[1] + arr[2]", fails);
    one( 11, s === c.arr[0] + c.arr[1], "c.arr[0] + c.arr[1]", fails);
    one( 12, s === (vec & 8'hff) + 8'd100, "(vec & 8'hff) + 100", fails);
    one( 13, s === ia + ib,             "int ia + ib", fails);
    // ---- 3. virtual-interface members ------------------------------
    one( 14, vif.sum === s,             "vif.sum === s", fails);
    one( 15, s === vif.q1 + vif.q2,     "vif.q1 + vif.q2", fails);
    // ---- 4. static / package variables -----------------------------
    one( 16, s === E::ta + E::tb,       "E::ta + E::tb (static)", fails);
    one( 17, s === p528_pkg::pa + p528_pkg::pb, "pkg::pa + pkg::pb", fails);
    // ---- 5. function results ---------------------------------------
    one( 18, s === f8a() + f8b(),       "f8a() + f8b()", fails);
    one( 19, s === c.f8a() + c.f8b(),   "c.f8a() + c.f8b()", fails);
    // ---- 6. relational / equality operators ------------------------
    one( 20, (c.a + c.b) >= 300,        "(c.a+c.b) >= 300", fails);
    one( 21, (c.a + c.b) <= 300,        "(c.a+c.b) <= 300", fails);
    one( 22, (c.a + c.b) == 300,        "(c.a+c.b) == 300", fails);
    one( 23, !((c.a + c.b) != s),       "(c.a+c.b) != s is false", fails);
    one( 24, (x + y) >= 300,            "(x+y) >= 300", fails);
    one( 25, (x + y) <= 300,            "(x+y) <= 300", fails);

    // ---- 7. context propagation through nesting --------------------
    one( 26, s + (c.a + c.b) === 9'd88, "s+(c.a+c.b) === 88 (self)", fails);
    one( 27, (c.a + c.b) + (x + y) === 10'd600, "(c.a+c.b)+(x+y) === 600", fails);
    one( 28, s === (1'b1 ? c.a : 8'd0) + (1'b1 ? c.b : 8'd0), "ternary arms add", fails);
    one( 29, (c.a + c.b) inside {300},  "(c.a+c.b) inside {300}", fails);
    one( 30, s inside {c.a + c.b},      "s inside {c.a+c.b}", fails);
    one( 31, ((c.a + c.b) >= 300) && ((c.a + c.b) <= 300), "range via &&", fails);
    one( 32, (1'b1 -> (s === c.a + c.b)), "1 -> (s === c.a+c.b)", fails);
    one( 34, (c.a + c.b) === 8'd44,     "self-determined stays 44", fails);
    one( 35, {1'b0, c.a + c.b} === 9'd44, "{0,c.a+c.b} self-det. 44", fails);
    // ---- 8. unary / shift operators are context-determined ---------
    one( 36, (-x) === 9'd312,           "-x === 312 (9-bit ctx)", fails);
    one( 37, (~x) === 9'd311,           "~x === 311 (9-bit ctx)", fails);
    one( 38, (c.a << 1) === 9'd400,     "c.a<<1 === 400", fails);
    one( 39, ((c.a + c.b) << 1) === 10'd600, "(c.a+c.b)<<1 === 600", fails);
    one( 40, s === -(-c.a) + c.b,       "s === -(-c.a)+c.b", fails);
    one( 41, s === $signed(c.a) + $signed(c.b), "$signed(c.a)+$signed(c.b)", fails);
    one( 42, s === sa + sb,             "signed sa+sb", fails);
    one( 43, s === c.sa + c.sb,         "signed c.sa+c.sb", fails);

    // ---- 9. statement contexts -------------------------------------
    if (s === c.a + c.b) one( 44, 1'b1, "if (s === c.a+c.b)", fails);
    else                 one( 44, 1'b0, "if (s === c.a+c.b)", fails);
    case (c.a + c.b)
      300:     one( 45, 1'b1, "case(c.a+c.b) hits 300", fails);
      default: one( 45, 1'b0, "case(c.a+c.b) hits 300", fails);
    endcase
    begin
      int guard;
      guard = 0;
      while (s > c.a + c.b) begin
        guard++;
        if (guard > 5) guard = 99;
      end
      one( 46, guard == 0, "while (s > c.a+c.b) never runs", fails);
    end
    begin
      one( 47, t_chk(s), "class function body compare", fails);
    end
    begin
      logic [8:0] r;
      r = c.a + c.b;
      one( 49, r === 300, "r = c.a+c.b; r === 300", fails);
    end
  endfunction

  task t_body(logic [8:0] s); tb_ok = (s === c_g.a + c_g.b); endtask

endclass

module top;
  int fails;
  int i, extra;
  add_if ifc();
  logic [8:0] s;
  st_t        mst;
  bit  [7:0]  mx, my, marr [4];
  E  e, e2, earg;
  CC d;

  // module-scope helper: same shape as the class method
  function automatic void mchk(int r, bit g, string nm, ref int n);
    if (!g) begin n++; $display("  FAIL %-4d %s", r, nm); end
    else    begin      $display("  ok   %-4d %s", r, nm); end
  endfunction

  function automatic bit [7:0] mf8a(); return 8'd200; endfunction
  function automatic bit [7:0] mf8b(); return 8'd100; endfunction

  initial begin
    fails = 0;
    s   = 300;
    e   = new; e2 = new; earg = new; d = new;
    e.c2 = e2;
    e.c_g = e2;
    e.vif = ifc;
    ifc.q1 = 200; ifc.q2 = 100; ifc.sum = 300;
    E::ta = 200; E::tb = 100;
    p528_pkg::pa = 200; p528_pkg::pb = 100;
    mx = 200; my = 100;
    marr[0] = 200; marr[1] = 100; marr[2] = 200; marr[3] = 100;
    mst.sa = 200; mst.sb = 100;
    d.a = 200; d.b = 100;

    // =================================================================
    $display("== A. class-method scope (cold: AST interpreter) ==");
    e.run(s, e2, earg, d, fails);

    // =================================================================
    $display("== A2. class-task scope ==");
    e.t_body(s);
    mchk(50, e.tb_ok, "class task body: s === c_g.a + c_g.b", fails);


    // =================================================================
    $display("== C. module scope ==");
    mchk(60, s === mx + my,            "mx + my", fails);
    mchk(61, s === e.x + e.y,           "e.x + e.y", fails);
    mchk(62, s === e2.a + e2.b,         "e2.a + e2.b", fails);
    mchk(63, s === d.a + d.b,           "d.a + d.b", fails);
    mchk(64, s === mst.sa + mst.sb,     "mst.sa + mst.sb", fails);
    mchk(66, s === marr[1] + marr[2],   "marr[1] + marr[2]", fails);
    mchk(68, s === ifc.q1 + ifc.q2,     "ifc.q1 + ifc.q2", fails);
    mchk(70, s === p528_pkg::pa + p528_pkg::pb, "pkg::pa + pkg::pb", fails);
    mchk(71, s === mf8a() + mf8b(),     "mf8a() + mf8b()", fails);
    mchk(72, (e2.a + e2.b) >= 300,      "(e2.a+e2.b) >= 300", fails);
    mchk(73, (e2.a + e2.b) <= 300,      "(e2.a+e2.b) <= 300", fails);
    mchk(74, (e2.a + e2.b) inside {300},"(e2.a+e2.b) inside {300}", fails);
    mchk(75, ((e2.a + e2.b) >= 300) && ((e2.a + e2.b) <= 300), "range via &&", fails);
    mchk(76, (e2.a << 1) === 9'd400,    "e2.a<<1 === 400", fails);
    mchk(77, s === e2.sa + e2.sb,       "e2.sa + e2.sb (signed)", fails);

    // =================================================================
    $display("TOTAL_FAILS=%0d", fails);
    if (fails == 0) $display("TAG_PASS");
    else            $display("TAG_FAIL count=%0d", fails);
    $finish;
  end
endmodule