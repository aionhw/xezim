//! Two bugs the sibling audit turned up, both confirmed against a reference
//! simulator before being fixed, and both silent — the code ran and produced
//! plausible-looking state.
//!
//! 1. An associative-array write from inside an `always_ff` was DROPPED
//!    entirely — `exists()` returned 0 afterwards, for blocking and
//!    non-blocking alike — while the identical write from an `initial` block
//!    worked. Cause: an `always_ff` body is bytecode-compiled, and none of the
//!    bytecode store paths can address an assoc element (the key is not a dense
//!    index and the elements have no signal ids). `lookup_array_name` misses,
//!    and the fall-through treated the base as a scalar and wrote a bit of a
//!    phantom signal. The compiler now bails on an assoc target so the
//!    statement runs on the AST path, which handles it. Scoreboards and
//!    coverage code write assoc arrays from clocked blocks constantly, so this
//!    lost data quietly.
//!
//! 2. String methods other than `len()` returned empty on an ELEMENT of a
//!    string collection (`q[0].substr(1,3)`, `q[0][2]`). `eval_builtin_method`
//!    resolves its receiver by NAME, and only `len`/`size` had a fallback for a
//!    non-identifier base — so an element reported the right LENGTH with no
//!    CONTENT, which reads like data corruption rather than a missing feature.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Blocking and non-blocking assoc writes from a clocked block must land, and
/// agree with the same write from an `initial` block.
#[test]
fn associative_writes_from_always_ff_land() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] a_blk [string];
  logic [3:0] a_nba [string];
  logic [3:0] a_ini [string];
  int v_blk, v_nba, v_ini, e_blk, e_nba, e_ini;
  always_ff @(posedge clk) a_blk["k"]  = 4'hD;
  always_ff @(posedge clk) a_nba["k"] = 4'hC;
  initial begin
    @(posedge clk);
    a_ini["k"] = 4'hE;
    #20;
    v_blk = a_blk["k"]; v_nba = a_nba["k"]; v_ini = a_ini["k"];
    e_blk = a_blk.exists("k");
    e_nba = a_nba.exists("k");
    e_ini = a_ini.exists("k");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "e_blk"),
        1,
        "blocking write from always_ff must create the entry"
    );
    assert_eq!(u(&sim, "e_nba"), 1, "non-blocking too");
    assert_eq!(
        u(&sim, "e_ini"),
        1,
        "and the initial-block control still works"
    );
    assert_eq!(u(&sim, "v_blk"), 0xD, "blocking value");
    assert_eq!(u(&sim, "v_nba"), 0xC, "non-blocking value");
    assert_eq!(u(&sim, "v_ini"), 0xE, "initial-block value");
}

/// A clocked block that keys an assoc array by a RUNTIME value — the shape a
/// scoreboard actually uses, and the one where the drop mattered most.
///
/// The stored value is a plain expression rather than `hits[key] + 1`: a
/// read-modify-write of an assoc element stores 0 instead of the incremented
/// value, which is a SEPARATE pre-existing bug (it reproduces in an `initial`
/// block, and on the build before this fix). Keeping it out keeps this test
/// about what it names.
#[test]
fn associative_write_from_always_ff_with_a_dynamic_key() {
    let src = r#"
`timescale 1ns/1ns
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  int  hits [int];
  int  key, stamp;
  int  n0, n1, total;
  always_ff @(posedge clk) hits[key] = stamp;
  initial begin
    key = 0; stamp = 7;
    repeat (2) @(posedge clk);
    key = 1; stamp = 9;
    repeat (3) @(posedge clk);
    #1;
    n0 = hits[0]; n1 = hits[1]; total = hits.num();
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("simulate failed");
    assert_eq!(
        u(&sim, "total"),
        2,
        "two distinct runtime keys were written"
    );
    assert_eq!(u(&sim, "n0"), 7, "key 0 holds what the clocked block wrote");
    assert_eq!(u(&sim, "n1"), 9, "key 1 likewise");
}

/// String methods and character indexing work on an element of a queue, an
/// associative array, and a fixed array — not just on a plain variable.
#[test]
fn string_methods_work_on_collection_elements() {
    let src = r#"
module top;
  string q [$];
  string aa [string];
  string arr [2];
  int q_sub_ok, q_idx, aa_sub_ok, arr_sub_ok, q_up_ok, q_getc;
  initial begin
    q.push_back("hello");
    aa["k"] = "world";
    arr[1]  = "there";
    q_sub_ok  = (q[0].substr(1, 3) == "ell");
    q_idx     = q[0][0];
    q_getc    = q[0].getc(1);
    q_up_ok   = (q[0].toupper() == "HELLO");
    aa_sub_ok = (aa["k"].substr(0, 2) == "wor");
    arr_sub_ok = (arr[1].substr(0, 2) == "the");
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "q_sub_ok"), 1, "substr on a queue element");
    assert_eq!(
        u(&sim, "q_idx"),
        b'h' as u64,
        "character select on a queue element"
    );
    assert_eq!(u(&sim, "q_getc"), b'e' as u64, "getc on a queue element");
    assert_eq!(u(&sim, "q_up_ok"), 1, "toupper on a queue element");
    assert_eq!(u(&sim, "aa_sub_ok"), 1, "substr on an assoc element");
    assert_eq!(u(&sim, "arr_sub_ok"), 1, "substr on a fixed-array element");
}

/// Those methods still work on a LONG element — the two fixes have to compose,
/// since a 200-character element also exercises the dynamic-width path.
#[test]
fn string_methods_on_a_long_collection_element() {
    let src = r#"
module top;
  string big;
  string q [$];
  int len_n, head_ok, tail_ok, first_c;
  initial begin
    big = "";
    repeat (200) big = {big, "y"};
    q.push_back(big);
    len_n   = q[0].len();
    head_ok = (q[0].substr(0, 2) == "yyy");
    tail_ok = (q[0].substr(197, 199) == "yyy");
    first_c = q[0][0];
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(u(&sim, "len_n"), 200, "length survives");
    assert_eq!(u(&sim, "head_ok"), 1, "front of the text readable");
    assert_eq!(u(&sim, "tail_ok"), 1, "end of the text readable");
    assert_eq!(u(&sim, "first_c"), b'y' as u64, "character select");
}
