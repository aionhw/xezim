//! Two more silent-corruption bugs from the LRM audits.
//!
//! 1. A LOCAL array's declaration initializer was dropped: `int a[4] =
//!    '{10,20,30,40};` inside a block read back all zeros. The same declaration
//!    at module scope worked. A local MULTI-dimensional array was also
//!    registered from its first dimension only, so `int n[2][2]` became a 1-D
//!    array of double-width elements and `n[1][0]` read 0.
//!
//! 2. `always @(s) cnt++;` ran 100 times (the settle cap) per change of `s`.
//!    A sensitivity list of all-`AnyEdge` signals was routed through the
//!    combinational settle path, which re-runs the block whenever any value it
//!    reads changes — including the ones its own writes caused. Only bodies that
//!    read what they write diverge, so those now take the edge path; an
//!    idempotent body (`always @(a or b) y = a & b;`) keeps the comb path, which
//!    it needs in order to react to a change in a variable INDEX
//!    (`always @(vco_tap[c_ph_val[0]])` — tests/prtest/pr2011429.v).

use xezim::simulate;

const LOCAL_ARRAYS: &str = r#"
module tb;
  typedef struct { int a; string s; } p_t;
  int ord0, ord3, rep2, dflt3, nested, sarr_a, sum;
  initial begin
    int a  [4]    = '{10, 20, 30, 40};
    int r  [3]    = '{3{7}};
    int d  [4]    = '{default:9};
    int n  [2][2] = '{'{1,2}, '{3,4}};
    p_t sa [2]    = '{'{1,"x"}, '{2,"y"}};

    ord0   = a[0];
    ord3   = a[3];
    rep2   = r[2];
    dflt3  = d[3];
    nested = n[1][0];
    sarr_a = sa[1].a;

    sum = 0;
    foreach (a[i]) sum = sum + a[i];
  end
endmodule
"#;

/// `always @(s) cnt++` must fire once per change of `s`, not settle_limit times.
/// The idempotent forms must keep working.
const EDGE_ALWAYS: &str = r#"
module tb;
  logic s = 0;
  logic a = 0, b = 0;
  int c_any, c_pos, c_blk;
  logic y_list, y_star, y_comb;

  always @(edge s)    c_any++;      // non-idempotent, explicit list
  always @(posedge s) c_pos++;
  always @(a or b) y_list = a & b;  // idempotent, explicit list
  always @*        y_star = a | b;
  always_comb      y_comb = a ^ b;

  initial forever begin @(edge s); c_blk++; end

  initial begin
    #5 s = 1;
    #5 s = 0;
    #5 a = 1;
    #5 b = 1;
    #5 $finish;
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn a_local_array_declaration_initializer_is_applied() {
    let sim = simulate(LOCAL_ARRAYS, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ord0"), 10, "a local array initializer was dropped");
    assert_eq!(u(&sim, "ord3"), 40);
    assert_eq!(u(&sim, "sum"), 100);
}

#[test]
fn every_pattern_form_works_on_a_local_array() {
    let sim = simulate(LOCAL_ARRAYS, 100).expect("simulate failed");
    assert_eq!(u(&sim, "rep2"), 7, "replication");
    assert_eq!(u(&sim, "dflt3"), 9, "default:");
    assert_eq!(u(&sim, "nested"), 3, "a local 2-D array was registered as 1-D");
    assert_eq!(u(&sim, "sarr_a"), 2, "an unpacked-struct element read 0");
}

#[test]
fn an_explicit_sensitivity_list_fires_once_per_change() {
    let sim = simulate(EDGE_ALWAYS, 200).expect("simulate failed");
    // `s` goes x->0 at t0 then 0->1 and 1->0: three AnyEdge events.
    // Before the fix this was 100 (the settle cap).
    assert_eq!(u(&sim, "c_any"), 3, "always @(edge s) ran away");
    assert_eq!(u(&sim, "c_pos"), 1);
    // A blocking `@(edge s)` never had the bug; it sees the two real edges.
    assert_eq!(u(&sim, "c_blk"), 2);
}

#[test]
fn idempotent_bodies_keep_their_combinational_behaviour() {
    let sim = simulate(EDGE_ALWAYS, 200).expect("simulate failed");
    // At the end a = b = 1.
    assert_eq!(u(&sim, "y_list") & 1, 1, "always @(a or b) y = a & b");
    assert_eq!(u(&sim, "y_star") & 1, 1, "always @* y = a | b");
    assert_eq!(u(&sim, "y_comb") & 1, 0, "always_comb y = a ^ b");
}
