//! `T a[N]` where `N` is a parameter/localparam.
//!
//! A bare identifier before `]` is assumed by the parser to name a TYPE, so
//! `a[N]` parsed as an associative array keyed by "type" N. The array therefore
//! never got a size: `$size()` was wrong, `%p` printed one struct instead of an
//! element list, and the elements were never pre-registered. Elaboration now
//! rewrites such a dimension back to a fixed size when the identifier names a
//! parameter and not a type — while leaving genuine associative arrays
//! (`int m[int]`, `int m[string]`, `int m[key_t]`) alone.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct { int v; } r_t;
  typedef int key_t;
  parameter  int P = 3;
  localparam int L = 2;

  r_t ppar[P];
  r_t lpar[L];
  int assoc_i[int];
  int assoc_s[string];
  int assoc_t[key_t];

  int sz_p, sz_l;
  int p2v, l1v;
  int ai10, asK, atK;
  int n_assoc;

  initial begin
    ppar[2].v = 9;
    lpar[1].v = 8;
    assoc_i[10] = 100;
    assoc_s["k"] = 7;
    assoc_t[5]  = 55;

    sz_p = $size(ppar);
    sz_l = $size(lpar);
    p2v = ppar[2].v;
    l1v = lpar[1].v;
    ai10 = assoc_i[10];
    asK  = assoc_s["k"];
    atK  = assoc_t[5];
    n_assoc = assoc_i.num();
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

#[test]
fn parameter_sized_arrays_are_fixed_not_associative() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Parameter- and localparam-sized arrays get a real size.
    assert_eq!(u(&sim, "sz_p") & 0xFFFF_FFFF, 3, "$size(ppar) wrong — a[P] not a fixed array");
    assert_eq!(u(&sim, "sz_l") & 0xFFFF_FFFF, 2, "$size(lpar) wrong — a[L] not a fixed array");

    // Their elements are addressable.
    assert_eq!(u(&sim, "p2v") & 0xFFFF_FFFF, 9);
    assert_eq!(u(&sim, "l1v") & 0xFFFF_FFFF, 8);

    // Genuine associative arrays must be untouched.
    assert_eq!(u(&sim, "ai10") & 0xFFFF_FFFF, 100, "int-keyed assoc broke");
    assert_eq!(u(&sim, "asK") & 0xFFFF_FFFF, 7, "string-keyed assoc broke");
    assert_eq!(u(&sim, "atK") & 0xFFFF_FFFF, 55, "typedef-keyed assoc broke");
    assert_eq!(u(&sim, "n_assoc") & 0xFFFF_FFFF, 1, "assoc num() broke");
}
