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
  cls_a h0, obj;
  req_t q[$], fa[2], sv, sv_ref;
  outer_t ov;
  int rd;

  task automatic poke(ref req_t r);
    r.delay_counter = 32'd7;   // member write on a ref formal
  endtask

  initial begin
    cls_a c;
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
        ]
    );
}