//! Randomizing a class that contains a `rand obj arr[]` (dynamic array of rand
//! objects).
//!
//! Before this fix, `randomize()` treated `arr` two conflicting ways:
//!   1. as a collection to draw fresh random scalars for (overwriting each
//!      element handle with a random 32-bit number — destroying the object
//!      reference and any aliasing), AND
//!   2. as a single object handle to recurse into (but a collection isn't
//!      stored as one scalar handle, so the recursion was a no-op).
//!
//! The fix:
//!   * collection members whose element type is a CLASS are NOT drawn as
//!     scalars (their handles are randomized recursively, §18.4.1), and
//!   * such a member's element handles are iterated (`<handle>#arr[i]`) and
//!     each is randomized recursively, preserving aliasing (a shared element
//!     is randomized once, and both slots read the same result).

use xezim::simulate;

#[test]
fn test_randomize_rand_obj_array() {
    const SRC: &str = r#"
class obj;
  rand int i;
  function new(); endfunction
endclass

class container;
  rand obj arr[];
  rand int sarr[];   // scalar array — must still be randomized
  function new(); endfunction
endclass

module tb;
  int pass_count;
  initial begin
    container c;
    pass_count = 0;

    c = new();
    c.arr = new[3];
    c.arr[0] = new(); c.arr[1] = new(); c.arr[2] = new();
    c.arr[2] = c.arr[1];   // alias: arr[1] and arr[2] share one handle
    c.sarr = new[2];
    c.sarr[0] = 5; c.sarr[1] = 6;

    void'(c.randomize());

    // Case 1: object-array elements were randomized (rand fields changed).
    if (c.arr[0].i != 0 && c.arr[1].i != 0) pass_count++;
    // Case 2: aliasing preserved — arr[1] and arr[2] are the same object, so
    // they hold the same randomized value.
    if (c.arr[1] == c.arr[2] && c.arr[1].i == c.arr[2].i) pass_count++;
    // Case 3: scalar array still randomized (regression guard).
    if (c.sarr[0] != 5 && c.sarr[1] != 6) pass_count++;
    // Case 4: size preserved.
    if (c.arr.size() == 3 && c.sarr.size() == 2) pass_count++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let pc: u64 = sim
        .get_signal("pass_count")
        .unwrap_or_else(|| panic!("signal not found: pass_count"))
        .to_u64()
        .unwrap_or_else(|| panic!("pass_count not u64-able"));
    assert_eq!(pc, 4, "randomize of rand-object dynamic-array failed");
}
