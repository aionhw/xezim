//! Members of an UNPACKED struct stored in an unpacked array.
//!
//! Array elements were given a packed bit-slice layout regardless of whether
//! the element struct was packed. For an unpacked element that is wrong:
//!   * a `real` member has no meaningful bit offset, so it read back as raw
//!     bits (e.g. 4609434218613702656.0 instead of 1.5);
//!   * a member sitting next to a `string`/packed-struct member had its offset
//!     shifted, so `arr[0].m` and `arr[1].m` collided on the same bits.
//! Unpacked elements now keep each member in its own `arr[i].member` signal,
//! carrying the declared width / signedness / real-ness.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct packed { bit [3:0] vlan; bit [11:0] id; } tag_t;
  typedef struct { int i; bit [7:0] b; string s; real r; } m_t;
  typedef struct { int status; tag_t tag; string name; } node_t;

  // array of unpacked structs nested INSIDE a struct
  typedef struct { int cid; node_t nodes[2]; } cluster_t;

  m_t       a[2];
  node_t    arr[2];
  cluster_t c;

  // observation copies (hierarchical reads of array members)
  int  a0_i, a1_i;
  real a0_r, a1_r;
  int  n0_st, n1_st;
  int  vlan0;          // nested PACKED member inside an unpacked element
  int  c0_st, c1_st;   // array-of-structs nested inside a struct

  initial begin
    a[0].i = 11; a[0].b = 8'hAB; a[0].s = "A0"; a[0].r = 1.5;
    a[1].i = 22; a[1].b = 8'hCD; a[1].s = "A1"; a[1].r = 2.5;
    arr[0].status = 11; arr[1].status = 22;
    arr[0].name = "N0";  arr[1].name = "N1";
    arr[0].tag = '{vlan: 4'h3, id: 12'h0AB};
    c.nodes[0].status = 11; c.nodes[1].status = 22;

    a0_i = a[0].i; a1_i = a[1].i;
    a0_r = a[0].r; a1_r = a[1].r;
    n0_st = arr[0].status; n1_st = arr[1].status;
    vlan0 = arr[0].tag.vlan;
    c0_st = c.nodes[0].status; c1_st = c.nodes[1].status;
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

fn f(sim: &xezim::compiler::Simulator, n: &str) -> f64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_f64()
}

#[test]
fn unpacked_struct_array_members_keep_per_element_types() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Distinct elements must not collide.
    assert_eq!(u(&sim, "a0_i") & 0xFFFF_FFFF, 11);
    assert_eq!(u(&sim, "a1_i") & 0xFFFF_FFFF, 22);

    // `real` members keep is_real (previously read back as raw bits).
    assert!((f(&sim, "a0_r") - 1.5).abs() < 1e-9, "a[0].r = {}", f(&sim, "a0_r"));
    assert!((f(&sim, "a1_r") - 2.5).abs() < 1e-9, "a[1].r = {}", f(&sim, "a1_r"));

    // A member sitting beside a packed-struct / string member must not alias.
    assert_eq!(u(&sim, "n0_st") & 0xFFFF_FFFF, 11, "arr[0].status aliased");
    assert_eq!(u(&sim, "n1_st") & 0xFFFF_FFFF, 22, "arr[1].status aliased");

    // Nested PACKED struct member inside an unpacked element: arr[0].tag.vlan.
    assert_eq!(u(&sim, "vlan0") & 0xF, 0x3, "arr[0].tag.vlan not sliced from its own signal");

    // Array-of-structs nested inside a struct: c.nodes[i].status.
    assert_eq!(u(&sim, "c0_st") & 0xFFFF_FFFF, 11, "c.nodes[0].status aliased");
    assert_eq!(u(&sim, "c1_st") & 0xFFFF_FFFF, 22, "c.nodes[1].status aliased");
}
