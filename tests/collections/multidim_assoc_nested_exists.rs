//! Multidimensional associative arrays (IEEE 1800-2023 §7.8.1): a built-in
//! method like `.exists()`/`.num()` reached through a *nested* index receiver
//! — `m[k1][k2].exists(k3)` — must dispatch to the stored compound key
//! `m[K1][K2]`.
//!
//! Before this fix, `nested_index_name` only matched a single-level
//! `Index(Ident, idx)` receiver. A depth-2 receiver `Index(Index(m, k1), k2)`
//! has a non-`Ident` base, so it returned `None` and `.exists()`/`.num()` never
//! dispatched for 3D+ assoc arrays. Elements were written correctly (the write
//! path `multidim_assoc_elem` recurses), but every nested `.exists()` falsely
//! reported absent and `.num()` reported 0 — silently breaking any code that
//! relies on deep nested assoc lookups (including UVM's 3D cycle detector
//! `m_recur_states[obj][obj][policy]`).

use xezim::simulate;

#[test]
fn test_multidim_assoc_nested_exists() {
    const SRC: &str = r#"
typedef enum { A, B } pol_t;

module tb;
  // 2D and 3D associative arrays.
  int m1 [int][pol_t];
  int m2 [int][int][int];
  int m3 [pol_t][pol_t][pol_t];
  int pass_count;

  initial begin
    pass_count = 0;

    // 2D [int][enum]: write then check both levels of .exists().
    m1[3][A] = 42;
    if (m1.exists(3) && m1[3].exists(A))
      pass_count++;
    if (!m1[3].exists(B))
      pass_count++;
    if (m1[3][A] == 42)
      pass_count++;

    // 3D [int][int][int]: the depth that was broken.
    m2[1][2][3] = 99;
    if (m2.exists(1) && m2[1].exists(2) && m2[1][2].exists(3))
      pass_count++;
    if (!m2[1][2].exists(9))
      pass_count++;
    if (m2[1][2][3] == 99)
      pass_count++;

    // 3D [enum][enum][enum]: all-enum-keyed.
    m3[A][B][A] = 7;
    if (m3.exists(A) && m3[A].exists(B) && m3[A][B].exists(A))
      pass_count++;
    if (m3[A][B][A] == 7)
      pass_count++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let pc: u64 = sim
        .get_signal("pass_count")
        .unwrap_or_else(|| panic!("signal not found: pass_count"))
        .to_u64()
        .unwrap_or_else(|| panic!("pass_count not u64-able"));
    assert_eq!(pc, 8, "multidim associative-array nested .exists() failed");
}
