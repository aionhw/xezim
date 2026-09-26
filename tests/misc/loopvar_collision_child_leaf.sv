// loopvar-leaf-collision — interpreter loop-variable resolution vs. inlined
// child instance leaf names. The packed-2D-array loop-reset failure family:
// the original MWE plus all 11 analysis probes, combined into ONE
// self-checking matrix (tests/classes SVTEST_* macro style).
//
// Bug family (full source-level path analysis in SOURCE_ROOT_CAUSE.md):
//   * An interpreted `for (int i = ...)` / `foreach (arr[k])` writes its
//     index variable under the BARE name into the runtime signals map, but
//     every READ of the same name goes through hierarchical name
//     resolution, whose single-segment leaf-name heuristic binds the bare
//     name to an inlined child instance's same-named signal (`u.i`).
//     Write and read then address two different storage cells.
//   * 4-state child (`integer`, X-init): X condition, zero loop iterations
//     (p1, p3). 2-state child (`int`, 0-init): the loop silently HIJACKS and
//     mutates the child's storage (p2, p11). A child that also re-declares
//     the parent's array receives the parent's NBAs through
//     resolution-hint poisoning (p4). A hierarchical write into the child
//     changes the parent's loop count — proof of aliased storage (p5).
//     Two children sharing the leaf make the pick design-arbitrary (p8).
//
// Controls that must stay green (frames / exact tables are consulted before
// the leaf heuristic there): compiled loop fast path (p10), block-local
// declarations (p6), automatic-task locals (p7), static-task locals (p9).
//
// Design notes — why the merged matrix stays faithful to the per-probe
// reproductions:
//   * Every probe owns UNIQUE leaf names (i1, i2, k3, i4/arr4, i5, v6, loc7,
//     j8, loc9, i10, i11/seen11), so the leaf-heuristic candidate set for
//     each loop variable is exactly the original probe's; merging the probes
//     cannot change which instance a name binds to.
//   * Deliberate exceptions that ARE the point: p4's child re-declares the
//     parent's `arr4` (hint-poisoning redirect), and p8's two children share
//     `j8` (ambiguous candidate set).
//   * The checker reads each bare name before any hierarchical name of the
//     same probe, so a poisoned resolution hint can never redirect the
//     checker itself on a buggy build.
//   * Deterministic: `$urandom` sits in a permanently dead branch (`req_valid`
//     is never true) — it is only there to make the loop body non-simple,
//     which is what forces the AST-interpreter path (loop-bail) in xezim.
//
// Expected: on the unfixed build the seven collision probes (p1-p5, p8, p11)
// fail their checks and the four controls stay green (TEST_FAIL count=11-12);
// a correct simulator (and a fixed build) prints TEST_PASS.
//
// Run:  xezim loopvar_leaf_collision.sv -s tb_top --module-timescale 1ps/1ps --max-time 40
//       xrun  loopvar_leaf_collision.sv -sv -timescale 1ps/1ps -exit

`timescale 1ps/1ps

`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
int failures = 0;

`define SVTEST_CHECK(expr, msg) \
if (!(expr)) begin \
  failures++; \
  $display("FAIL @%0t : %s", $time, msg); \
end

`define SVTEST_PASSFAIL \
if (failures == 0) begin \
  $display("TEST_PASS"); \
end else begin \
  $display("TEST_FAIL count=%0d", failures); \
  $fatal(1); \
end

