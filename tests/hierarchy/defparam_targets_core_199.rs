//! §23.10.1, §6.20.1, §6.20.3: a `defparam` can set only an overridable data
//! parameter, wherever the child is defined. A body `parameter` of a module
//! with a parameter port list, a `localparam`, a type parameter and a
//! parameter of a generate block are not; with the child in another file each
//! was applied or dropped without a message (aionhw/xezim-core#199, reported
//! by chuanseng-ng). A `defparam` written in a generate block names its target
//! from the block, and was dropped. Expected values are the reference
//! simulator's: every rejected case is an error there too.

use std::path::PathBuf;
use std::process::Command;

fn run_files(name: &str, files: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("defparam_targets_core_199")
        .join(name);
    std::fs::create_dir_all(&d).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--no-cache", "-s", "t"]).args(args);
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

#[track_caller]
fn rejected(name: &str, files: &[(&str, &str)], needles: &[&str]) {
    let (ok, text) = run_files(name, files, &[]);
    assert!(!ok, "accepted:\n{text}");
    assert!(
        !text.lines().any(|l| l.starts_with("y=")),
        "simulated:\n{text}"
    );
    for needle in needles {
        assert!(text.contains(needle), "no '{needle}' in:\n{text}");
    }
}

#[track_caller]
fn runs(name: &str, files: &[(&str, &str)], args: &[&str], line: &str) {
    let (ok, text) = run_files(name, files, args);
    assert!(ok, "rejected:\n{text}");
    assert!(text.contains(line), "expected {line}:\n{text}");
}

/// `L` is local: the module has a parameter port list (§6.20.1).
const BODY_LOCAL: &str = "module c #(parameter int A = 0) (output int y);
  parameter int L = 1;
  assign y = L;
endmodule
";
const PARENT: &str = "module t;
  int y;
  c u (y);
  defparam u.L = 9;
  initial begin #1 $display(\"y=%0d\", y); $finish; end
endmodule
";

#[test]
fn body_parameter_under_a_port_list() {
    rejected(
        "body",
        &[("c.sv", BODY_LOCAL), ("t.sv", PARENT)],
        &[
            "t.sv:4:12",
            "defparam cannot override 'L'",
            "is local (§6.20.1)",
        ],
    );
    // `--no-strict` keeps the old acceptance.
    runs(
        "body_no_strict",
        &[("c.sv", BODY_LOCAL), ("t.sv", PARENT)],
        &["--no-strict"],
        "y=9",
    );
}

#[test]
fn localparam_header_localparam_and_interface() {
    let local = "module c (output int y); localparam int L = 1; assign y = L; endmodule\n";
    rejected(
        "localparam",
        &[("c.sv", local), ("t.sv", PARENT)],
        &["defparam cannot override 'L' of module 'c'"],
    );
    let header = "module c #(parameter int A = 0, localparam int L = 1) (output int y);
  assign y = L + A;
endmodule
";
    rejected("header", &[("c.sv", header), ("t.sv", PARENT)], &["'L'"]);
    let ifc = "interface c #(parameter int A = 0) (); localparam int L = 1; endinterface\n";
    let t = "module t; c u (); defparam u.L = 9; endmodule\n";
    rejected(
        "interface",
        &[("c.sv", ifc), ("t.sv", t)],
        &["of interface 'c'"],
    );
}

#[test]
fn type_parameter() {
    let child =
        "module c #(parameter type T = int) (output int y); assign y = $bits(T); endmodule\n";
    let t = PARENT.replace("u.L = 9", "u.T = 8");
    rejected(
        "type",
        &[("c.sv", child), ("t.sv", &t)],
        &["defparam cannot override type parameter 'T'"],
    );
}

#[test]
fn deeper_paths_and_instance_arrays() {
    let mid = "module m (output int y); c u (y); endmodule\n";
    let t = "module t;
  int y;
  m k (y);
  defparam k.u.L = 9;
  initial begin #1 $display(\"y=%0d\", y); $finish; end
endmodule
";
    rejected(
        "deeper",
        &[("c.sv", BODY_LOCAL), ("m.sv", mid), ("t.sv", t)],
        &["t.sv:4:12", "'L'"],
    );
    let arr = "module t;
  int y [2];
  c u [1:0] (y);
  defparam u[0].L = 9;
  initial begin #1 $display(\"y=%0d\", y[0]); $finish; end
endmodule
";
    rejected("array", &[("c.sv", BODY_LOCAL), ("t.sv", arr)], &["'L'"]);
}

#[test]
fn header_parameter_is_still_set() {
    let child = "module c #(parameter int A = 0) (output int y);
  parameter int L = 1;
  assign y = A + L;
endmodule
";
    let t = PARENT.replace("u.L = 9", "u.A = 7");
    runs("header_ok", &[("c.sv", child), ("t.sv", &t)], &[], "y=8");
}

/// §23.10.1: a `defparam` in a generate block names its targets from the
/// block. The reference simulator prints `y=9 y2=4 y3=6`.
#[test]
fn defparam_in_a_generate_block() {
    let child = "module c #(parameter int A = 0) (output int y);
  parameter int L = 1;
  assign y = A;
endmodule
";
    let t = "module t;
  int y, y2, y3;
  if (1) begin : gb
    c u (y);
    defparam u.A = 9;
  end
  for (genvar i = 0; i < 1; i++) begin : gl
    c u (y2);
    defparam u.A = 4 + i;
  end
  if (1) begin : outer
    if (1) begin : inner
      c u (y3);
    end
    defparam inner.u.A = 6;
  end
  initial begin #1 $display(\"y=%0d y2=%0d y3=%0d\", y, y2, y3); $finish; end
endmodule
";
    runs(
        "gen_applied",
        &[("c.sv", child), ("t.sv", t)],
        &[],
        "y=9 y2=4 y3=6",
    );
    let local = "module t;
  int y;
  if (1) begin : gb
    c u (y);
    defparam u.L = 9;
  end
  initial begin #1 $display(\"y=%0d\", y); $finish; end
endmodule
";
    rejected(
        "gen_local",
        &[("c.sv", child), ("t.sv", local)],
        &["t.sv:5:14", "'L'"],
    );
}

/// §6.20.1: a parameter declared in a generate block is local.
#[test]
fn parameter_of_a_child_generate_block() {
    let child = "module c #(parameter int A = 0) (output int y);
  if (1) begin : g
    parameter int P = 5;
    assign y = P;
  end
endmodule
";
    let t = PARENT.replace("u.L = 9", "u.g.P = 9");
    rejected(
        "gen_child",
        &[("c.sv", child), ("t.sv", &t)],
        &[
            "t.sv:4:12",
            "declared in a generate block is a local parameter",
        ],
    );
}
