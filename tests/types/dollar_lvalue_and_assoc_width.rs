//! Two gaps found by auditing the batch 14-16 fixes. Both predate that work —
//! each was proven pre-existing before being fixed — and both are now
//! reference-validated.
//!
//! 1. §11.4.12 — `$` in a QUEUE index is the queue's last valid index. The READ
//!    paths push `dollar_bound` before evaluating the index; no LVALUE path
//!    did, so `ExprKind::Dollar` fell through to its `u64::MAX` default and
//!    `q[$] = v` wrote element 18446744073709551615: the write silently
//!    vanished. The index is now
//!    normalised once, with the bound installed, over the WHOLE lvalue at the
//!    top of `assign_value` — plus in `resolve_nba_target` and
//!    `freeze_lvalue_indices`, which each evaluate it on their own path.
//!    Normalising the whole lvalue rather than one arm is what makes
//!    `q[$][3:0]` (top node is a RangeSelect) and `mem[q[$]]` (the `$` belongs
//!    to the inner collection) come out right; each index is resolved against
//!    its own base.
//!
//! 2. §10.7 — an associative-array element has no entry in the typed signal
//!    table, and the elaborator recorded only whether the array was
//!    string-keyed, never its ELEMENT WIDTH. So a write stored the RHS at its
//!    own size: `logic [3:0] aa[string]; aa["k"] = 8'hEF` kept all eight bits
//!    and `$bits` reported 8. `ElaboratedModule::assoc_elem_widths` now carries
//!    the declared width and the store fits to it — registered under the
//!    instance-scoped name for submodules. The lookup is deliberately
//!    conservative: a class member (`<handle>#m`) is never resolved through a
//!    bare leaf, because guessing a too-narrow width silently CORRUPTS data
//!    whereas guessing nothing merely restores the older store-as-is.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// A write through `$` follows the current queue size; compare with a fixed index.
#[test]
fn dollar_index_lvalue_targets_the_last_element() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int qa [$]; int qb [$]; int qc [$];
  int r_blk, r_long, r_const, sz;
  initial begin
    qa.push_back(10); qa.push_back(20);
    qb.push_back(10); qb.push_back(20); qb.push_back(30);
    qc.push_back(10); qc.push_back(20);
    @(posedge clk);
    qa[$] = 77;       // blocking
    qb[$] = 88;      // last element
    qc[1] = 99;      // control: constant index
    @(posedge clk); #1;
    r_blk = qa[1]; r_long = qb[2]; r_const = qc[1]; sz = qa.size();
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "r_blk"),
        77,
        "blocking q[$] must hit the last element"
    );
    assert_eq!(u(&sim, "r_long"), 88, "q[$] must follow the longer queue");
    assert_eq!(u(&sim, "r_const"), 99, "constant index unaffected");
    assert_eq!(u(&sim, "sz"), 2, "writing q[$] must not grow the queue");
}

