//! A parameter override list on an instance must fit the instantiated
//! definition (§23.10.2): an ordered value needs a parameter to bind to, and
//! a name appears once and names an overridable parameter. Such lists were
//! accepted without a diagnostic:
//! an excess value was dropped or landed on a local parameter, and the last
//! of two values for one name won (#818).
//!
//! The check runs as each instance is built, so it reaches children in other
//! files, generate blocks and bound instances, and leaves alone what is not
//! built. These tests run the binary: `--no-strict` is process-wide state,
//! and the issue asks for a non-zero exit.

use std::path::PathBuf;
use std::process::Command;

fn run_files(name: &str, files: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("param_override_lists")
        .join(name);
    std::fs::create_dir_all(&d).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(args);
    for (file, src) in files {
        let path = d.join(file);
        std::fs::write(&path, src).unwrap();
        cmd.arg(path);
    }
    let output = cmd.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

const CHILD: &str = "module c #(parameter int A = 0, parameter int B = 0) (output int y); assign y = A + B; endmodule\n";
/// One overridable parameter and one local one (§6.20.1).
const LOCAL: &str = "module h #(parameter int A = 0) (output int y);
  parameter int B = 3;
  assign y = A + B;
endmodule
";
const SHOW: &str = "  initial begin #1; $display(\"y=%0d\", y); $finish; end\nendmodule\n";

fn top(body: &str) -> String {
    format!("module t;\n  int y;\n  {body}\n{SHOW}")
}

#[track_caller]
fn rejected(name: &str, files: &[(&str, &str)], needle: &str) {
    let (ok, text) = run_files(name, files, &["-s", "t"]);
    assert!(!ok, "accepted:\n{text}");
    assert!(!text.contains("y="), "simulated:\n{text}");
    assert!(text.contains(needle), "no '{needle}' in:\n{text}");
}

#[track_caller]
fn accepted(name: &str, files: &[(&str, &str)], y: &str) {
    let (ok, text) = run_files(name, files, &["-s", "t"]);
    assert!(ok, "rejected:\n{text}");
    assert!(text.contains(&format!("y={y}")), "expected y={y}:\n{text}");
}

#[test]
fn excess_ordered_values() {
    let one = "module c #(parameter int A = 0) (output int y); assign y = A; endmodule\n";
    let t = top("c #(1, 2) u (.y(y));");
    rejected(
        "excess_header",
        &[("t.sv", &format!("{one}{t}"))],
        "has 2 ordered",
    );
    let none = "module c (output int y); assign y = 7; endmodule\n";
    let t = top("c #(1) u (.y(y));");
    rejected(
        "excess_none",
        &[("t.sv", &format!("{none}{t}"))],
        "accepts 0",
    );
    let body = "module c (output int y); parameter A = 0; assign y = A; endmodule\n";
    let t = top("c #(1, 2) u (.y(y));");
    rejected(
        "excess_body",
        &[("t.sv", &format!("{body}{t}"))],
        "accepts 1",
    );
    let lp =
        "module c (output int y); parameter A = 0; localparam L = 3; assign y = A + L; endmodule\n";
    rejected(
        "excess_localparam",
        &[("t.sv", &format!("{lp}{t}"))],
        "accepts 1",
    );
}

/// §6.20.1: under a parameter port list, a body `parameter` is local, so it
/// takes no ordered value. It used to take the second one.
#[test]
fn excess_ordered_value_does_not_reach_a_local_parameter() {
    let t = top("h #(1, 2) u (.y(y));");
    rejected(
        "excess_local",
        &[("t.sv", &format!("{LOCAL}{t}"))],
        "accepts 1",
    );
    let hl = "module h #(parameter int A = 0, localparam int L = 9) (output int y); assign y = A + L; endmodule\n";
    rejected(
        "excess_header_local",
        &[("t.sv", &format!("{hl}{t}"))],
        "accepts 1",
    );
}

#[test]
fn a_name_given_twice() {
    let t = top("c #(.A(1), .A(2)) u (.y(y));");
    rejected("dup", &[("t.sv", &format!("{CHILD}{t}"))], "more than once");
    let t = top("c #(.A(), .A(2)) u (.y(y));");
    rejected(
        "dup_empty",
        &[("t.sv", &format!("{CHILD}{t}"))],
        "more than once",
    );
}

/// A child in another file: the per-file strict pass cannot see it.
#[test]
fn child_in_another_file() {
    let t = top("c #(1, 2, 3) u (.y(y));");
    rejected(
        "cross_excess",
        &[("c.sv", CHILD), ("t.sv", &t)],
        "accepts 2",
    );
    let t = top("c #(.A(1), .A(2)) u (.y(y));");
    rejected(
        "cross_dup",
        &[("c.sv", CHILD), ("t.sv", &t)],
        "more than once",
    );
    let t = top("c #(.Z(1)) u (.y(y));");
    rejected("cross_bad_name", &[("c.sv", CHILD), ("t.sv", &t)], "'Z'");
    let t = top("h #(.B(9)) u (.y(y));");
    rejected(
        "cross_local_name",
        &[("h.sv", LOCAL), ("t.sv", &t)],
        "is local",
    );
}

/// A bad name beside its module fails too, whichever check reports it.
#[test]
fn bad_name_in_the_same_file() {
    let t = top("c #(.Z(1)) u (.y(y));");
    rejected("same_bad_name", &[("t.sv", &format!("{CHILD}{t}"))], "'Z'");
}

#[test]
fn inside_generate_blocks() {
    let t = top("if (1) begin : g c #(1, 2, 3) u (.y(y)); end");
    rejected("gen_if", &[("t.sv", &format!("{CHILD}{t}"))], "accepts 2");
    let t = top("for (genvar i = 0; i < 1; i++) begin : g c #(.A(1), .A(2)) u (.y(y)); end");
    rejected(
        "gen_for",
        &[("t.sv", &format!("{CHILD}{t}"))],
        "more than once",
    );
    let t = top("case (1) 1: begin : g c #(1, 2, 3) u (.y(y)); end default: ; endcase");
    rejected("gen_case", &[("t.sv", &format!("{CHILD}{t}"))], "accepts 2");
}

/// §27: a branch a constant rules out is not elaborated.
#[test]
fn an_untaken_generate_branch_is_not_built() {
    let t =
        top("if (0) begin : g c #(1, 2, 3) u (.y(y)); end else begin : e c #(1, 2) u (.y(y)); end");
    accepted("gen_untaken", &[("t.sv", &format!("{CHILD}{t}"))], "3");
}

#[test]
fn interface_targets_and_parents() {
    let i = "interface i #(parameter int A = 0); int v = A; endinterface\n";
    let t = "module t; i #(1, 2) u (); endmodule\n";
    rejected(
        "iface_excess",
        &[("t.sv", &format!("{i}{t}"))],
        "of interface 'i'",
    );
    let t = "module t; i #(.A(1), .A(2)) u (); endmodule\n";
    rejected(
        "iface_dup",
        &[("t.sv", &format!("{i}{t}"))],
        "more than once",
    );
    let parent = "interface p; int y; c #(1, 2, 3) u (.y(y)); endinterface
module t; p n (); endmodule\n";
    rejected(
        "iface_parent",
        &[("t.sv", &format!("{CHILD}{parent}"))],
        "accepts 2",
    );
    let parent = "program p; int y; c #(1, 2, 3) u (.y(y)); endprogram
module t; p n (); endmodule\n";
    rejected(
        "program_parent",
        &[("t.sv", &format!("{CHILD}{parent}"))],
        "accepts 2",
    );
}

/// §23.11: a bound instance is built in its target, and its directive may
/// be in another file, so its error carries no location but names the bind.
#[test]
fn a_bound_instance() {
    let d = "module d; int y; endmodule\n";
    let b = "bind d c #(1, 2, 3) p (.y(y));\n";
    let t = "module t; d u (); int y; assign y = u.y; initial begin #1; $display(\"y=%0d\", y); $finish; end endmodule\n";
    rejected(
        "bind_unit",
        &[("c.sv", CHILD), ("d.sv", d), ("b.sv", b), ("t.sv", t)],
        "bound into 'd'",
    );
    let scoped = "module t; bind d c #(.A(1), .A(2)) p (.y(y)); d u (); endmodule\n";
    rejected(
        "bind_module",
        &[("c.sv", CHILD), ("d.sv", d), ("t.sv", scoped)],
        "more than once",
    );
    let good = "bind d c #(1, 2) p (.y(y));\n";
    accepted(
        "bind_legal",
        &[("c.sv", CHILD), ("d.sv", d), ("b.sv", good), ("t.sv", t)],
        "3",
    );
}

/// An unused module is built only when it becomes a top.
#[test]
fn an_unused_module() {
    let unused = "module unused; int y; c #(1, 2, 3) u (.y(y)); endmodule\n";
    let t = top("c u (.y(y));");
    let src = format!("{CHILD}{unused}{t}");
    accepted("unused_named_top", &[("t.sv", &src)], "0");
    let (ok, text) = run_files("unused_auto_top", &[("t.sv", &src)], &[]);
    assert!(!ok, "accepted:\n{text}");
    assert!(text.contains("accepts 2"), "{text}");
}

#[test]
fn legal_lists_stay_legal() {
    let body =
        "module c (output int y); parameter A = 0; parameter B = 0; assign y = A + B; endmodule\n";
    let t = top("c #(1, 2) u (.y(y));");
    accepted("ordered_body", &[("t.sv", &format!("{body}{t}"))], "3");
    let body = "module c (output int y); parameter A = 0, B = 0; assign y = A + B; endmodule\n";
    accepted(
        "ordered_body_multi",
        &[("t.sv", &format!("{body}{t}"))],
        "3",
    );
    accepted("ordered_header", &[("t.sv", &format!("{CHILD}{t}"))], "3");
    let t2 = top("c #(1) u (.y(y));");
    accepted("ordered_fewer", &[("t.sv", &format!("{CHILD}{t2}"))], "1");
    let t2 = top("c #(.B(2), .A(1)) u (.y(y));");
    accepted("named_any_order", &[("t.sv", &format!("{CHILD}{t2}"))], "3");
    let t2 = top("c #(.A(), .B(2)) u (.y(y));");
    accepted("named_empty", &[("t.sv", &format!("{CHILD}{t2}"))], "2");
    let none = "module c (output int y); assign y = 7; endmodule\n";
    let t2 = top("c #() u (.y(y));");
    accepted("empty_list", &[("t.sv", &format!("{none}{t2}"))], "7");
    let typed = "module c #(parameter type T = int, parameter int A = 2) (output int y);
  T v = 3;
  assign y = v + A;
endmodule\n";
    let t2 = top("c #(byte, 2) u (.y(y));");
    accepted("type_and_value", &[("t.sv", &format!("{typed}{t2}"))], "5");
    let arr = top("int ys[2]; c #(1, 2) u [1:0] (.y(ys)); assign y = ys[0];");
    accepted("instance_array", &[("t.sv", &format!("{CHILD}{arr}"))], "3");
}

/// §29: the list on a UDP instance holds delays, not parameter values.
#[test]
fn udp_delays_are_not_parameters() {
    let src = "primitive p (output o, input a);
  table 0 : 0; 1 : 1; endtable
endprimitive
module t;
  wire o;
  reg a = 1;
  p #(1, 2) u (o, a);
  initial begin #5; $display(\"y=%b\", o); $finish; end
endmodule\n";
    accepted("udp", &[("t.sv", src)], "1");
}

/// Nested modules of one name share a definition table in this
/// implementation, so a list cannot be judged against the survivor (§23.4).
#[test]
fn a_redefined_name_is_not_judged() {
    let src = "module a (output int y);
  module n #(parameter int X = 0) (output int y); assign y = X; endmodule
  n #(7) u (.y(y));
endmodule
module b; module n; endmodule n v (); endmodule
module t; int y; a i (.y(y)); b j ();
  initial begin #1; $display(\"y=%0d\", y); $finish; end
endmodule\n";
    let (ok, text) = run_files("redefined", &[("t.sv", src)], &["-s", "t"]);
    assert!(ok, "rejected:\n{text}");
}

#[test]
fn the_error_names_the_line_of_the_list() {
    let t = "module t;\n  int y;\n\n  c #(1, 2, 3) u (.y(y));\nendmodule\n";
    let (ok, text) = run_files("location", &[("c.sv", CHILD), ("t.sv", t)], &["-s", "t"]);
    assert!(!ok, "accepted:\n{text}");
    assert!(text.contains("t.sv:4:"), "{text}");
}

#[test]
fn compile_mode_rejects_too() {
    let t = top("c #(1, 2, 3) u (.y(y));");
    let src = format!("{CHILD}{t}");
    let (ok, text) = run_files("compile", &[("t.sv", &src)], &["--compile", "-s", "t"]);
    assert!(!ok, "accepted:\n{text}");
    assert!(text.contains("accepts 2"), "{text}");
}

/// `--no-strict` keeps the old leniency, as for the other strict checks.
#[test]
fn no_strict_accepts_them() {
    let t = top("c #(1, 2, 3) u (.y(y));");
    let src = format!("{CHILD}{t}");
    let (ok, text) = run_files(
        "lenient_excess",
        &[("t.sv", &src)],
        &["--no-strict", "-s", "t"],
    );
    assert!(ok, "rejected:\n{text}");
    assert!(text.contains("y=3"), "{text}");
    let t = top("c #(.A(1), .A(2)) u (.y(y));");
    let src = format!("{CHILD}{t}");
    let (ok, text) = run_files(
        "lenient_dup",
        &[("t.sv", &src)],
        &["--no-strict", "-s", "t"],
    );
    assert!(ok, "rejected:\n{text}");
    assert!(text.contains("y=2"), "{text}");
}

const MIXED: &str = "cannot mix ordered and named";

/// A.4.1.1: a parameter value list is all ordered or all named.
#[test]
fn a_list_mixing_ordered_and_named_entries() {
    for (name, list) in [
        ("mixed_ordered_first", "#(1, .B(2))"),
        ("mixed_named_first", "#(.A(1), 2)"),
    ] {
        let t = top(&format!("c {list} u (.y(y));"));
        let src = format!("{CHILD}{t}");
        rejected(name, &[("t.sv", &src)], MIXED);
    }
}

#[test]
fn a_mixed_class_specialization() {
    let class = "class k #(parameter int A = 0, parameter int B = 0);
  static int v = A + B;
endclass
";
    for (name, list) in [
        ("mixed_class_ordered_first", "#(1, .B(2))"),
        ("mixed_class_named_first", "#(.A(1), 2)"),
    ] {
        let t = top(&format!("initial y = k{list}::v;"));
        let src = format!("{class}{t}");
        rejected(name, &[("t.sv", &src)], MIXED);
    }
}

/// The form of a list is a rule of the grammar, so unlike the checks above it
/// also applies to what is never built.
#[test]
fn a_mixed_list_is_rejected_even_if_never_built() {
    let unused =
        format!("{CHILD}module never_built;\n  int y;\n  c #(1, .B(2)) u (.y(y));\nendmodule\n");
    let t = top("");
    rejected(
        "mixed_unused_module",
        &[("c.sv", &unused), ("t.sv", &t)],
        MIXED,
    );
    let t = top("if (0) begin c #(1, .B(2)) u (.y(y)); end");
    let src = format!("{CHILD}{t}");
    rejected("mixed_untaken_branch", &[("t.sv", &src)], MIXED);
}

#[test]
fn compile_mode_rejects_a_mixed_list() {
    let t = top("c #(1, .B(2)) u (.y(y));");
    let src = format!("{CHILD}{t}");
    let (ok, text) = run_files(
        "mixed_compile",
        &[("t.sv", &src)],
        &["--compile", "-s", "t"],
    );
    assert!(!ok, "accepted:\n{text}");
    assert!(text.contains(MIXED), "{text}");
}

#[test]
fn no_strict_accepts_a_mixed_list() {
    for (name, list) in [
        ("lenient_mixed_ordered_first", "#(1, .B(2))"),
        ("lenient_mixed_named_first", "#(.A(1), 2)"),
    ] {
        let t = top(&format!("c {list} u (.y(y));"));
        let src = format!("{CHILD}{t}");
        let (ok, text) = run_files(name, &[("t.sv", &src)], &["--no-strict", "-s", "t"]);
        assert!(ok, "rejected:\n{text}");
        assert!(text.contains("y=3"), "{text}");
    }
}
