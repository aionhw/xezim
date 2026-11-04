//! Nested class-member dynamic array of class handles.
//!
//! Regression for the UVM `NOPACK` member-array scenario (`a.mid.base = new[N]`,
//! populating and reading `a.mid.base[i]`, and packing/unpacking the nested
//! objects).
//! Exercises three interlocking behaviors that xezim previously got wrong:
//!   1. A `new` whose rvalue is assigned to an OBJECT-MEMBER array element
//!      (`obj.member[i] = new(...)`) must construct the DECLARED element
//!      class (`base_class`), not dispatch the bare `new` onto the enclosing
//!      component's constructor (which tripped UVM's ILLCRT guard).
//!   2. `obj.member = new[N]` (a member array of a NESTED object handle,
//!      flattened to a 3+ segment `a.mid.base`) must be treated as dynamic-
//!      array SIZING with per-instance storage `<handle>#member`, not as a
//!      scalar/object handle store.
//!   3. Elements of that nested array must read/write consistently.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn nested_member_class_array_size_fill_and_read() {
    let src = r#"
class base_class;
  int a;
  function new(string name="base"); endfunction
endclass

class mid_class;
  int a;
  base_class base[];
  function new(string name="mid"); endfunction
endclass

class my_class;
  int a;
  mid_class mid;
  function new(string name="my"); endfunction
endclass

module top;
  initial begin
    my_class a;
    a = new("a");
    a.mid = new("b");
    a.mid.base = new[2];
    a.mid.base[0] = new("b0"); a.mid.base[0].a = 5;
    a.mid.base[1] = new("b1"); a.mid.base[1].a = 7;
    $display("T|%0d %0d %0d", a.mid.base.size(), a.mid.base[0].a, a.mid.base[1].a);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert!(
        msgs(&sim).iter().any(|m| m == "T|2 5 7"),
        "got {:?}",
        msgs(&sim)
    );
}
