//! IEEE 1800-2023 §22.8, §23.2.2.3 and §6.6.5: ANSI ports with an
//! inferred net kind use the active default, including tri0 and tri1.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        loop {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir()
                .join(format!("xezim_default_nettype_{}_{id}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => return Self(dir),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => panic!("create {}: {err}", dir.display()),
            }
        }
    }

    fn write(&self, name: &str, source: &str) {
        fs::write(self.0.join(name), source).expect("write SystemVerilog fixture");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check_run(dir: &Path, args: &[&str], expected: &[&str]) {
    let run = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .arg("--no-cache")
        .args(args)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success(), "{args:?}\n{stdout}\n{stderr}");
    let values: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("value="))
        .collect();
    assert_eq!(values, expected, "{args:?}\n{stdout}\n{stderr}");
}

fn check(source: &str, expected: &[&str]) {
    let dir = TestDir::new();
    dir.write("design.sv", source);
    check_run(&dir.0, &["design.sv"], expected);
}

#[test]
fn open_tri0_input_read_inside_the_child() {
    check(
        r#"`timescale 1ns/1ps
`default_nettype tri0
module child(input a);
  initial begin #1; $display("value=%b", a); $finish; end
endmodule
module top;
  child c();
endmodule
`default_nettype wire
"#,
        &["value=0"],
    );
}

/// An omitted connection and a named empty connection both leave an input
/// without an external driver (§23.3.2).
#[test]
fn open_and_named_empty_inputs() {
    check(
        r#"`timescale 1ns/1ps
`default_nettype tri0
module pull_down(input a); endmodule
`default_nettype tri1
module pull_up(input a); endmodule
`default_nettype wire
module top;
  pull_down lo(), lo_empty(.a());
  pull_up hi(), hi_empty(.a());
  initial begin
    #1; $display("value=%b%b%b%b", lo.a, lo_empty.a, hi.a, hi_empty.a);
    $finish;
  end
endmodule
"#,
        &["value=0011"],
    );
}

#[test]
fn inherited_and_four_state_input_types() {
    check(
        r#"`timescale 1ns/1ps
package widths;
  localparam int W = 3;
endpackage
`default_nettype tri1
module child #(parameter int P = 2)
  (input [P-1:0] a, b, input logic [widths::W-1:0] c, input integer d,
   input reg e, input struct packed { logic hi, lo; } f,
   input enum logic [1:0] {ZERO = 2'b00, ONE = 2'b11} g);
endmodule
`default_nettype wire
module top;
  child c();
  initial begin
    #1; $display("value=%b %b %b %h %b %b %b", c.a, c.b, c.c, c.d, c.e, c.f, c.g);
    $finish;
  end
endmodule
"#,
        &["value=11 11 111 ffffffff 1 11 11"],
    );
}

/// Open ports expose their own net values (§6.6.5, §23.2.2.3). Connecting
/// an output to a variable implies a continuous assignment (§6.5).
#[test]
fn open_inout_and_implicit_outputs() {
    check(
        r#"`timescale 1ns/1ps
`default_nettype tri1
module child(inout a, output y, output signed [1:0] v); endmodule
`default_nettype wire
module top;
  logic [1:0] out;
  child c(.a(), .y(), .v(out));
  initial begin
    #1; $display("value=%b %b %b %b", c.a, c.y, c.v, out); $finish;
  end
endmodule
"#,
        &["value=1 1 11 11"],
    );
}

