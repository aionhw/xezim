//! §6.18, §8.3, §8.13: a type identifier shall be declared before it is used,
//! and a class handle or base class shall name a class. A declaration whose
//! type nothing in the design declares used to elaborate as an untyped
//! handle: `randomize()` did nothing, every member read `x` and the run
//! finished normally, so a bench missing an include looked like a pass.

use xezim::simulate;

fn error_of(src: &str) -> String {
    match simulate(src, 10) {
        Ok(sim) => panic!("accepted; output {:?}", sim.output),
        Err(e) => e,
    }
}

/// The published bench: the class it instantiates is in no compiled file.
#[test]
fn undefined_class_in_a_block_declaration() {
    let e = error_of(
        r#"
module tb;
  initial begin
    addr_constrained_txn txn = new();
    txn.randomize();
    $display("addr=%0h", txn.addr);
  end
endmodule
"#,
    );
    assert!(
        e.contains("'addr_constrained_txn' is not a declared type or class"),
        "{e}"
    );
    assert!(e.contains("type of 'txn'"), "{e}");
    // Located at the type name: line 4, column 5 of the source.
    assert!(e.contains(":4:5"), "{e}");
}

#[test]
fn undefined_type_of_a_module_variable() {
    let e = error_of(
        r#"
module tb;
  undefined_t v;
  initial $display("v=%0d", v);
endmodule
"#,
    );
    assert!(e.contains("'undefined_t' is not a declared type"), "{e}");
    assert!(e.contains(":3:3"), "{e}");
}

#[test]
fn undefined_class_of_a_class_property() {
    let e = error_of(
        r#"
class C;
  nope_c h;
  function new(); h = new(); endfunction
endclass
module tb;
  C c;
  initial begin c = new(); $display("done"); end
endmodule
"#,
    );
    assert!(e.contains("'nope_c' is not a declared type"), "{e}");
    assert!(e.contains(":3:3"), "{e}");
}

#[test]
fn undefined_type_of_a_subroutine_port_return_and_local() {
    let e = error_of(
        r#"
module tb;
  function nope_ret_t f(nope_arg_t a);
    nope_local_t l;
    return a;
  endfunction
  task automatic t;
    for (nope_loop_t i = 0; i < 1; i++) $display("i");
  endtask
  initial t();
endmodule
"#,
    );
    for n in ["nope_ret_t", "nope_arg_t", "nope_local_t", "nope_loop_t"] {
        assert!(
            e.contains(&format!("'{n}' is not a declared type")),
            "{n}: {e}"
        );
    }
}

#[test]
fn undefined_base_class() {
    let e = error_of(
        r#"
class D extends nope_base;
  int x;
endclass
module tb;
  D d;
  initial begin d = new(); $display("x=%0d", d.x); end
endmodule
"#,
    );
    assert!(e.contains("class 'D' extends 'nope_base'"), "{e}");
    assert!(e.contains(":2:17"), "{e}");
}

/// Every way a type name is legally declared stays clean.
#[test]
fn declared_types_are_accepted() {
    let src = r#"
package pk;
  class pc; int v = 3; endclass
  typedef int pk_t;
endpackage
interface my_if; logic a; endinterface
typedef class fwd;
class base #(type T = int); T item; endclass
class fwd extends base #(int);
  typedef enum {A, B} e_t;
  e_t e;
  class inner; int z; endclass
  inner in;
  covergroup cg; coverpoint e; endgroup
  function new(); cg = new(); in = new(); endfunction
  function fwd clone(); fwd f = new(); return f; endfunction
endclass
module tb;
  import pk::*;
  my_if u();
  virtual my_if vif;
  pc p; pk_t q; fwd f; mailbox #(int) mb; semaphore s; process pr; event ev;
  covergroup cg2 @(ev); coverpoint q; endgroup
  cg2 c2 = new();
  initial begin
    typedef logic [3:0] nib_t;
    nib_t n; pk::pc p2; std::mailbox #(int) m2;
    p = new(); f = new(); mb = new(); vif = u;
    for (int i = 0; i < 1; i++) begin fwd g = f.clone(); end
    $display("ok %0d", p.v);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("every type is declared");
    assert!(
        sim.output.iter().any(|o| o.message == "ok 3"),
        "{:?}",
        sim.output
    );
}
