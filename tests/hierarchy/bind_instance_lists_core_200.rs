//! §23.11: the bind instantiation is a complete module, interface, program or
//! checker instantiation, so it can list several instances, and each one is
//! bound into every target with the directive's parameters and connections.
//! Only the first was read and the comma dropped the whole directive, so the
//! bound module became an extra top (aionhw/xezim-core#200, reported by
//! chuanseng-ng). Expected values follow the LRM: the reference simulator
//! binds only the first instance of a list (and rejects the colon form with a
//! list as unsupported), while each single-instance case matches it.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    let mut out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    out.sort();
    out
}

const SHOW: &str = "module c #(parameter int A = 0) ();
  initial #1 $display(\"%m A=%0d\", A);
endmodule
";

#[test]
fn unit_scope_list() {
    let src = format!("{SHOW}module t; initial #2 $finish; endmodule\nbind t c #(3) a(), b();\n");
    assert_eq!(lines(&src), ["t.a A=3", "t.b A=3"]);
}

#[test]
fn module_scope_list() {
    let src = format!(
        "{SHOW}module t; endmodule\nmodule top; t u (); bind t c #(5) a(), b(); endmodule\n"
    );
    assert_eq!(lines(&src), ["top.u.a A=5", "top.u.b A=5"]);
}

#[test]
fn instance_path_and_colon_form_lists() {
    let src = format!(
        "{SHOW}module m; endmodule\nmodule top; m u1 (); m u2 (); m u3 (); endmodule\n\
         bind top.u1 c #(4) a(), b();\nbind m : top.u2, top.u3 c #(6) x(), y();\n"
    );
    assert_eq!(
        lines(&src),
        [
            "top.u1.a A=4",
            "top.u1.b A=4",
            "top.u2.x A=6",
            "top.u2.y A=6",
            "top.u3.x A=6",
            "top.u3.y A=6",
        ]
    );
}

/// Named parameters, port connections and an array of instances in one list;
/// each element of `d` is bound too (the reference simulator gives
/// `d[0].y=15 d[1].y=15` for the array alone).
#[test]
fn connections_and_an_instance_array_in_a_list() {
    let src = "module c #(parameter int A = 0) (input int x, output int y);
  assign y = A + x;
endmodule
module t;
  int r1 = 10, r2 = 20;
  initial #1 $display(\"a.y=%0d b.y=%0d d[0].y=%0d d[1].y=%0d\", top.u.a.y, top.u.b.y, top.u.d[0].y, top.u.d[1].y);
endmodule
module top;
  t u ();
endmodule
bind t c #(.A(5)) a(.x(r1), .y()), b(.x(r2), .y()), d[1:0](.x(r1), .y());
";
    assert_eq!(lines(src), ["a.y=15 b.y=25 d[0].y=15 d[1].y=15"]);
}

#[test]
fn interface_list_and_nested_binds() {
    let src = "interface ifc #(parameter int A = 0) ();
  int v = A;
endinterface
module t;
endmodule
module top;
  t u ();
  initial #1 $display(\"i1.v=%0d i2.v=%0d\", u.i1.v, u.i2.v);
endmodule
bind t ifc #(6) i1(), i2();
";
    assert_eq!(lines(src), ["i1.v=6 i2.v=6"]);
    let src = "module d (); initial #1 $display(\"%m d\"); endmodule
module c (); initial #1 $display(\"%m c\"); endmodule
module t; endmodule
module top; t u (); endmodule
bind t c a(), b();
bind c d k1(), k2();
";
    assert_eq!(
        lines(src),
        [
            "top.u.a c",
            "top.u.a.k1 d",
            "top.u.a.k2 d",
            "top.u.b c",
            "top.u.b.k1 d",
            "top.u.b.k2 d",
        ]
    );
}

/// A bind directive the parser cannot read is reported, not dropped.
#[test]
fn unreadable_bind_is_an_error() {
    let src = "module c (); endmodule\nmodule t; endmodule\nbind t c a() b();\n";
    let err = simulate(src, 10).err().expect("must be rejected");
    assert!(err.contains("bind directive not understood"), "{err}");
}
