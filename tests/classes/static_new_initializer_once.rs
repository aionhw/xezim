//! §6.8, §8.9: a class-typed static with `= new()` constructs exactly one
//! object, and its constructor sees the handle still null. An extern
//! constructor that read the handle re-ran the initializer (two
//! constructions, the inner one reading x), and a package class ran its
//! initializer once per spelling (`p::C` and `C`). Expected values are
//! the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

/// Extern constructor reading its own static handle.
#[test]
fn extern_constructor_reads_static_handle_once() {
    const SRC: &str = r#"class C;
  static C inst = new();
  extern function new();
endclass
function C::new();
  $display("T|ctor null=%0d", inst == null);
endfunction
module tb;
  initial $display("T|after null=%0d", C::inst == null);
endmodule
"#;
    assert_eq!(t_lines(SRC), ["T|ctor null=1", "T|after null=0"]);
}

/// Package class: inline and extern constructors, and a static initializer
/// function, each run once; both spellings name one cell.
#[test]
fn package_class_static_initializers_run_once() {
    const SRC: &str = r#"package p;
  class A;
    static int n;
    static A inst = new();
    int id;
    function new(); n++; id = n; endfunction
  endclass
  class B;
    static local B inst = new();
    extern function new();
  endclass
  function B::new();
    $display("T|B ctor null=%0d", inst == null);
  endfunction
  class F;
    static int k = f();
    static function int f(); $display("T|F f"); return 1; endfunction
  endclass
endpackage
module tb;
  import p::*;
  initial $display("T|A n=%0d id=%0d same=%0d", A::n, A::inst.id, A::inst == p::A::inst);
endmodule
"#;
    let mut got = t_lines(SRC);
    got.sort();
    assert_eq!(got, ["T|A n=1 id=1 same=1", "T|B ctor null=1", "T|F f"]);
}
