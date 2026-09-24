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

package pk;
  // 296-bit packed struct: zero payload + req_number=1 -> value 1.
  typedef struct packed {
    logic [255:0] req_type;
    logic [31:0]  delay_counter;
    logic [7:0]   req_number;
  } req_t;

  // 304-bit wrapper for the 2-segment chained heads (c4, f8).
  typedef struct packed {
    req_t       inner;   // bits [303:8]
    logic [7:0] pad;     // bits [7:0]
  } outer_t;

  // 312-bit wrapper x2 for the 3-segment chained heads (f10/f11).
  typedef struct packed {
    outer_t     mid;     // bits [311:8]
    logic [7:0] pad2;    // bits [7:0]
  } outer_outer_t;

  // 16-bit struct for the ungated-seeding / non-class-field-name controls
  // (c5, f1/f2). `secret` is deliberately NOT declared by cls_a.
  typedef struct packed {
    logic [7:0] secret;      // bits [15:8]
    logic [7:0] req_number;  // bits [7:0]
  } tiny_t;

  // 40-bit struct for the method-hijack / method-name probes (c10, f12):
  // value 0x100 == 256.
  typedef struct packed {
    logic [31:0] tick;       // bits [39:8]
    logic [7:0]  req_number; // bits [7:0]
  } mt_t;
endpackage

import pk::*;

// Universal collision class: every primed heap object is a cls_a so that
// handle 1 carries `delay_counter` (write-divert gate), handle 256 carries
// the bare `tick()` function (read hijack), and NO object has `secret`.
class cls_a;
  int   delay_counter;
  req_t pfield;
  function int tick();
    return 77;
  endfunction
endclass

// c9: hierarchical target of the parent's member writes.
module sub;
  import pk::*;
  req_t sv;    // holds value 1 (colliding)
  req_t sv2;   // control: holds 0 (non-colliding)
  initial begin
    req_t e;
    e.req_type      = '0;
    e.delay_counter = 32'd0;
    e.req_number    = 8'd1;
    sv  = e;
    sv2 = '0;
  end
endmodule

// f6: interface-instance hierarchical head.
interface ifc_f;
  import pk::*;
  req_t iv;
endinterface

// f9: hierarchical member READ target. Fresh variable on purpose: c9's
// golden write already landed in u.sv (reference), so f9 must read an untouched
// field-0 value to prove the read-side divert.
module sub_r;
  import pk::*;
  req_t sv_sub;
  initial begin
    req_t e;
    e = '0;
    e.req_number = 8'd1;    // value 0 during the write: hygiene-safe
    sv_sub = e;             // whole-assignment: struct value == 1
  end
endmodule