/// A `$`-relative expression (`q[$-1]`) resolves against the same bound.
#[test]
fn dollar_relative_index_lvalue() {
    let src = r#"
`timescale 1ns/1ns
module top;
  int q [$];
  int first, last;
  initial begin
    q.push_back(10); q.push_back(20); q.push_back(30);
    q[$-2] = 55;      // first element
    q[$]   = 66;      // last element
    #1 first = q[0]; last = q[2];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "first"), 55, "q[$-2] is the first of three");
    assert_eq!(u(&sim, "last"), 66, "q[$] is the last");
}

/// §7.10/§11.4.1: capture a mutating selector in the queue's `$` context,
/// then retain the aggregate copy and evaluate the function only once.
#[test]
fn captured_queue_tail_selector_keeps_its_bound() {
    let src = r#"
module top;
  typedef struct { int tag; string text; } record_t;
  record_t records[$], source;
  int words[$], calls = 0, first, last, copied, record_calls;
  string copied_text;
  function automatic int distance(); calls++; return 1; endfunction
  initial begin
    words = '{10, 20, 30};
    words[$ - distance()] = 77;
    first = words[1];
    last = words[2];
    source = '{11, "before"};
    records.push_back(source);
    records.push_back(source);
    source = '{73, "after"};
    records[$ - distance()] = source;
    copied = records[0].tag;
    copied_text = records[0].text;
    record_calls = calls;
    $finish;
  end
endmodule"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "first"), 77);
    assert_eq!(u(&sim, "last"), 30);
    assert_eq!(u(&sim, "copied"), 73);
    assert_eq!(u(&sim, "record_calls"), 2);
    assert_eq!(
        sim.get_signal("copied_text").unwrap().to_sv_string(),
        "after"
    );
}

/// Assoc elements take their DECLARED width — narrow truncates, wide extends —
/// for both literal and variable keys.
#[test]
fn associative_element_writes_fit_the_declared_width() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] narrow_blk [string];
  string key = "k";
  logic [3:0] narrow_var [string];
  int         wide_blk   [string];
  int         wide_var   [string];
  int nb, nn, wb, wn, bits_n;
  initial begin
    @(posedge clk);
    narrow_blk["k"]  = 8'hEF;
    narrow_var[key] = 8'hEF;
    wide_blk["k"]    = 8'hEF;
    wide_var[key]   = 8'hEF;
    @(posedge clk); #1;
    nb = narrow_blk["k"]; nn = narrow_var["k"];
    wb = wide_blk["k"];   wn = wide_var["k"];
    bits_n = $bits(narrow_blk["k"]);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "nb"), 0xF, "4-bit element truncates 8'hEF");
    assert_eq!(u(&sim, "nn"), 0xF, "4-bit element with a variable key");
    assert_eq!(u(&sim, "wb"), 0xEF, "an int element keeps the value");
    assert_eq!(u(&sim, "wn"), 0xEF, "int element with a variable key");
    assert_eq!(u(&sim, "bits_n"), 4, "$bits reports the DECLARED width");
}

/// Queues and dynamic arrays keep fitting to their element width — the assoc
/// change must not have moved them onto the untyped path.
#[test]
fn queue_and_dynamic_elements_still_fit() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] q [$];
  logic [3:0] dyn [];
  int rq, rd;
  initial begin
    q.push_back(4'h0); q.push_back(4'h0);
    dyn = new[2];
    @(posedge clk);
    q[1]   = 8'hAB;
    dyn[1] = 8'hCD;
    @(posedge clk); #1;
    rq = q[1]; rd = dyn[1];
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(u(&sim, "rq"), 0xB, "queue element truncates");
    assert_eq!(u(&sim, "rd"), 0xD, "dynamic-array element truncates");
}

/// Sibling shapes of the `$` fix, all reference-validated:
///
///  * `q[$][3:0]` — a part-select whose BASE carries the `$`. The lvalue's top
///    node is a RangeSelect, so an Index-arm-local fix would have missed it;
///    the normalisation runs once over the whole lvalue instead.
///  * `mem[q[$]]` — the `$` belongs to the INNER collection. Each index is
///    resolved against its own base, so this must not pick up `mem`'s bound.
#[test]
fn dollar_in_nested_and_range_select_lvalues() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic [7:0] q [$];
  int qmem [0:7];
  int last, first, sz, hit, untouched;
  initial begin
    q.push_back(8'h11); q.push_back(8'h22); q.push_back(8'h33);
    q[$][3:0] = 4'hF;          // part-select of the last element -> 8'h3F

    for (int i = 0; i < 8; i++) qmem[i] = 0;
    qmem[q[$]] = 0;            // q[$] is 8'h3f -> out of range, must not crash
    q.delete();
    q.push_back(8'h03); q.push_back(8'h05);
    qmem[q[$]] = 99;           // q[$] == 5 -> qmem[5], NOT qmem[7]

    #1;
    last = q[1]; first = q[0]; sz = q.size();
    hit = qmem[5]; untouched = qmem[7];
  end
endmodule
"#;
    let sim = simulate(src, 40).expect("simulate failed");
    assert_eq!(u(&sim, "first"), 0x03, "untouched element");
    assert_eq!(u(&sim, "sz"), 2, "queue size unchanged by the writes");
    assert_eq!(
        u(&sim, "hit"),
        99,
        "inner `$` resolves against the QUEUE, not the array"
    );
    assert_eq!(
        u(&sim, "untouched"),
        0,
        "the outer array's own bound was not used"
    );
}

/// An assoc array declared in a CLASS must not take its width from a
/// same-named module-scope array. The lookup is keyed by declaration name, so
/// a bare-leaf fallback would silently truncate an 8-bit class member to the
/// module array's 4 bits — corrupting data rather than merely missing a fit.
#[test]
fn class_member_assoc_does_not_inherit_a_module_arrays_width() {
    let src = r#"
`timescale 1ns/1ns
package p;
  class holder;
    logic [7:0] m [string];
  endclass
endpackage
module top;
  import p::*;
  logic [3:0] m [string];
  holder h;
  int cls_val, mod_val;
  initial begin
    h = new();
    h.m["k"] = 8'hEF;
    m["k"]   = 8'hEF;
    #1 cls_val = h.m["k"]; mod_val = m["k"];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(
        u(&sim, "cls_val"),
        0xEF,
        "the 8-bit class member keeps its value"
    );
    assert_eq!(u(&sim, "mod_val"), 0xF, "the 4-bit module array truncates");
}

/// An assoc array inside a SUBMODULE gets its width under the instance-scoped
/// name, so the fit applies there too.
#[test]
fn instance_scoped_assoc_fits_its_element_width() {
    let src = r#"
`timescale 1ns/1ns
module sub;
  logic [3:0] aa [string];
  int seen;
  initial begin
    aa["k"] = 8'hEF;
    #1 seen = aa["k"];
  end
endmodule
module top;
  sub u1();
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let v = sim
        .get_signal("u1.seen")
        .expect("u1.seen not found")
        .to_u64()
        .expect("not u64-able");
    assert_eq!(
        v, 0xF,
        "submodule assoc element truncates to its declared width"
    );
}
