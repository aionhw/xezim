//! §6.20.2, §8.25, §5.7: a name declared nowhere in the design is an error in
//! a class parameter-override list (nested ones too), a class value-parameter
//! default and a class property or package variable initializer (#888). The
//! nested lists are kept as source text, so the literals and system calls in
//! them must not be read as names: `8'hFF`, `$clog2(16)`, `1.5e3` and `10ns`
//! are legal actuals. Both halves are cross-checked against the reference
//! simulator.

const CLASSES: &str = "
class A #(int W = 8); endclass
class R #(type T = int); endclass
class B #(int W = 8); endclass
";

fn sim(body: &str) -> Result<Vec<String>, String> {
    xezim::simulate(&format!("{CLASSES}{body}"), 1_000_000).map(|r| {
        r.output
            .iter()
            .map(|o| o.message.trim().to_string())
            .collect()
    })
}

#[test]
fn undeclared_names_are_rejected() {
    let cases = [
        (
            "bad_nested",
            "module top; R #(A #(bad_nested)) x; endmodule",
        ),
        ("bad_value", "module top; A #(bad_value) x; endmodule"),
        (
            "bad_typedef",
            "typedef R #(A #(bad_typedef)) t;\nmodule top; endmodule",
        ),
        (
            "bad_default",
            "class D #(int W = bad_default); endclass\nmodule top; endmodule",
        ),
        (
            "bad_extends",
            "class E extends B #(bad_extends); endclass\nmodule top; endmodule",
        ),
        (
            "bad_prop",
            "class F; int m = bad_prop; endclass\nmodule top; endmodule",
        ),
        (
            "bad_pkg",
            "package pk; int v = bad_pkg; endpackage\nmodule top; endmodule",
        ),
    ];
    for (name, body) in cases {
        match sim(body) {
            Ok(out) => panic!("{name}: accepted: {out:?}"),
            Err(e) => assert!(
                e.contains(&format!("undeclared identifier '{name}'")),
                "{name}: {e}"
            ),
        }
    }
}

#[test]
fn literals_and_system_calls_in_nested_overrides_are_not_names() {
    let out = sim(r#"
class T #(realtime D = 1ns); endclass
interface bus_if #(int W = 8); logic [W-1:0] d; endinterface
module top;
  localparam int N = 3;
  R #(A #(8'hFF)) x1;
  R #(A #(4'b1010)) x2;
  R #(A #(16'd5)) x3;
  R #(A #(8'h FF)) x4;
  R #(A #('sd7)) x5;
  R #(A #($clog2(16))) x6;
  R #(A #($bits(byte))) x7;
  R #(A #(1.5e3)) x8;
  R #(T #(10ns)) x9;
  R #(A #(N + 1)) x10;
  R #(R #(virtual bus_if #(4))) x11;
  bus_if #(4) b();
  initial $display("ok");
endmodule
"#)
    .expect("legal overrides rejected");
    assert!(out.iter().any(|l| l == "ok"), "{out:?}");
}
