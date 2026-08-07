//! Whole-array copy between a class-property dynamic array and a fixed array.
//!
//! Before this fix, `c.p = tmp` (a module-scope fixed array assigned to a
//! class-property dynamic array) silently dropped the copy: the collection
//! resolver recognized only dynamic collections, so a fixed-array RHS resolved
//! to `None` and the copy was skipped.
//!
//! This test exercises both directions plus descending fixed arrays:
//!   * `c.p = tmp` (module fixed → class-property dynamic) — destination
//!     resized to the source length,
//!   * `tmp = c.p` (class-property dynamic → module fixed) — copies up to the
//!     fixed destination's capacity,
//!   * a descending `[2:0]` fixed destination receives elements in the correct
//!     semantic order.
//!
//! Related gap (NOT covered here): collection-property → collection-property
//! (`c.p = c.q`) takes a different assign path and is tracked separately.

use xezim::simulate;

#[test]
fn test_dynarray_property_fixed_array_copy() {
    const SRC: &str = r#"
class C;
  int p[];
  function new(); endfunction
endclass

module tb;
  int pass_count;
  initial begin
    C c;
    int tmp[3];      // ascending fixed array
    int dsc[2:0];    // descending fixed array
    pass_count = 0;

    c = new();
    tmp[0] = 1; tmp[1] = 2; tmp[2] = 3;

    // Case 1: module fixed array -> class-property dynamic array.
    c.p = tmp;
    if (c.p.size() == 3 && c.p[0] == 1 && c.p[1] == 2 && c.p[2] == 3)
      pass_count++;

    // Case 2: class-property dynamic array -> module fixed array.
    c.p[0] = 7; c.p[1] = 8; c.p[2] = 9;
    tmp = c.p;
    if (tmp[0] == 7 && tmp[1] == 8 && tmp[2] == 9)
      pass_count++;

    // Case 3: descending fixed destination receives elements in ascending
    // index order (dsc[2] <- src[0], dsc[1] <- src[1], dsc[0] <- src[2]).
    c.p[0] = 4; c.p[1] = 5; c.p[2] = 6;
    dsc = c.p;
    if (dsc[2] == 4 && dsc[1] == 5 && dsc[0] == 6)
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
    assert_eq!(
        pc, 3,
        "class-property dynamic-array <-> fixed-array whole-array copy failed"
    );
}
