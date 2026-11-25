//! IEEE 1800-2023 §6.17 / §15.5 Named event semantics tests (Milestone 4).
//!
//! Covers both AST interpretation and compiled bytecode contexts across:
//! - Transitive handle aliasing (`a = b; b = c;`), self/null re-binding, event arrays
//! - Identity comparisons (`==`, `!=`, `===`, `!==`) including `null`
//! - Boolean conversion (`if (e)`, `e ? 1 : 0`, `!e`, `e && 1`, `e || 0`, `while (e)`)
//!   independent of trigger toggle count
//! - Passing `event` by value, `output`, `inout`, `ref`, and returning `event`
//! - Procedural-local `event` initializers and `automatic` re-entry resets
//! - Class-member `event` `null` default/assignment, cross-object/cross-kind aliasing,
//!   class event arrays, and method formal shadowing
//! - `typedef event` (scalar, chained, module, child instance, local, class, formal, return)

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 500).unwrap_or_else(|e| panic!("simulation failed: {e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn event_aliasing_identity_case_eq_and_arrays() {
    let src = r#"
module top;
  event a, b, c, d;
  event arr[3];
  int hits_a = 0;
  int hits_b = 0;
  int hits_c = 0;
  int hits_arr1 = 0;

  initial begin
    $display("INIT:%0d:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
             (a == a), (a === a), (a == b), (a === b), (a !== b),
             (a == null), (a === null), (a !== null));

    a = b;
    b = c;
    $display("TRANS:%0d:%0d:%0d:%0d:%0d:%0d:%0d:%0d",
             (a == b), (a == c), (b == c),
             (a === b), (a === c), (b === c),
             (a !== b), (b !== c));

    fork
      forever begin @a; hits_a++; end
      forever begin @b; hits_b++; end
      forever begin @c; hits_c++; end
      forever begin @(arr[1]); hits_arr1++; end
    join_none

    #1;
    -> b;
    $display("TRIG_B:%0d:%0d:%0d", a.triggered, b.triggered, c.triggered);
    #1;
    $display("HITS_B:%0d:%0d:%0d", hits_a, hits_b, hits_c);

    -> a;
    $display("TRIG_A:%0d:%0d:%0d", a.triggered, b.triggered, c.triggered);
    #1;
    $display("HITS_A:%0d:%0d:%0d", hits_a, hits_b, hits_c);

    d = null;
    -> d;
    $display("NULL_D:%0d:%0d:%0d:%0d", (d == null), (d === null), (d !== null), d.triggered);
    d = b;
    $display("REBIND_D:%0d:%0d:%0d", (d == null), (d == c), (d === b));

    arr[0] = arr[1];
    -> arr[0];
    $display("ARR:%0d:%0d:%0d:%0d", (arr[0] == arr[1]), (arr[0] === arr[1]),
             (arr[0] == arr[2]), arr[1].triggered);
    #1;
    $display("ARR_HITS:%0d", hits_arr1);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(
        lines,
        vec![
            "INIT:1:1:0:0:1:0:0:1",
            "TRANS:0:0:1:0:0:1:1:0",
            "TRIG_B:0:1:1",
            "HITS_B:0:1:1",
            "TRIG_A:1:0:0",
            "HITS_A:1:1:1",
            "NULL_D:1:1:0:0",
            "REBIND_D:0:1:1",
            "ARR:1:1:0:1",
            "ARR_HITS:1",
        ]
    );
}

#[test]
fn event_boolean_conversion_ignores_toggle_and_handles_null() {
    let src = r#"
class Runner;
  function int check_bool(event e, int do_trig);
    int b_if, b_tern, b_not, b_and, b_or;
    if (do_trig) -> e;
    b_if = 0;
    if (e) b_if = 1;
    b_tern = e ? 1 : 0;
    b_not = !e;
    b_and = (e && 1'b1);
    b_or = (e || 1'b0);
    return 100000 + b_if * 10000 + b_tern * 1000 + b_not * 100 + b_and * 10 + b_or;
  endfunction
endclass

module top;
  event e, en;
  initial begin
    automatic Runner r = new();
    int loop_cnt = 0;
    int null_cnt = 0;
    int i;

    for (i = 0; i < 4; i++) begin
      int bf = 0;
      if (i > 0) -> e;
      if (e) bf = 1;
      $display("ITER%0d:%0d:%0d:%0d:%0d:%0d:%0d",
               i, bf, (e ? 1 : 0), !e, (e && 1'b1), (e || 1'b0),
               r.check_bool(e, 1));
    end

    en = null;
    $display("NULL:%0d:%0d:%0d:%0d:%0d:%0d",
             (en ? 1 : 0), !en, (en && 1'b1), (en || 1'b0),
             (en == null), r.check_bool(en, 0));

    while (e) begin
      loop_cnt++;
      e = null;
    end
    while (en) begin
      null_cnt++;
    end
    $display("WHILE:%0d:%0d:%0d", loop_cnt, null_cnt, (e == null));
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(
        lines,
        vec![
            "ITER0:1:1:0:1:1:111011",
            "ITER1:1:1:0:1:1:111011",
            "ITER2:1:1:0:1:1:111011",
            "ITER3:1:1:0:1:1:111011",
            "NULL:0:1:0:0:1:100100",
            "WHILE:1:0:1",
        ]
    );
}

#[test]
fn event_subroutine_formals_and_return_values() {
    let src = r#"
class Helper;
  event cls_ev;
  task automatic ping_val(event ev);
    -> ev;
  endtask
  task automatic clobber_val(event ev, event other);
    ev = other;
    ev = null;
  endtask
  function automatic event echo_ev(event ev);
    return ev;
  endfunction
  task automatic rebind_ref(ref event ev, event other);
    ev = other;
    -> ev;
  endtask
endclass

module top;
  event e1, e2, e3, res;
  int w1 = 0;
  int w2 = 0;

  task automatic trigger_val(event ev);
    -> ev;
  endtask

  task automatic rebind_val(event ev, event other);
    ev = other;
    ev = null;
  endtask

  function automatic bit same_ev(event a, event b);
    return (a == b) && (a === b);
  endfunction

  task automatic set_out(output event dst, input event src);
    dst = src;
  endtask

  task automatic swap_inout(inout event x, inout event y);
    automatic event t = x;
    x = y;
    y = t;
  endtask

  task automatic trig_ref(ref event ev);
    -> ev;
  endtask

  task automatic same_name_ref(ref event e1, input event dst);
    e1 = dst;
    -> e1;
  endtask

  function automatic event passthrough(event ev);
    return ev;
  endfunction

  initial begin
    automatic Helper h = new();
    fork
      forever begin @e1; w1++; end
      forever begin @e2; w2++; end
    join_none

    #1;
    trigger_val(e1);
    $display("VAL_TRIG:%0d", e1.triggered);
    #1;
    rebind_val(e1, e2);
    $display("VAL_REBIND:%0d:%0d:%0d", w1, (e1 == null), (e1 == e2));

    $display("SAME:%0d:%0d:%0d", same_ev(e1, e1), same_ev(e1, e2), same_ev(null, null));

    set_out(res, e2);
    -> res;
    $display("OUT:%0d:%0d:%0d", (res == e2), (res === e2), e2.triggered);
    #1;

    swap_inout(e1, e2);
    $display("SWAP:%0d:%0d:%0d", same_ev(e1, res), same_ev(e1, e2), same_ev(e2, res));
    #1;

    trig_ref(e1);
    #1;
    $display("REF_TRIG:%0d:%0d", w1, w2);

    res = passthrough(e3);
    -> res;
    $display("RET_MOD:%0d:%0d", (res == e3), e3.triggered);
    #1;

    h.ping_val(e3);
    $display("METH_VAL:%0d", e3.triggered);
    #1;
    h.clobber_val(e3, e2);
    res = h.echo_ev(e3);
    $display("METH_RET:%0d:%0d", (e3 == null), (res == e3));

    h.rebind_ref(h.cls_ev, e3);
    $display("METH_REF:%0d:%0d", (h.cls_ev == e3), e3.triggered);
    #1;

    same_name_ref(e1, e3);
    $display("SAME_NAME_REF:%0d:%0d", (e1 === e3), e3.triggered);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(
        lines,
        vec![
            "VAL_TRIG:1",
            "VAL_REBIND:1:0:0",
            "SAME:1:0:1",
            "OUT:1:1:1",
            "SWAP:1:0:0",
            "REF_TRIG:1:2",
            "RET_MOD:1:1",
            "METH_VAL:1",
            "METH_RET:0:1",
            "METH_REF:1:1",
            "SAME_NAME_REF:1:1",
        ]
    );
}

#[test]
fn event_local_initializers_and_automatic_reentry() {
    let src = r#"
module top;
  event ext;
  event saved_fresh;
  int ext_hits = 0;

  task automatic probe(input int iter, input event ext_e, inout event prev_fresh);
    automatic event le = ext_e;
    automatic event lnull = null;
    automatic event lfresh;
    int trig_ok;
    int fresh_distinct;

    if (iter == 0) begin
      saved_fresh = lfresh;
      prev_fresh = lfresh;
    end
    fresh_distinct = (iter == 0) ? 1 : ((lfresh != prev_fresh) && (lfresh !== saved_fresh));

    -> lfresh;
    trig_ok = lfresh.triggered;
    -> le;

    $display("PROBE%0d:%0d:%0d:%0d:%0d:%0d:%0d",
             iter, (le == ext_e), (lnull == null), (!lnull),
             (lfresh ? 1 : 0), trig_ok, fresh_distinct);

    // Mutate lnull on first activation to verify next activation resets to null.
    lnull = lfresh;
  endtask

  initial begin
    automatic event dummy;
    automatic int i;
    fork
      forever begin @ext; ext_hits++; end
    join_none

    #1;
    probe(0, ext, dummy);
    #1;
    probe(1, ext, dummy);
    #1;
    $display("EXT_HITS:%0d", ext_hits);

    for (i = 0; i < 2; i++) begin
      automatic event blk_ev = null;
      $display("BLK%0d:%0d:%0d", i, (blk_ev == null), !blk_ev);
      blk_ev = ext;
    end
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(
        lines,
        vec![
            "PROBE0:1:1:1:1:1:1",
            "PROBE1:1:1:1:1:1:1",
            "EXT_HITS:2",
            "BLK0:1:1",
            "BLK1:1:1",
        ]
    );
}

#[test]
fn event_class_members_null_alias_cross_kind_and_shadowing() {
    let src = r#"
class Box;
  event ev = null;
  event ev2;
  event evs[2];
  int cls_hits = 0;

  task automatic wait_ev2();
    @(ev2);
    cls_hits++;
  endtask

  task automatic shadow_ping(event ev);
    -> ev;
  endtask
endclass

module top;
  event mod_e, mod_e2;
  int mod_hits = 0;
  int box_hits = 0;

  initial begin
    automatic Box b1 = new();
    automatic Box b2 = new();

    $display("C_INIT:%0d:%0d:%0d:%0d",
             (b1.ev == null), (b1.ev === null), (b1.ev ? 1 : 0),
             (b2.ev2 == null));
    -> b1.ev;
    -> b2.ev2;
    $display("C_TRIG:%0d:%0d", b1.ev.triggered, b2.ev2.triggered);
    #1;

    b1.ev2 = null;
    -> b1.ev2;
    $display("C_SET_NULL:%0d:%0d:%0d", (b1.ev2 == null), (b1.ev2 ? 1 : 0), b1.ev2.triggered);
    #1;

    b1.ev = b2.ev2;
    -> b1.ev;
    $display("C_ALIAS:%0d:%0d:%0d", (b1.ev == b2.ev2), (b1.ev === b2.ev2), b2.ev2.triggered);
    #1;

    fork
      forever begin @mod_e; mod_hits++; end
      forever begin @(b1.ev); box_hits++; end
    join_none

    // Cross-kind: class member aliased to module event.
    b1.ev = mod_e;
    #1;
    -> b1.ev;
    $display("XK_C2M_TRIG:%0d:%0d", mod_e.triggered, (b1.ev == mod_e));
    #1;
    -> mod_e;
    $display("XK_M2C_TRIG:%0d:%0d", mod_e.triggered, b1.ev.triggered);
    #1;
    $display("XK_HITS:%0d:%0d", mod_hits, box_hits);

    // Reverse cross-kind: module event aliased to class member event.
    mod_e2 = b2.ev2;
    -> b2.ev2;
    $display("XK_REV:%0d:%0d:%0d", (mod_e2 == b2.ev2), mod_e2.triggered, b2.ev2.triggered);
    #1;

    // Class event array element aliasing and independent nulling.
    b1.evs[0] = b1.evs[1];
    -> b1.evs[0];
    $display("C_ARR1:%0d:%0d", (b1.evs[0] == b1.evs[1]), b1.evs[1].triggered);
    #1;
    b1.evs[1] = null;
    -> b1.evs[1];
    $display("C_ARR2:%0d:%0d:%0d", (b1.evs[1] == null), b1.evs[1].triggered,
             (b1.evs[0] == null));
    -> b1.evs[0];
    $display("C_ARR3:%0d", b1.evs[0].triggered);
    #1;

    // Method formal named `ev` shadows property `ev`.
    b2.ev = b2.ev2;
    b2.shadow_ping(b1.evs[0]);
    $display("SHADOW:%0d:%0d", b1.evs[0].triggered, b2.ev.triggered);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(
        lines,
        vec![
            "C_INIT:1:1:0:0",
            "C_TRIG:0:1",
            "C_SET_NULL:1:0:0",
            "C_ALIAS:1:1:1",
            "XK_C2M_TRIG:1:1",
            "XK_M2C_TRIG:1:1",
            "XK_HITS:2:2",
            "XK_REV:1:1:1",
            "C_ARR1:1:1",
            "C_ARR2:1:0:0",
            "C_ARR3:1",
            "SHADOW:1:0",
        ]
    );
}

#[test]
fn event_typedef_across_scopes_and_classes() {
    let src = r#"
package P;
  typedef event ev_t;
  typedef ev_t ev2_t;
endpackage

module sub;
  import P::*;
  ev_t sub_e;
  ev2_t sub_e2;
  int woken = 0;
  initial begin
    forever begin @sub_e; woken++; end
  end
endmodule

class Wrap;
  import P::*;
  typedef event cls_ev_t;
  ev_t ce = null;
  cls_ev_t ce2;

  task automatic ping(ev2_t fe);
    -> fe;
  endtask
endclass

module top;
  import P::*;
  sub u();
  ev_t m1, m2;
  event plain_e;

  function automatic ev_t pass_ev(ev2_t x);
    return x;
  endfunction

  initial begin
    automatic Wrap w = new();
    automatic ev_t loc_null = null;
    automatic ev2_t loc_alias = m1;
    automatic ev_t ret;

    $display("TD_INIT:%0d:%0d:%0d:%0d:%0d",
             (m1 == null), (loc_null == null), (w.ce == null),
             (w.ce2 == null), (loc_alias == m1));

    -> w.ce;
    -> w.ce2;
    $display("TD_CLS:%0d:%0d", w.ce.triggered, w.ce2.triggered);
    #1;

    m2 = m1;
    -> m2;
    $display("TD_MOD:%0d:%0d:%0d", (m1 == m2), m1.triggered, loc_alias.triggered);
    #1;

    ret = pass_ev(plain_e);
    $display("TD_PLAIN:%0d", (ret === plain_e));
    ret = pass_ev(u.sub_e);
    w.ping(ret);
    $display("TD_SUB:%0d:%0d", (ret == u.sub_e), u.sub_e.triggered);
    #1;
    $display("TD_WOKEN:%0d", u.woken);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(
        lines,
        vec![
            "TD_INIT:0:1:1:0:1",
            "TD_CLS:0:1",
            "TD_MOD:1:1:1",
            "TD_PLAIN:1",
            "TD_SUB:1:1",
            "TD_WOKEN:1",
        ]
    );
}

#[test]
fn event_gc_transient_objects_stress() {
    let src = r#"
class Ephemeral;
  event ev;
  event nil_ev = null;
endclass

module top;
  int ok = 0;
  initial begin
    automatic int i;
    for (i = 0; i < 600; i++) begin
      automatic Ephemeral e = new();
      -> e.ev;
      if (e.ev && e.ev.triggered && (e.nil_ev == null) && !e.nil_ev) begin
        ok++;
      end
      if ((i % 200) == 0) #1;
    end
    $display("GC_OK:%0d", ok);
  end
endmodule
"#;
    let lines = sim_lines(src);
    assert_eq!(lines, vec!["GC_OK:600"]);
}
