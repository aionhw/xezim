//! Dynamic-array class property: `c.p = new[n]` must record the size so that
//! `c.p.size()` reads back `n`.
//!
//! Before this fix, `new[n]` only resolved *module-scope* dynamic arrays
//! (`module.dynamic_arrays`). A class-property dynamic array lives under the
//! instance-scoped name `<handle>#p`, which is never in that module-scope set,
//! so the sizing block was skipped and `.size()` stayed 0.
//!
//! Both standalone features work — module-scope dynamic arrays (cases 1/2
//! below) and class-property dynamic arrays (cases 3/4). The bug is only in the
//! *combination* "a class containing a dynamic-array property".

use xezim::simulate;

#[test]
fn test_class_dynarray_property_new_size() {
    const SRC: &str = r#"
class C;
  int p[];
  function new(); endfunction
endclass

class D;
  int q[3];
  function new(); endfunction
endclass

module tb;
  int pass_count;
  initial begin
    C c;
    int m[];          // module-scope dynamic array (regression sentinel)
    pass_count = 0;

    // Case 1: module-scope dynamic array still sized correctly.
    m = new[2];
    if (m.size() == 2) pass_count++;

    // Case 2: class-property dynamic array sized via external `c.p = new[n]`.
    c = new();
    c.p = new[5];
    if (c.p.size() == 5) pass_count++;

    // Case 3: re-sizing to a different length.
    c.p = new[3];
    if (c.p.size() == 3) pass_count++;

    // Case 4: element writes survive the sizing path.
    c.p[0] = 7; c.p[1] = 8; c.p[2] = 9;
    if (c.p.size() == 3 && c.p[0] == 7 && c.p[1] == 8 && c.p[2] == 9)
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
    assert_eq!(pc, 4, "class-property dynamic-array new[n] sizing failed");
}
