//! §3.13, §8.3: a scope's name space holds one declaration per name, so a
//! second class of the same name in the compilation unit, a package, a
//! module or a class is illegal. The name-keyed class tables used to keep
//! one of the two definitions (the later one in the compilation unit, the
//! earlier one in a package or module) and run the design without a word.

use xezim::simulate;

fn error_of(src: &str) -> String {
    match simulate(src, 10) {
        Ok(sim) => panic!("accepted; output {:?}", sim.output),
        Err(e) => e,
    }
}

fn simulate_files(sources: &[&str], paths: &[&str]) -> Result<xezim::compiler::Simulator, String> {
    let sources: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
    let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
    xezim::simulate_multi(
        &sources,
        10,
        Some("tb"),
        &[],
        &paths,
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
}

#[test]
fn compilation_unit() {
    let e = error_of(
        r#"
class policy; int a = 1; endclass
class policy; int b = 2; endclass
module tb;
  initial begin
    policy p = new();
    $display("b=%0d", p.b);
  end
endmodule
"#,
    );
    assert!(
        e.contains("duplicate definition of class 'policy' in the compilation unit"),
        "{e}"
    );
    // The second definition is the located one; the first is named.
    assert!(e.contains(":3:7: error:"), "{e}");
    assert!(
        e.contains("already defined at") && e.contains(":2:7"),
        "{e}"
    );
}

/// All source files share one compilation unit.
#[test]
fn compilation_unit_across_files() {
    let e = match simulate_files(
        &[
            "class policy; int a = 1; endclass\n",
            "class policy; int b = 2; endclass\nmodule tb; policy p; initial p = new(); endmodule\n",
        ],
        &["first.sv", "second.sv"],
    ) {
        Ok(sim) => panic!("accepted; output {:?}", sim.output),
        Err(e) => e,
    };
    assert!(e.contains("second.sv:1:7: error:"), "{e}");
    assert!(e.contains("already defined at first.sv:1:7"), "{e}");
}

#[test]
fn package() {
    let e = error_of(
        r#"
package p;
  class k; int a; endclass
  class k; int b; endclass
endpackage
module tb;
  initial begin p::k h = new(); $display("b=%0d", h.b); end
endmodule
"#,
    );
    assert!(
        e.contains("duplicate definition of class 'k' in package 'p'"),
        "{e}"
    );
    assert!(e.contains(":4:9: error:"), "{e}");
}

#[test]
fn module() {
    let e = error_of(
        r#"
module tb;
  class k; int a; endclass
  class k; int b; endclass
  initial begin k h = new(); $display("b=%0d", h.b); end
endmodule
"#,
    );
    assert!(
        e.contains("duplicate definition of class 'k' in module 'tb'"),
        "{e}"
    );
}

#[test]
fn nested_class() {
    let e = error_of(
        r#"
class outer;
  class k; int a; endclass
  class k; int b; endclass
endclass
module tb;
  outer o;
  initial begin o = new(); $display("done"); end
endmodule
"#,
    );
    assert!(
        e.contains("duplicate definition of class 'k' in class 'outer'"),
        "{e}"
    );
}

/// The same name in different scopes is legal (§3.13): a class beside a
/// module, a forward typedef, classes of two packages or two modules, a
/// nested class named like a compilation-unit one, and the alternatives of
/// a conditional generate.
#[test]
fn same_name_in_different_scopes_is_legal() {
    let src = r#"
typedef class k;
class k; int a = 1; endclass
module k; endmodule
package p1; class c; int v = 10; endclass endpackage
package p2; class c; int v = 20; endclass endpackage
class outer;
  class k; int a = 2; endclass
  k in = new();
endclass
module other;
  class loc; int v = 5; endclass
endmodule
module tb;
  class loc; int v = 6; endclass
  if (1) begin : g
    class gc; int v = 7; endclass
    gc h = new();
  end else begin : g
    class gc; int v = 8; endclass
    gc h = new();
  end
  initial begin
    k x = new();
    outer o = new();
    p1::c a = new();
    p2::c b = new();
    loc l = new();
    $display("R|%0d %0d %0d %0d %0d %0d", x.a, o.in.a, a.v, b.v, l.v, g.h.v);
  end
endmodule
"#;
    let sim = simulate_files(&[src], &[]).expect("no scope declares a class twice");
    assert!(
        sim.output.iter().any(|o| o.message == "R|1 2 10 20 6 7"),
        "{:?}",
        sim.output
    );
}
