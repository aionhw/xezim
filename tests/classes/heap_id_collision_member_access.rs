//! id-collision-in-heap: the member-write fall-throughs in `assign_value`
//! (the W-indexed heap divert, the part-select member base, and the
//! MemberAccess tail divert) trusted the receiver's INTEGRAL VALUE as a heap
//! handle. A packed struct splices its members into one integral value
//! (IEEE 1800-2017 §7.8), so `sv.delay_counter = v` with `sv == 32'd1`
//! landed in `heap[1].delay_counter`, stealing the write from the struct and
//! clobbering the live object; the same misroute hit queue/fixed-array
//! elements (`q[0].delay_counter = v` — the element-class map also records
//! struct typedefs, which are not classes), chained nested heads, `ref`
//! formals, and part-selected element fields. The diverts are now gated on
//! the receiver statically denoting a class-object lvalue, and `r.f = v` on
//! a `ref` formal redirects to the caller's actual (§13.5.2), so every
//! struct-field write below lands in the struct — while genuine class
//! property writes through real handles keep working.
//!
//! Review regressions: (1) a module-scope unpacked struct with a
//! class-typed member (`su_t gx; gx.h = new; gx.h.v = …`) — the root sits
//! in no runtime class map, so the gates misclassified `gx.h` and the
//! handle write/read through it broke; `dotted_name_declared_class` now
//! starts its walk from the module-scope `var_decl_types` declaration.
//! (2) typedef-aliased class handles — a plain alias (`typedef cfg_c
//! alias_t;`) and a parameterized one (`typedef pc#(16) pc16_t;`) — which
//! `bare_name_is_class_channel` now resolves through
//! `resolve_typeref_class_name_str`.

use xezim::simulate;

const SRC: &str = r#"
package pk;
  typedef struct packed {
    logic [255:0] req_type;
    logic [31:0]  delay_counter;
    logic [7:0]   req_number;
  } req_t;
  typedef struct packed {
    req_t       inner;
    logic [7:0] pad;
  } outer_t;
endpackage

import pk::*;

class cls_a;
  int delay_counter;
endclass

module top;
  import pk::*;

  // Review regression shapes: declared at MODULE scope (the scope the
  // review regressions were reported against), like
  // `struct_member_class_handle_new`'s `S gx;`.
  class inner_c;
    int v = 32'd7;
  endclass

  class cfg_c;
    int f;
    function int get_f();
      return f;
    endfunction
  endclass

  class pc #(parameter int W = 8);
    int v;
    function int get_v();
      return v;
    endfunction
  endclass

  typedef cfg_c alias_t;
  typedef pc#(16) pc16_t;

  typedef struct {
    inner_c h;
    int pad;
  } su_t;

  cls_a h0, obj;
  req_t q[$], fa[2], sv, sv_ref;
  outer_t ov;
  int rd;
  su_t gx;
  alias_t al;
  pc16_t pp;

  task automatic poke(ref req_t r);
    r.delay_counter = 32'd7;   // member write on a ref formal
  endtask

  initial begin
    cls_a c;
    alias_t la;
    req_t e;
    h0 = new;                  // handle 1: prime the collision
    c  = new;                  // handle 2
    e = '0;
    e.req_number = 8'd1;       // struct value == 1 == a live handle

    q.push_back(e);
    q[0].delay_counter = 32'd7;      // queue element (struct typedef, not a class)
    $display("NOTE: q=%0d", q[0][39:8]);

    fa[0] = e;
    fa[0].delay_counter = 32'd7;     // fixed-array element
    $display("NOTE: fa=%0d", fa[0][39:8]);

    sv = e;
    sv.delay_counter = 32'd7;        // flat struct variable
    $display("NOTE: sv=%0d", sv[39:8]);

    ov = '0;
    ov.inner = e;                    // head value 0 during chain seeding
    ov.inner.delay_counter = 32'd7;  // chained nested head
    $display("NOTE: ov=%0d", ov[47:16]);

    sv_ref = e;
    poke(sv_ref);                    // member write through a ref formal
    $display("NOTE: ref=%0d", sv_ref[39:8]);

    q[0].delay_counter[3:0] = 4'hF;  // part-select on a queue element
    $display("NOTE: ps=%0d", q[0][11:8]);

    rd = q[0].delay_counter;         // element member READ (colliding value)
    $display("NOTE: rd=%0d", rd);

    // The struct writes above must not have touched the live object.
    $display("NOTE: h0=%0d", h0.delay_counter);

    // Controls: genuine class property writes keep working.
    h0.delay_counter = 32'd9;
    $display("NOTE: h0w=%0d", h0.delay_counter);
    obj = new;
    obj.delay_counter = 32'd33;
    $display("NOTE: obj=%0d", obj.delay_counter);

    // Review regression 1: module-scope unpacked struct with a
    // class-typed member — `new` must construct the member's class (the
    // initializer read proves it), then read/write through the handle.
    gx.h = new;
    $display("NOTE: gxc=%0d", gx.h.v);
    gx.h.v = 32'd5;
    $display("NOTE: gxv=%0d", gx.h.v);
    gx.pad = 32'd2;          // 2 == a live handle index: must stay in the struct
    $display("NOTE: gxp=%0d", gx.pad);
    $display("NOTE: gxn=%0d", c.delay_counter);

    // Review regression 2: typedef-aliased class handles — write, read,
    // method call; plain and parameterized aliases, module-scope and local.
    al = new;
    al.f = 32'd9;
    $display("NOTE: alw=%0d", al.f);
    $display("NOTE: alm=%0d", al.get_f());
    pp = new;
    pp.v = 32'd21;
    $display("NOTE: ppw=%0d", pp.v);
    $display("NOTE: ppm=%0d", pp.get_v());
    la = new;
    la.f = 32'd4;
    $display("NOTE: law=%0d", la.f);
    $display("NOTE: lam=%0d", la.get_f());
  end
endmodule
"#;

#[test]
fn struct_member_writes_survive_heap_id_collision() {
    let sim = simulate(SRC, 1_000_000).expect("simulate failed");
    let notes: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("NOTE:"))
        .collect();
    assert_eq!(
        notes,
        [
            "NOTE: q=7",
            "NOTE: fa=7",
            "NOTE: sv=7",
            "NOTE: ov=7",
            "NOTE: ref=7",
            "NOTE: ps=15",
            "NOTE: rd=15",
            "NOTE: h0=0",
            "NOTE: h0w=9",
            "NOTE: obj=33",
            "NOTE: gxc=7",
            "NOTE: gxv=5",
            "NOTE: gxp=2",
            "NOTE: gxn=0",
            "NOTE: alw=9",
            "NOTE: alm=9",
            "NOTE: ppw=21",
            "NOTE: ppm=21",
            "NOTE: law=4",
            "NOTE: lam=4",
        ]
    );
}