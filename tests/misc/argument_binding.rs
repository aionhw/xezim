//! §13.5: argument binding — an argument past the last formal, or a formal
//! left without an actual and without a default, is an error; defaults and
//! named arguments still bind. Each rejected case is also rejected by the
//! reference simulator.

use xezim::simulate;

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(_) => panic!("accepted an illegal design:\n{src}"),
        Err(e) => e,
    }
}

#[test]
fn arguments_bind_to_formals() {
    let e = rejected(
        "module top; function int f(int a); return a; endfunction\n\
         initial $display(f(1, 2)); endmodule",
    );
    assert!(e.contains("too many arguments"), "{e}");
    rejected(
        "module top; function int f(int a = 1, int b = 2); return a; endfunction\n\
         initial $display(f( , , )); endmodule",
    );
    let e = rejected(
        "module top; task t(input int a, output int b); b = a; endtask\n\
         initial t(1); endmodule",
    );
    assert!(e.contains("no actual argument"), "{e}");
}

#[test]
fn defaults_and_named_arguments_bind() {
    let src = r#"
module top;
  function automatic int sum(int a, int b = 2, int c = 3);
    return a + b + c;
  endfunction
  task automatic t(input int a, output int b, input int c = 4);
    b = a + c;
  endtask
  int i, o;
  initial begin
    i = sum(1) + sum(1, 1) + sum(.a(1), .c(1)) + sum(1, , 0);
    t(1, o);
    $display("A|%0d %0d", i, o);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("legal calls must run");
    assert!(
        sim.output.iter().any(|o| o.message == "A|18 5"),
        "{:?}",
        sim.output
    );
}

/// §8.7, §13.5: a constructor call binds its actuals to the class's own
/// `new`, as any other call does; a class that declares none has the
/// implicit `new()` with no formals (#812). Each rejected case is also
/// rejected by the reference simulator.
#[test]
fn constructor_arguments_bind_to_formals() {
    let classes = "class E; int x; endclass\n\
        class B; function new(int a); endfunction endclass\n\
        class D extends B; function new(); super.new(1); endfunction endclass\n\
        class F extends B; function new(int a, int b = 2); super.new(a); endfunction endclass\n";
    for (stmt, msg) in [
        (
            "E e; e = new(1);",
            "too many arguments to 'E::new': it has 0",
        ),
        (
            "D d; d = new(5);",
            "too many arguments to 'D::new': it has 0",
        ),
        (
            "F f; f = new();",
            "no actual argument for formal 'a' of 'F::new'",
        ),
        (
            "F f; f = new(.b(3));",
            "no actual argument for formal 'a' of 'F::new'",
        ),
        (
            "B b; b = F::new(1, 2, 3);",
            "too many arguments to 'F::new': it has 2",
        ),
        (
            "B b; b = new;",
            "no actual argument for formal 'a' of 'B::new'",
        ),
        (
            "B b = new(1, 2);",
            "too many arguments to 'B::new': it has 1",
        ),
    ] {
        let src = format!("{classes}module top;\n  initial begin\n    {stmt}\n  end\nendmodule\n");
        let e = rejected(&src);
        assert!(e.contains(msg), "{stmt}: {e}");
    }
    let e = rejected(
        "class P; function new(int a); endfunction endclass\n\
         class H; P p = new(); endclass\n\
         module top; H h = new; endmodule",
    );
    assert!(
        e.contains("no actual argument for formal 'a' of 'P::new'"),
        "{e}"
    );
}

/// Defaults, named arguments, `C::new`, the implicit constructor and a
/// parameterized class's default all bind; expected values are the reference
/// simulator's.
#[test]
fn constructor_defaults_and_named_arguments_bind() {
    let src = r#"
class B; int v; function new(int a); v = a; endfunction endclass
class F extends B;
  int w;
  function new(int a, int b = 2); super.new(a); w = b; endfunction
endclass
class E; int x = 7; endclass
class G #(int W = 8); int v; function new(int a = W); v = a; endfunction endclass
module top;
  initial begin
    F f1 = new(1);
    F f2 = new(.b(3), .a(4));
    B b = F::new(5, 6);
    E e = new;
    G #(4) g = new();
    $display("A|%0d %0d %0d %0d %0d %0d %0d", f1.v, f1.w, f2.v, f2.w, b.v, e.x, g.v);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("legal constructor calls");
    assert!(
        sim.output.iter().any(|o| o.message == "A|1 2 4 3 5 7 4"),
        "{:?}",
        sim.output
    );
}