/// Interface and program ANSI inputs use the default net kind (§24.3, §25.3).
#[test]
fn interface_and_program_inputs() {
    check(
        r#"`timescale 1ns/1ps
`default_nettype tri0
interface bus(input a); endinterface
`default_nettype tri1
program monitor(input a, input wire observed);
  initial begin
    #1; $display("value=%b%b", observed, a); $finish;
  end
endprogram
`default_nettype wire
module top;
  bus b();
  monitor m(.observed(b.a));
endmodule
"#,
        &["value=01"],
    );
}

/// An include can change the default for its own text and the enclosing source
/// (§22.2, §22.4, §22.8).
#[test]
fn include_changes_the_default_net_kind() {
    let dir = TestDir::new();
    dir.write(
        "pulled.svh",
        r#"`default_nettype tri1
module inside_include(input a); endmodule
"#,
    );
    dir.write(
        "design.sv",
        r#"`timescale 1ns/1ps
`default_nettype tri0
module before_include(input a); endmodule
`include "pulled.svh"
module after_include(input a); endmodule
`default_nettype wire
module top;
  before_include b();
  inside_include i();
  after_include a();
  initial begin
    #1; $display("value=%b%b%b", b.a, i.a, a.a); $finish;
  end
endmodule
"#,
    );
    check_run(&dir.0, &["design.sv"], &["value=011"]);
}

/// Directive scope extends across files in a compilation unit (§22.2).
#[test]
fn default_net_kind_crosses_source_files() {
    let dir = TestDir::new();
    dir.write(
        "first.sv",
        r#"`timescale 1ns/1ps
`default_nettype tri0
module first_child(input a); endmodule
"#,
    );
    dir.write(
        "second.sv",
        r#"`timescale 1ns/1ps
module second_child(input a); endmodule
`default_nettype wire
module top;
  first_child f();
  second_child s();
  initial begin #1; $display("value=%b%b", f.a, s.a); $finish; end
endmodule
"#,
    );
    check_run(&dir.0, &["first.sv", "second.sv"], &["value=00"]);
}

/// A directive whose end is defined can share a line with the next one (§22.2).
#[test]
fn starts_pull_on_the_same_line_as_timescale() {
    check(
        r#"`timescale 1ns/1ps `default_nettype tri1
module child(input a); endmodule
`default_nettype wire
module top;
  child c();
  initial begin #1; $display("value=%b", c.a); $finish; end
endmodule
"#,
        &["value=1"],
    );
}