module tb_top;
  import pk::*;
  `SVTEST_INIT

  sub   u();                    // c9
  ifc_f u_if();                 // f6
  sub_r u2();                   // f9

  // --- original matrix variables (t3, c1-c10) ------------------------------
  req_t  q[$];                 // t3
  req_t  sv;                   // c1 + c6 (reused)
  req_t  fa[2];                // c2
  req_t  da[];                 // c3
  outer_t ov;                  // c4
  tiny_t tq[$];                // c5
  req_t  aa[logic [7:0]];      // c7
  cls_a  h0, obj;              // h0 = primed handle 1; obj = c8 real handle
  int    rd, pre;              // c8 / follow-up reads
  req_t  tmp;                  // c8
  mt_t   msv;                  // c10

  // --- follow-up sweep variables (f1-f14) ----------------------------------
  req_t  q_nba[$], fq[$], eq7[$], qp[$];
  req_t  sv_nba, sv_ref, sv_rmw, sv_ps, e;
  outer_t ov_r;
  outer_outer_t oov, oov_r;
  tiny_t tv_c, tv_r;
  mt_t   msv_c, msv_c2, me;

  task automatic poke_f(ref req_t r);
    r.delay_counter = 32'd7;   // f5 surface: ref-formal head
  endtask
  initial begin
    tiny_t te;
    cls_a  c;

    // Heap primer: handles 1..257 live, all cls_a (superset of both halves:
    // c10/f12 need handle 256, f2 needs handle 257).
    h0 = new();                                // handle 1
    for (int i = 0; i < 256; i++) c = new();   // handles 2..257

    e.req_type      = '0;
    e.delay_counter = 32'd0;
    e.req_number    = 8'd1;                    // req_t value == 1

    // t3 (anchor): queue-element member write ---------------------------
    q.push_back(e);
    #1;
    q[0].delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(q[0][39:8] == 32'd7, "t3: queue element member write vanished (q[0].delay_counter != 7)")

    // c1: struct-variable member write -----------------------------------
    sv = e;
    #1;
    sv.delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(sv[39:8] == 32'd7, "c1: struct variable member write vanished (sv.delay_counter != 7)")

    // c2: fixed-array element member write -------------------------------
    fa[0] = e;
    #1;
    fa[0].delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(fa[0][39:8] == 32'd7, "c2: fixed array element member write vanished")

    // c3: dynamic-array element member write -----------------------------
    da = new[2];
    da[0] = e;
    #1;
    da[0].delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(da[0][39:8] == 32'd7, "c3: dynamic array element member write vanished")
    // c4: nested-struct chained member write (head [ov,inner]) -----------
    ov = '0;
    ov.inner = e;               // lands: base value 0 (no collision yet)
    #1;
    ov.inner.delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(ov[47:16] == 32'd7, "c4: nested struct chained member write vanished")

    // c5 (control): ungated-seeding probe --------------------------------
    // No cls_a declares `secret`; the write must land in the element.
    te.secret     = 8'h00;
    te.req_number = 8'd1;       // tiny_t value == 1
    tq.push_back(te);
    #1;
    tq[0].secret = 8'd42;
    #1;
    `SVTEST_CHECK(tq[0][15:8] == 8'd42, "c5 CONTROL: element write should land (no cls_a property named secret)")

    // c6: flat member read on colliding variable (read-side twin) ---------
    // The field bits must be zero for the whole value to stay 1, so the
    // expected field read is 0 while heap[1].delay_counter holds 9 (set
    // below via a legit class write). The read must return the field.
    sv = '0;
    sv.req_number = 8'd1;       // lands: sv still value 0 during the write
    h0.delay_counter = 32'd9;   // legit class write: heap[1].delay_counter = 9
    #1;
    `SVTEST_CHECK(sv.delay_counter == 32'd0, "c6: member read returned the heap object property (9) instead of the struct field (0)")

    // c7: associative-array element member write -------------------------
    e = '0;
    e.req_number = 8'd1;        // rebuild value 1/field 0 without a member
                                // write on the now-colliding shared local
    aa[8'h01] = e;
    #1;
    aa[8'h01].delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(aa[8'h01][39:8] == 32'd7, "c7: associative array element member write vanished")

    // c8 (control): chained access through a REAL class handle -----------
    // obj.pfield holds value 1 but obj is a genuine handle: the chain
    // must walk the property - not divert into heap[1].
    obj = new();                         // fresh real handle
    pre = h0.delay_counter;              // remember collision target
    e = '0;
    e.req_number = 8'd1;                 // rebuild value 1/field 0
    obj.pfield = e;                      // legit whole-property write
    #1;
    rd = obj.pfield.delay_counter;       // chained read: expect field 0
    `SVTEST_CHECK(rd == 32'd0, "c8 CONTROL: chained read must return pfield.delay_counter == 0 - not another object")
    obj.pfield.delay_counter = 32'd7;    // chained write
    #1;
    tmp = obj.pfield;                    // whole-property read
    `SVTEST_CHECK(tmp[39:8] == 32'd7, "c8 CONTROL: chained write must land in pfield.delay_counter")
    `SVTEST_CHECK(h0.delay_counter == pre, "c8 CONTROL: chained access must not pollute the colliding object")

    // c9: cross-module hierarchical member write (head [u,sv]) ------------
    #1;                                  // ensure sub's initial ran
    u.sv.delay_counter  = 32'd7;         // colliding base (value 1)
    u.sv2.delay_counter = 32'd3;         // control: non-colliding base
    #1;
    `SVTEST_CHECK(u.sv[39:8] == 32'd7, "c9: hierarchical member write vanished (u.sv.delay_counter != 7)")
    `SVTEST_CHECK(u.sv2[39:8] == 32'd3, "c9 CONTROL: non-colliding hierarchical write must land (u.sv2.delay_counter)")

    // c10: method-hijack member read (value 0x100 == handle 256) ---------
    me.tick       = 32'd1;      // mt_t value == 0x100 == 256
    me.req_number = 8'd0;
    msv = me;
    #1;
    `SVTEST_CHECK(msv.tick == 32'd1, "c10: member read hijacked as method call (cls_a::tick() returns 77)")

    //==== follow-up sweep (RCA section 9): f1-f14 ==========================
    #1;                          // boundary: settle before the sweep half
    // f3: NBA store path (queue element + flat variable) -------------------
    e = '0;
    e.req_number = 8'd1;       // rebuild value 1/field 0 (value 0 -> safe)
    q_nba.push_back(e);
    sv_nba = e;                // whole-assignment
    #1;
    q_nba[0].delay_counter <= 32'd7;
    sv_nba.delay_counter   <= 32'd7;
    #1;
    `SVTEST_CHECK(q_nba[0][39:8] == 32'd7, "f3: NBA queue element member write vanished (q[0].delay_counter != 7)")
    `SVTEST_CHECK(sv_nba[39:8] == 32'd7, "f3: NBA flat member write vanished (sv.delay_counter != 7)")

    // f4: foreach loop-variable head (colliding + control elements) --------
    e = '0;
    e.req_number = 8'd1;
    fq.push_back(e);           // fq[0] value 1 (colliding)
    fq.push_back('0);          // fq[1] value 0 (control)
    #1;
    foreach (fq[i])
      fq[i].delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(fq[0][39:8] == 32'd7, "f4: foreach colliding element member write vanished (fq[0].delay_counter != 7)")
    `SVTEST_CHECK(fq[1][39:8] == 32'd7, "f4 CONTROL: foreach non-colliding element write must land (fq[1].delay_counter)")

    // f5: ref-formal head ---------------------------------------------------
    e = '0;
    e.req_number = 8'd1;
    sv_ref = e;
    #1;
    poke_f(sv_ref);
    #1;
    `SVTEST_CHECK(sv_ref[39:8] == 32'd7, "f5: member write through ref formal vanished (sv.delay_counter != 7)")

    // f6: interface-instance hierarchical head ------------------------------
    e = '0;
    e.req_number = 8'd1;
    u_if.iv = e;               // whole hierarchical assignment
    #1;
    u_if.iv.delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(u_if.iv[39:8] == 32'd7, "f6: interface-instance member write vanished (u_if.iv.delay_counter != 7)")

    // f8: chained member READ (2-segment head, value-1 prefix) --------------
    e = '0;
    e.req_number = 8'd1;
    ov_r = '0;
    ov_r.inner = e;            // lands: base value 0 (no collision yet)
    h0.delay_counter = 32'd9;  // legit class write: arm the collision target
    #1;
    rd = ov_r.inner.delay_counter;
    `SVTEST_CHECK(rd == 32'd0, "f8: chained member read returned the heap object property (9) instead of the struct field (0)")

    // f9: hierarchical member READ (submodule) -------------------------------
    h0.delay_counter = 32'd9;  // re-arm
    #1;                        // sub_r's initial completed at t0
    rd = u2.sv_sub.delay_counter;
    `SVTEST_CHECK(rd == 32'd0, "f9: hierarchical member read returned the heap object property (9) instead of the struct field (0)")

    // f10: 3-segment chained member write -------------------------------------
    e = '0;
    e.req_number = 8'd1;
    oov = '0;
    oov.mid.inner = e;       // lands: head `oov.mid` value 0 at this point
    #1;
    oov.mid.inner.delay_counter = 32'd7;
    #1;
    `SVTEST_CHECK(oov[55:24] == 32'd7, "f10: 3-segment chained member write vanished (oov.mid.inner.delay_counter != 7)")

    // f11: 3-segment chained member READ --------------------------------------
    e = '0;
    e.req_number = 8'd1;
    oov_r = '0;
    oov_r.mid.inner = e;       // lands: head `oov_r.mid` value 0
    h0.delay_counter = 32'd9;  // re-arm
    #1;
    rd = oov_r.mid.inner.delay_counter;
    `SVTEST_CHECK(rd == 32'd0, "f11: 3-segment chained member read returned the heap object property (9) instead of the struct field (0)")

    // f13: compound RMW (read-modify-write) -----------------------------------
    e = '0;
    e.req_number = 8'd1;
    sv_rmw = e;
    h0.delay_counter = 32'd9;  // re-arm: the RMW read half must see 9
    #1;
    sv_rmw.delay_counter += 32'd1;
    #1;
    `SVTEST_CHECK(sv_rmw[39:8] == 32'd1, "f13: compound member assignment did not update the struct field (RMW diverted)")
    `SVTEST_CHECK(h0.delay_counter == 32'd9, "f13: compound member assignment corrupted the heap object (delay_counter 9 -> 10)")

    // f14: part-select member write + read -------------------------------------
    e = '0;
    e.req_number = 8'd1;
    qp.push_back(e);
    sv_ps = e;
    h0.delay_counter = 32'd9;  // re-arm
    #1;
    qp[0].delay_counter[3:0] = 4'hF;   // element bits [11:8]
    rd = sv_ps.delay_counter[3:0];
    #1;
    `SVTEST_CHECK(qp[0][11:8] == 4'hF, "f14: part-select member write vanished (qp[0].delay_counter[3:0] != 15)")
    `SVTEST_CHECK(rd == 4'd0, "f14: part-select member read returned the heap object property (15) instead of the struct field (0)")

    // f1 CONTROL: flat write, field name NOT a cls_a property -----------------
    tv_c = '0;
    tv_c.req_number = 8'd1;    // value 1 (write issued at value 0: safe)
    #1;
    tv_c.secret = 8'd42;       // `secret` not declared on cls_a -> must land
    #1;
    `SVTEST_CHECK(tv_c[15:8] == 8'd42, "f1 CONTROL: flat write with a non-class field name must land (tv.secret == 42)")

    // f2 CONTROL: flat read, field name NOT a cls_a property (value 0x101) -----
    tv_r = {8'd1, 8'd1};       // ONE whole-assign: secret=1, req_number=1 -> 257
    #1;
    rd = tv_r.secret;          // `secret` not declared on cls_a -> must land
    `SVTEST_CHECK(rd == 8'd1, "f2 CONTROL: flat read with a non-class field name must return the struct field (1)")

    // f7 CONTROL: collection-element member READ must not divert ---------------
    e = '0;
    e.req_number = 8'd1;
    eq7.push_back(e);
    h0.delay_counter = 32'd9;  // re-arm
    #1;
    rd = eq7[0].delay_counter;
    `SVTEST_CHECK(rd == 32'd0, "f7 CONTROL: collection-element member read must return the struct field (0), not the heap property")

    // f12 CONTROL: method-name member write must land ----------------------------
    me = '0;
    me.tick = 32'd1;           // value 0 -> 0x100 (write issued at value 0: safe)
    msv_c = me;                // value 0x100 == live handle 256
    msv_c2 = '0;               // control: value 0
    #1;
    msv_c.tick  = 32'd5;       // `tick` is a METHOD on cls_a, not a property
    msv_c2.tick = 32'd3;
    #1;
    `SVTEST_CHECK(msv_c[39:8] == 32'd5, "f12 CONTROL: method-name member write must land (msv.tick == 5)")
    `SVTEST_CHECK(msv_c2[39:8] == 32'd3, "f12 CONTROL: non-colliding method-name write must land (msv2.tick == 3)")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule