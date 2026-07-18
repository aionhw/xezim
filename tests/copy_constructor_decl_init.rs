//! §8.12 shallow copy via the DECLARATION-INITIALIZER form `T o2 = new o1;`.
//! This must copy o1's property values — including nested object HANDLES, which
//! are SHARED (not cloned). The decl-init path used to run the constructor
//! (`new()`), passing the handle as an ignored argument, so it produced a fresh
//! object instead of a copy. The separate-assignment form always worked.

use xezim::simulate;

fn line(src: &str, tag: &str) -> String {
    let sim = simulate(src, 100).expect("sim");
    sim.output.iter().find(|o| o.message.starts_with(tag))
        .map(|o| o.message.clone()).unwrap_or_default()
}

#[test]
fn decl_init_copy_shares_nested_handle() {
    let src = "\
class Inner; int v; endclass\n\
class Outer; int a; Inner in; function new(); in=new(); endfunction endclass\n\
module t; initial begin\n\
  Outer o1=new(); o1.a=5; o1.in.v=10;\n\
  Outer o2=new o1;\n\
  o2.a=99; o2.in.v=42;\n\
  $display(\"A o1a=%0d o1inv=%0d\", o1.a, o1.in.v);\n\
  if (o1.in == o2.in) $display(\"B SHARED\"); else $display(\"B DIFF\");\n\
  $finish; end endmodule";
    // scalar `a` is copied then set independently; nested `in` handle is SHARED.
    assert_eq!(line(src, "A "), "A o1a=5 o1inv=42", "scalar independent, nested handle shared");
    assert_eq!(line(src, "B "), "B SHARED", "nested object handle must be shared");
}

#[test]
fn decl_init_copy_copies_scalar_values() {
    let src = "\
class C; int a; function new(); a=7; endfunction endclass\n\
module t; initial begin\n\
  C c1=new(); c1.a=5;\n\
  C c2=new c1;\n\
  $display(\"V %0d\", c2.a);\n\
  $finish; end endmodule";
    // must copy c1.a (=5), NOT run the constructor (which sets a=7).
    assert_eq!(line(src, "V "), "V 5", "copy must take source value, not construct");
}