/// Classes, subroutines, covergroups and typedefs at compilation-unit scope
/// are outside the design elements listed in §3.2 and §22.8. Sampling one
/// of two bins gives 50 percent instance coverage (§19.8).
#[test]
fn legal_unit_scope_directive_placements() {
    check(
        r#"`timescale 1ns/1ps
class holder;
  int a = 4;
`default_nettype tri1
  int b = 5;
endclass

function automatic int increment(int arg);
`default_nettype tri0
  return arg + 1;
endfunction

task automatic show(int v);
  v += 1;
`default_nettype tri1
  $display("value=task %0d", v);
endtask

covergroup cg with function sample(bit v);
`default_nettype tri0
  coverpoint v { bins zero = {0}; bins one = {1}; }
endgroup

typedef struct packed {
  logic first;
`default_nettype tri1
  logic second;
} pair_t;

module top;
  holder h = new;
  pair_t pair_value = 2'b10;
  cg group = new;
  initial begin
    group.sample(1);
    #1;
    $display("value=class %0d%0d function %0d struct %b", h.a, h.b, increment(6), pair_value);
    show(8);
    $display("value=coverage %0.0f", group.get_inst_coverage());
    $finish;
  end
endmodule
`default_nettype wire
"#,
        &[
            "value=class 45 function 7 struct 10",
            "value=task 9",
            "value=coverage 50",
        ],
    );
}

const WIRE_SAMPLE: &str = r#"
module child(input a); endmodule
module top;
  child c();
  initial begin #1; $display("value=%b", c.a); $finish; end
endmodule
"#;

/// Each boundary restores the wire default (§22.2, §22.3, §22.4, §22.8,
/// §22.10, §22.12). The line-control directive occupies a line of its own.
#[test]
fn wire_default_after_other_directives() {
    let dir = TestDir::new();
    dir.write("empty.svh", "// Empty include.\n");
    for boundary in [
        "`timescale 1ns/1ps `default_nettype wire",
        "`celldefine `default_nettype wire\n`endcelldefine",
        "`include \"empty.svh\" `default_nettype wire",
        "`line 7 \"source.sv\" 0\n`default_nettype wire",
        "`resetall `default_nettype wire\n`timescale 1ns/1ps",
        "`timescale 1ns/1ps `resetall\n`timescale 1ns/1ps",
    ] {
        let source =
            format!("`timescale 1ns/1ps\n`default_nettype tri1\n{boundary}\n{WIRE_SAMPLE}");
        dir.write("design.sv", &source);
        check_run(&dir.0, &["design.sv"], &["value=z"]);
    }
}

/// The tail must expand as a text macro (§22.5.1), including the entire
/// argument list in the function-like form.
#[test]
fn macro_generated_wire_default_after_timescale() {
    for tail in ["`END_PULL", "`IDENTITY(`default_nettype wire)"] {
        let source = format!(
            "`timescale 1ns/1ps\n`define END_PULL `default_nettype wire\n\
             `define IDENTITY(x) x\n`default_nettype tri1\n\
             `timescale 1ns/1ps {tail}\n{WIRE_SAMPLE}"
        );
        check(&source, &["value=z"]);
    }
}

/// Source and library have separate directive state (§22.2). A merged dump
/// must replay the library input's wire default for either library option.
#[test]
fn merged_dump_preserves_library_wire_default() {
    let dir = TestDir::new();
    dir.write(
        "design.sv",
        r#"`timescale 1ns/1ps
`default_nettype tri1
module top;
  lib_child c();
  initial begin #1; $display("value=%b", c.a); $finish; end
endmodule
"#,
    );
    fs::create_dir(dir.0.join("lib")).expect("create library directory");
    dir.write(
        "lib/lib_child.sv",
        "`timescale 1ns/1ps\nmodule lib_child(input a); endmodule\n",
    );
    for (flag, lib, dump) in [
        ("-y", "lib", "merged_y.sv"),
        ("-v", "lib/lib_child.sv", "merged_v.sv"),
    ] {
        check_run(
            &dir.0,
            &[
                "design.sv",
                "-s",
                "top",
                flag,
                lib,
                "--dump-merged-sv",
                dump,
            ],
            &["value=z"],
        );
        check_run(&dir.0, &[dump, "-s", "top"], &["value=z"]);
    }
}

/// §22.3 and §22.8 require placement outside a design element. The lenient
/// CLI accepts the placement and the explicitly declared wire still floats.
#[test]
fn lenient_module_directive_keeps_explicit_wire() {
    for directive in ["`default_nettype wire", "`resetall"] {
        let dir = TestDir::new();
        let source = format!(
            r#"`timescale 1ns/1ps
`default_nettype tri1
module child(input wire a);
{directive}
endmodule
`timescale 1ns/1ps
module top;
  child c();
  initial begin #1; $display("value=%b", c.a); $finish; end
endmodule
"#
        );
        dir.write("design.sv", &source);
        check_run(&dir.0, &["--no-strict", "design.sv"], &["value=z"]);
    }
}

/// With the implicit or explicit wire default, undriven nets read z and
/// driven nets convey the driver (§6.6, §6.6.1, §22.8). Explicit port kinds
/// and explicitly typed outputs use their own kinds (§23.2.2.3).
#[test]
fn plain_wire_ports() {
    check(
        r#"`timescale 1ns/1ps
module ordinary(input a); endmodule
`default_nettype tri1
module declared(input wire w, input tri0 low, input tri1 high, output logic v); endmodule
`default_nettype wire
module explicit_default(input a); endmodule
module top;
  wire driven = 1'b1;
  ordinary open_input(), driven_input(.a(driven));
  explicit_default explicit_input();
  declared kinds();
  initial begin
    #1; $display("value=%b%b%b", open_input.a, driven_input.a, explicit_input.a);
    $display("value=%b%b%b%b", kinds.w, kinds.low, kinds.high, kinds.v);
    $finish;
  end
endmodule
"#,
        &["value=z1z", "value=z01x"],
    );
}