`endif

// ---- one child module per probe (each owns its probe's colliding leaf) ----
module d1_c();  integer i1;                 endmodule // p1: 4-state (X-init)
module d2_c();  int    i2;                  endmodule // p2: 2-state (0-init)
module d3_c();  integer k3;                 endmodule // p3: foreach sibling
module d4_c();  integer i4; logic [1:0][7:0] arr4; endmodule // p4: + same-named arr
module d5_c();  integer i5;                 endmodule // p5
module d6_c();  integer v6;                 endmodule // p6 CONTROL
module d7_c();  integer loc7;               endmodule // p7 CONTROL
module d8a_c(); integer j8;                 endmodule // p8: candidate A
module d8b_c(); integer j8;                 endmodule // p8: candidate B
module d9_c();  integer loc9;               endmodule // p9 CONTROL
module d10_c(); integer i10;                endmodule // p10 CONTROL
module d11_c(input logic clk);                        // p11: child USES its i
  int i11;
  int seen11 = 0;
  always @(posedge clk) begin
    if (i11 == 0) seen11 <= seen11 + 1;  // child relies on i11 staying 0
  end
endmodule


module tb_top;
  `SVTEST_INIT

  logic clk = 1'b0;
  logic req_valid = 1'b0;   // dead-branch guard: never true

  logic [1:0][7:0] arr1;    // p1 target array
  logic [1:0][7:0] arr2;    // p2
  logic [1:0][7:0] arr3;    // p3
  logic [1:0][7:0] arr4;    // p4 (child u4 re-declares this name: redirect)
  logic [1:0][7:0] arr5;    // p5
  logic [1:0][7:0] arr8;    // p8
  logic [1:0][7:0] arr10;   // p10 CONTROL
  logic [1:0][7:0] arr11;   // p11

  logic p6_ok = 1'b0;       // p6 CONTROL flag (set by the named block)
  logic p7_ok = 1'b0;       // p7 CONTROL flag (set by the automatic task)
  logic p9_ok = 1'b0;       // p9 CONTROL flag (set by the static task)

  d1_c  u1();
  d2_c  u2();
  d3_c  u3();
  d4_c  u4();
  d5_c  u5();
  d6_c  u6();
  d7_c  u7();
  d8a_c u8a();
  d8b_c u8b();
  d9_c  u9();
  d10_c u10();
  d11_c u11(.clk(clk));

  initial forever #500 clk = ~clk;

  // p1: for-init VarDecl loop var vs. child `integer i1` (the MWE shape).
  // The dead $urandom NBA makes the body non-simple -> loop-bail -> the
  // whole loop runs on the AST interpreter path.
  always @(posedge clk) begin
    for (int i1 = 0; i1 < 2; i1++) begin
      arr1[i1] <= '0;
      if (req_valid) arr1[0] <= $urandom;
    end
  end

  // p2: 2-state `int i2` in the child — the hijack is SILENT (loop appears
  // to work) but it reads/steps/mutates the child's storage.
  always @(posedge clk) begin
    for (int i2 = 0; i2 < 2; i2++) begin
      arr2[i2] <= '0;
      if (req_valid) arr2[0] <= $urandom;
    end
  end

  // p3: foreach index var vs. child `integer k3` — same family, twin guard.
  always @(posedge clk) begin
    foreach (arr3[k3]) begin
      arr3[k3] <= '0;
      if (req_valid) arr3[k3] <= $urandom;
    end
  end

  // p4: child also declares `arr4` — after i4 leaf-resolves into u4, the
  // poisoned hint makes the parent's bare `arr4` resolve to `u4.arr4`.
  always @(posedge clk) begin
    u4.i4 = 1;   // force the aliased loop var to a true condition once
    for (int i4 = 0; i4 < 2; i4++) begin
      arr4[i4] <= '0;
      if (req_valid) arr4[0] <= $urandom;
    end
  end

  // p5: hierarchical write to the child's i5 — if the loop var aliases it,
  // the loop now runs exactly one iteration instead of two.
  always @(posedge clk) begin
    u5.i5 = 1;
    for (int i5 = 0; i5 < 2; i5++) begin
      arr5[i5] <= '0;
      if (req_valid) arr5[0] <= $urandom;
    end
  end

  // p6 CONTROL: block-local declaration — frame/signals-map precedence.
  always @(posedge clk) begin
    begin : blk6
      integer v6;
      v6 = 5;
      v6 = v6 + 1;
      p6_ok <= (v6 == 6);
    end
  end

  // p7 CONTROL: automatic-task local — call-frame precedence.
  task automatic t7();
    integer loc7;
    begin
      loc7 = 5;
      loc7 = loc7 + 1;
      p7_ok <= (loc7 == 6);
    end
  endtask
  always @(posedge clk) t7();

  // p8: two children both declaring `j8` — the leaf heuristic picks one
  // design-arbitrarily (hint-guided); the hierarchical write to u8a steers it.
  always @(posedge clk) begin
    u8a.j8 = 1;
    for (int j8 = 0; j8 < 2; j8++) begin
      arr8[j8] <= '0;
      if (req_valid) arr8[0] <= $urandom;
    end
  end

  // p9 CONTROL: static-task local — static-local storage precedence.
  task t9();
    integer loc9;
    begin
      loc9 = 5;
      loc9 = loc9 + 1;
      p9_ok <= (loc9 == 6);
    end
  endtask
  always @(posedge clk) t9();

  // p10 CONTROL: simple loop body (no $urandom) — compiled fast path; the
  // compiled side has no leaf heuristic and carries for_loop_var_ids.
  always @(posedge clk) begin
    for (int i10 = 0; i10 < 2; i10++) begin
      arr10[i10] <= '0;
    end
  end

  // p11: the child USES its own `int i11`; the parent's loop hijacking it
  // corrupts the child's own always logic (cross-process corruption).
  always @(posedge clk) begin
    for (int i11 = 0; i11 < 2; i11++) begin
      arr11[i11] <= '0;
      if (req_valid) arr11[0] <= $urandom;
    end
  end

  // ---- checker: bare names are read before hierarchical ones so a
  // ---- poisoned resolution hint can never redirect the checker itself.
  initial begin
    repeat (3) @(posedge clk);
    #1;

    // p1: 4-state sibling — loop reads X, so zero iterations.
    `SVTEST_CHECK(arr1 === '0 && u1.i1 === 'x,
      "p1: for-init loop var i1 bound to child u1.i1 (X) -> loop ran 0 iterations, arr1 never cleared")

    // p2: 2-state sibling — silent hijack: loop works but mutates child.
    `SVTEST_CHECK(arr2 === '0 && u2.i2 == 0,
      "p2: for-init loop var i2 hijacked child u2.i2 (2-state): loop ran on the child's storage and left i2 != 0")

    // p3: foreach index — same drop as p1.
    `SVTEST_CHECK(arr3 === '0 && u3.k3 === 'x,
      "p3: foreach index k3 bound to child u3.k3 (X) -> body never ran, arr3 never cleared")

    // p4: hint poisoning — parent's NBAs redirected into the child.
    `SVTEST_CHECK(arr4 === '0 && u4.arr4 === 'x && u4.i4 == 1,
      "p4: poisoned hint redirected parent NBAs into child u4 (tb arr4 uncleared / child arr4 written / u4.i4 stepped to 2)")

    // p5: hierarchical write changes the loop count -> aliased storage.
    `SVTEST_CHECK(arr5 === '0 && u5.i5 == 1,
      "p5: hierarchical u5.i5=1 changed the loop iteration count -> loop var aliased child storage (arr5 partial / u5.i5 != 1)")

    // p6 CONTROL: block-local.
    `SVTEST_CHECK(p6_ok === 1'b1 && u6.v6 === 'x,
      "p6 CONTROL: block-local v6 must compute 6 without touching child u6.v6")

    // p7 CONTROL: automatic-task local.
    `SVTEST_CHECK(p7_ok === 1'b1 && u7.loc7 === 'x,
      "p7 CONTROL: automatic-task local loc7 must compute 6 without touching child u7.loc7")

    // p8: two same-leaf children — arbitrary (hint-guided) pick.
    `SVTEST_CHECK(arr8 === '0 && u8a.j8 == 1 && u8b.j8 === 'x,
      "p8: two same-leaf children: loop bound j8 to a child instance (arr8 partial / u8a.j8 stepped / pick not isolated)")

    // p9 CONTROL: static-task local.
    `SVTEST_CHECK(p9_ok === 1'b1 && u9.loc9 === 'x,
      "p9 CONTROL: static-task local loc9 must compute 6 without touching child u9.loc9")

    // p10 CONTROL: compiled fast path.
    `SVTEST_CHECK(arr10 === '0 && u10.i10 === 'x,
      "p10 CONTROL: compiled loop path must zero arr10 without touching child u10.i10")

    // p11: cross-process corruption of the child's own logic.
    `SVTEST_CHECK(arr11 === '0 && u11.i11 == 0 && u11.seen11 == 3,
      "p11: parent loop hijacked child u11.i11 -> child's own always corrupted (i11 != 0 / seen11 != 3)")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule