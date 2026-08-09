//! Constructing objects into a class-property dynamic-array element *inside a
//! foreach body*: `foreach(inst.arr[idx]) inst.arr[idx] = new();`.
//!
//! Before this fix, the `collection[key] = new(...)` element-class resolver
//! recognized only a flattened `Ident([obj, member])` base (the parsed form of
//! `c.p[i]`). Inside a `foreach` body the index is a loop variable, so the
//! collection base is lowered to a `MemberAccess(obj, member)` shape — which
//! the resolver's root-walk (an `if let Ident`) rejected, dropping the
//! constructor and leaving elements null.
//!
//! Also covered (by the same fix): int-element writes through a foreach loop
//! variable (`foreach(c.a[i]) c.a[i] = i*10;`) and storing existing handles
//! (`foreach(c.a[i]) c.a[i] = h;`).

use xezim::simulate;

#[test]
fn test_foreach_dynarray_property_elem_new() {
    const SRC: &str = r#"
class obj;
  int i;
  function new(input int v = 0); i = v; endfunction
endclass

class container;
  obj arr[];
  function new(); endfunction
endclass

module tb;
  int pass_count;
  initial begin
    container inst;
    pass_count = 0;

    inst = new();
    inst.arr = new[4];

    // Construct objects into each element inside a foreach body.
    foreach (inst.arr[idx])
      inst.arr[idx] = new();

    // Case 1: elements populated.
    if (inst.arr.size() == 4
        && inst.arr[0] != null && inst.arr[1] != null
        && inst.arr[2] != null && inst.arr[3] != null)
      pass_count++;

    // Case 2: a second foreach assigns distinct values to each element.
    foreach (inst.arr[idx])
      inst.arr[idx].i = idx * 10;
    if (inst.arr[0].i == 0 && inst.arr[1].i == 10
        && inst.arr[2].i == 20 && inst.arr[3].i == 30)
      pass_count++;

    // Case 3: aliasing inside a foreach body.
    inst.arr[2] = inst.arr[1];
    if (inst.arr[1] == inst.arr[2]) pass_count++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let pc: u64 = sim
        .get_signal("pass_count")
        .unwrap_or_else(|| panic!("signal not found: pass_count"))
        .to_u64()
        .unwrap_or_else(|| panic!("pass_count not u64-able"));
    assert_eq!(pc, 3, "foreach-body element construction failed");
}
