//! A DYNAMIC array written element-by-element inside a `for` loop must make
//! `always_comb` readers of that array re-evaluate.
//!
//! `for_body_is_simple`'s lvalue gate admitted any `Index{Ident, ..}` target
//! into the register-backed loop compilation, dynamic arrays included. The
//! compiled store LANDED the value — a direct read in the `initial` block
//! returned it — but skipped the coarse dirty-marking the AST interpreter
//! store does, so readers were never re-evaluated and silently kept stale or
//! `x` data:
//!
//! ```text
//! after loop write   : ra=x  a[0]=70    <- reader stale
//! after distinct wr  : ra=99 a[0]=99    <- a NON-loop write did notify
//! after 2nd loop wr  : ra=99 a[0]=80    <- loop write ignored again
//! ```
//!
//! Bounded precisely when found: it was the LOOP, not the variable index — a
//! constant index inside a loop failed too, while a variable index outside a
//! loop worked. Only dynamic arrays were affected; static unpacked arrays and
//! queues with the identical shape were already correct, and both are pinned
//! below so the fix cannot be "fixed" by dropping everything to the
//! interpreter.

use xezim::simulate;

/// Loop write, constant read: the reader must see the loop's values, and a
/// SECOND loop write must be observed too (the original bug let the first
/// non-loop write through and then went deaf again).
const DYN_LOOP_WRITE: &str = r#"
module tb;
  logic [7:0] a [];
  logic [7:0] ra;
  int ok;
  always_comb ra = a[0];
  initial begin
    a = new[4];
    for (int k = 0; k < 4; ++k) a[k] = 8'd70 + k[7:0];
    #1;
    ok = (ra == 8'd70) && (a[0] == 8'd70);
    for (int k = 0; k < 4; ++k) a[k] = 8'd80 + k[7:0];
    #1;
    ok = ok && (ra == 8'd80) && (a[3] == 8'd83);
  end
endmodule
"#;

/// The narrow discriminators from the hunt, kept as one case: a constant index
/// INSIDE a loop (which also failed), and a variable index OUTSIDE a loop
/// (which always worked).
const DYN_INDEX_FORMS: &str = r#"
module tb;
  logic [7:0] c [], e [];
  logic [7:0] rc, re;
  int vidx;
  int ok;
  always_comb rc = c[0];
  always_comb re = e[0];
  initial begin
    vidx = 0;
    c = new[4]; for (int k = 0; k < 1; ++k) c[0] = 8'd33;   // loop, CONST index
    e = new[4]; e[vidx] = 8'd44;                            // VAR index, no loop
    #1;
    ok = (rc == 8'd33) && (re == 8'd44);
  end
endmodule
"#;

/// Static unpacked arrays and queues take the same loop shape and were never
/// broken — they must stay correct (and are deliberately still compiled).
const STATIC_AND_QUEUE: &str = r#"
module tb;
  logic [7:0] s1 [7:0];
  logic [7:0] q1 [$];
  logic [7:0] r1, r2;
  int ok;
  always_comb r1 = s1[0];
  always_comb r2 = q1[0];
  initial begin
    for (int k = 0; k < 8; ++k) s1[k] = 8'd50 + k[7:0];
    q1.delete();
    for (int k = 0; k < 8; ++k) q1.push_back(8'd60 + k[7:0]);
    #1;
    ok = (r1 == 8'd50) && (r2 == 8'd60);
  end
endmodule
"#;

/// Round-4 siblings: the same dirty-marking path reached through a `foreach`,
/// a loop nested inside a task, a dynamic array OF STRUCTS, a 2-D dynamic
/// array, and readers that are NOT `always_comb` (a continuous assign and an
/// `always @(*)`). The blocking-loop and `always @(*)` cases were both broken
/// before the fix; the rest are pinned so the conservative guard cannot be
/// narrowed in a way that drops them.
const SIBLING_SHAPES: &str = r#"
typedef struct { logic [7:0] f; } st_t;
module tb;
  logic [7:0] d_fe [], d_task [], d_ca [], d_alw [];
  logic [7:0] d2 [][];
  st_t        d_st [];
  logic [7:0] r_fe, r_task, r_alw, r_2d, r_st;
  logic [7:0] r_ca;
  int ok;
  always_comb r_fe   = d_fe[0];
  always_comb r_task = d_task[0];
  assign      r_ca   = d_ca[0];        // continuous-assign reader
  always @(*) r_alw  = d_alw[0];       // always @(*) reader
  always_comb r_2d   = d2[0][0];
  always_comb r_st   = d_st[0].f;
  task automatic fill_it(); for (int k = 0; k < 4; ++k) d_task[k] = 8'd90 + k[7:0]; endtask
  task automatic init_2d();
    d2 = new[2];
    foreach (d2[i]) begin
      d2[i] = new[2];
      for (int j = 0; j < 2; ++j) d2[i][j] = 8'd180 + i[7:0]*2 + j[7:0];
    end
  endtask
  initial begin
    d_fe = new[4]; d_task = new[4]; d_ca = new[4]; d_alw = new[4]; d_st = new[4];
    init_2d();
    foreach (d_fe[k])     d_fe[k]   = 8'd100 + k[7:0];
    fill_it();
    for (int k = 0; k < 4; ++k) d_ca[k]  = 8'd110 + k[7:0];
    for (int k = 0; k < 4; ++k) d_alw[k] = 8'd130 + k[7:0];
    for (int k = 0; k < 4; ++k) d_st[k].f = 8'd140 + k[7:0];
    #1;
    ok = (r_fe == 8'd100) && (r_task == 8'd90) && (r_ca == 8'd110)
      && (r_alw == 8'd130) && (r_2d == 8'd180) && (r_st == 8'd140);
  end
endmodule
"#;

/// A NON-BLOCKING write to a dynamic-array or queue ELEMENT must notify
/// readers. (Audit rounds 4-6; FIXED.)
///
/// It was a different path from the loop bug above: it reproduced with no loop
/// at all.
/// This is a DIFFERENT path from the loop bug fixed above: it reproduces with
/// no loop at all, so the compile-gate guard does not reach it, and the
/// interpreter's own NBA-to-collection commit is what fails to mark dirty.
/// The value lands (a direct read returns it) and a later BLOCKING write does
/// notify, so a reader silently serves stale data indefinitely:
///
/// ```text
/// A after NBA : direct n1[0]=11 comb r1=x   <- never notified
/// B after blk : direct n1[0]=55 comb r1=55  <- blocking DOES notify
/// C after NBA2: direct n1[0]=77 comb r1=55  <- stuck on the stale value
/// ```
///
/// A dynamic-array / queue element carries a signal TWIN, so the NBA fast
/// path resolved it and `apply_nba_entry` wrote (and dirtied) THAT id — but a
/// reader of the COLLECTION does not depend on it, so nothing re-evaluated.
/// The value landed while readers served stale data, and a later BLOCKING
/// write did notify, which made it look intermittent.
///
/// Fixed by returning None from `resolve_nba_target` for these two kinds, so
/// they commit through the queued-lvalue path (`assign_value`) — the same
/// route ASSOCIATIVE arrays already took, which is why assoc was always
/// correct. Edge blocks needed the matching compiler-side gate
/// (`collection_store_denied`), since they emit array-store insns directly.
///
/// Assoc and static are kept in the same case as positive controls.
const NBA_TO_DYNAMIC_ELEMENT: &str = r#"
module tb;
  logic [7:0] n1 [];
  logic [7:0] q1 [$];
  logic [7:0] a1 [int];
  logic [7:0] s1 [3:0];
  logic [7:0] r1, r2, r3, r4;
  int ok;
  always_comb r1 = n1[0];
  always_comb r2 = q1[0];
  always_comb r3 = a1[0];
  always_comb r4 = s1[0];
  initial begin
    n1 = new[4];
    q1.delete(); for (int z = 0; z < 4; ++z) q1.push_back(8'd0);
    for (int z = 0; z < 4; ++z) a1[z] = 8'd0;
    n1[0] <= 8'd11;
    q1[0] <= 8'd22;
    a1[0] <= 8'd33;   // associative: already correct
    s1[0] <= 8'd44;   // static: already correct
    #1;
    ok = (r1 == 8'd11) && (r2 == 8'd22) && (r3 == 8'd33) && (r4 == 8'd44);
  end
endmodule
"#;

fn ok_flag(src: &str) -> u64 {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.get_signal("ok")
        .or_else(|| sim.get_signal("tb.ok"))
        .expect("signal 'ok' not found")
        .to_u64()
        .unwrap_or(0)
}

#[test]
fn dynamic_array_loop_write_reaches_comb_readers() {
    assert_eq!(
        ok_flag(DYN_LOOP_WRITE),
        1,
        "an always_comb reader did not see a dynamic array written in a for loop"
    );
}

#[test]
fn dynamic_array_const_index_in_loop_and_var_index_outside_both_notify() {
    assert_eq!(ok_flag(DYN_INDEX_FORMS), 1, "a dynamic array index form lost its notification");
}

#[test]
fn static_array_and_queue_loop_writes_still_notify() {
    assert_eq!(ok_flag(STATIC_AND_QUEUE), 1, "a static array or queue loop write regressed");
}

#[test]
fn dynamic_array_sibling_shapes_and_non_comb_readers_notify() {
    assert_eq!(
        ok_flag(SIBLING_SHAPES),
        1,
        "a foreach/task/struct/2-D dynamic array write, or a non-always_comb reader, lost its notification"
    );
}

#[test]
fn nba_to_dynamic_array_or_queue_element_notifies_readers() {
    assert_eq!(
        ok_flag(NBA_TO_DYNAMIC_ELEMENT),
        1,
        "a non-blocking write to a dynamic array or queue element did not reach its reader"
    );
}
