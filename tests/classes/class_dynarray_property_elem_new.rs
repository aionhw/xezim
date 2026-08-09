//! Constructing objects directly into a class-property dynamic-array element:
//! `c.p[i] = new(...)`.
//!
//! Before this fix, the element-class resolver for `collection[key] = new(...)`
//! found the element type only for module-scope/static collections
//! (`array_elem_class`), local vars (`var_class_types`), or properties reached
//! inside a class method (`class_context_stack`). A `c.p[i] = new(...)` from an
//! `initial` block (the base is a flattened `Ident([c, p])` with 2+ segments)
//! resolved to `None`, so the constructor was dropped and the element stayed
//! null.
//!
//! Also note: storing an EXISTING handle (`c.p[i] = h`) already worked — only
//! the `new(...)` construction path was broken, because it keys off the
//! collection's element type.

use xezim::simulate;

#[test]
fn test_dynarray_property_elem_new() {
    const SRC: &str = r#"
class obj;
  int i;
  function new(input int v = 0); i = v; endfunction
endclass

class C;
  obj p[];
  function new(); endfunction
endclass

module tb;
  int pass_count;
  initial begin
    C c;
    pass_count = 0;

    c = new();
    c.p = new[3];
    // Construct objects directly into elements.
    c.p[0] = new(10);
    c.p[1] = new(20);
    // Aliasing: two slots share one handle.
    c.p[2] = c.p[1];

    // Case 1: element construction landed.
    if (c.p.size() == 3 && c.p[0].i == 10 && c.p[1].i == 20 && c.p[2].i == 20)
      pass_count++;
    // Case 2: aliasing preserved.
    if (c.p[1] == c.p[2]) pass_count++;

    // Case 3: storing an existing handle still works (regression guard).
    obj h;
    h = new(42);
    c.p[0] = h;
    if (c.p[0] != null && c.p[0].i == 42) pass_count++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let pc: u64 = sim
        .get_signal("pass_count")
        .unwrap_or_else(|| panic!("signal not found: pass_count"))
        .to_u64()
        .unwrap_or_else(|| panic!("pass_count not u64-able"));
    assert_eq!(pc, 3, "class-property dynamic-array element construction failed");
}
