//! The last three findings from the clause-7 audit.
//!
//! §11.4.5 — `==` / `!=` on unpacked structs compare member by member. Their
//! leaves live in separate signals, so the packed-value path compared two
//! container signals that do not exist and always yielded X.
//!
//! §7.3.2 — a tagged union stores one member at a time. Reading the active
//! member yields its value; reading any other member is not valid and yields X.
//! Only `case ... matches` could read a tagged union before; `t.Valid` was X.
//!
//! §20.7 — `$dimensions` counts every packed and unpacked dimension. Only one
//! unpacked dimension was ever counted, so `int m[2][3]` reported 1 (and
//! `$unpacked_dimensions` reported 0).

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct { int k; }                in_t;
  typedef struct { int a; in_t n; string s; } up_t;
  typedef struct packed { bit [3:0] x; bit [3:0] y; } pk_t;
  typedef union tagged { void Invalid; int Valid; } tu_t;

  up_t u, v, w;
  up_t un1, un2;      // never written: comparison must be X
  pk_t p, q;
  tu_t t;

  int m [2][3];
  int n [2][3][4];
  bit [7:0] pkarr [2];
  int one [5];
  int scalar;

  int eq_same, eq_diff, neq_diff, nested_diff, packed_eq;
  int tag_active;
  int dim_m, dim_n, dim_pk, dim_one, dim_scalar, undim_m, undim_one;

  initial begin
    u = '{1, '{7}, "x"};
    v = '{1, '{7}, "x"};
    w = '{1, '{8}, "x"};    // differs only in the NESTED member

    eq_same     = (u == v);
    eq_diff     = (u == w);
    neq_diff    = (u != w);
    nested_diff = (u == w);

    p = 8'hAB; q = 8'hAB;
    packed_eq = (p == q);

    t = tagged Valid (5);
    tag_active = t.Valid;

    dim_m      = $dimensions(m);
    dim_n      = $dimensions(n);
    dim_pk     = $dimensions(pkarr);
    dim_one    = $dimensions(one);
    dim_scalar = $dimensions(scalar);
    undim_m    = $unpacked_dimensions(m);
    undim_one  = $unpacked_dimensions(one);

    $display("UNINIT=%b", un1 == un2);
    $display("INACTIVE=%b", $isunknown(t.Invalid));
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

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn unpacked_struct_equality_is_member_wise() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "eq_same"), 1, "u == v (identical) was not 1");
    assert_eq!(u(&sim, "eq_diff"), 0);
    assert_eq!(u(&sim, "neq_diff"), 1);
    // A difference buried in a nested struct member must still be seen.
    assert_eq!(u(&sim, "nested_diff"), 0);
    // Packed structs are one signal and keep the vector path.
    assert_eq!(u(&sim, "packed_eq"), 1);
}

#[test]
fn comparing_uninitialised_structs_yields_x() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(line(&sim, "UNINIT="), "UNINIT=x");
}

#[test]
fn tagged_union_member_reads_honour_the_active_tag() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "tag_active"), 5, "t.Valid did not read the stored value");
    // Reading a member that is not the active one is invalid -> X.
    assert_eq!(line(&sim, "INACTIVE="), "INACTIVE=1");
}

#[test]
fn dimensions_counts_every_dimension() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Packed + unpacked. `int` contributes one packed dimension.
    assert_eq!(u(&sim, "dim_m"), 3, "int m[2][3] -> 2 unpacked + 1 packed");
    assert_eq!(u(&sim, "dim_n"), 4, "int n[2][3][4] -> 3 unpacked + 1 packed");
    assert_eq!(u(&sim, "dim_pk"), 2, "bit [7:0] pkarr[2]");
    assert_eq!(u(&sim, "dim_one"), 2, "int one[5]");
    assert_eq!(u(&sim, "dim_scalar"), 1, "a plain int");

    assert_eq!(u(&sim, "undim_m"), 2);
    assert_eq!(u(&sim, "undim_one"), 1);
}
