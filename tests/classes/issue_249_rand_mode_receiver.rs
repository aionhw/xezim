//! Issue #249: inside a method, `prop.rand_mode(0)` on a non-class random
//! property (no `this.`) tried the property's VALUE as an object handle
//! first. When the value equalled a live object's handle, that whole other
//! object was switched off and `prop` stayed random. A bare name that
//! declares a non-class or rand class-handle property of `this` names that
//! variable (IEEE 1800-2017 §8.11, §18.8), whatever it holds; for a handle
//! only its own mode changes. For a handle that is an interpretation of
//! §18.8, which also reads the call as the object form. Non-rand handle
//! properties, locals and constraint names keep their meaning.

use xezim::simulate;

fn tagged(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T "))
        .collect()
}

#[test]
fn rand_mode_on_a_property_whose_value_equals_a_handle() {
    let out = tagged(
        r#"
class other;
  rand bit [7:0] a;
endclass
class item;
  rand bit [31:0] id;
  rand bit [31:0] len;
  rand other sub;
  constraint c_len { len < 5000; }
  function void freeze();
    id.rand_mode(0);
    len.rand_mode(0);
  endfunction
  function int q_id(); return id.rand_mode(); endfunction
  function void sub_off(); sub.rand_mode(0); endfunction
  function void cons_off(); c_len.constraint_mode(0); endfunction
  function void arg_off(other o2); o2.rand_mode(0); endfunction
endclass
module top;
  initial begin
    other o = new();   // handle 1
    other p = new();
    item  i = new();
    i.id  = 1;         // same value as o's handle
    i.len = 1000;
    i.freeze();
    $display("T id=%0d len=%0d o=%0d q_id=%0d",
             i.id.rand_mode(), i.len.rand_mode(), o.a.rand_mode(), i.q_id());
    void'(i.randomize());
    $display("T after id=%0d len=%0d", i.id, i.len);
    i.sub = p;
    i.sub_off();
    $display("T sub_off sub=%0d p.a=%0d o=%0d", i.sub.rand_mode(), p.a.rand_mode(), o.rand_mode());
    i.cons_off();
    $display("T constraint c_len=%0d", i.c_len.constraint_mode());
    i.arg_off(o);
    $display("T argument o=%0d", o.rand_mode());
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T id=0 len=0 o=1 q_id=0",
            "T after id=1 len=1000",
            "T sub_off sub=0 p.a=1 o=1",
            "T constraint c_len=0",
            "T argument o=0",
        ]
    );
}

/// The same with an inherited property, a type-parameter-typed one bound to
/// `int`, and a module-level variable of the same name holding a handle
/// number.
#[test]
fn rand_mode_receiver_inherited_and_type_parameter_properties() {
    let out = tagged(
        r#"
class other; rand bit [7:0] a; endclass
class item #(type T = int);
  rand bit [31:0] id;
  rand T tp;
  function void freeze(); id.rand_mode(0); tp.rand_mode(0); endfunction
endclass
class base; rand bit [31:0] id; endclass
class derived extends base;
  function void freeze(); id.rand_mode(0); endfunction
endclass
module top;
  int id = 1;
  initial begin
    other o = new();
    item #(int) i = new();
    derived d = new();
    i.id = 1; i.tp = 1; d.id = 1;
    i.freeze();
    d.freeze();
    $display("T i.id=%0d i.tp=%0d d.id=%0d o=%0d",
             i.id.rand_mode(), i.tp.rand_mode(), d.id.rand_mode(), o.rand_mode());
  end
endmodule
"#,
    );
    assert_eq!(out, ["T i.id=0 i.tp=0 d.id=0 o=1"]);
}
