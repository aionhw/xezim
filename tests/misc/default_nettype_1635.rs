//! IEEE 1800-2023 §6.10, §22.8 and §23.2.2.3.

fn run(src: &str) -> Result<Vec<String>, String> {
    xezim::simulate(src, 100).map(|sim| {
        sim.output
            .iter()
            .map(|entry| entry.message.clone())
            .collect()
    })
}

#[test]
fn wire_and_resetall_restore_implicit_nets_without_retroactive_effects() {
    let lines = run(r#"
module early; assign n = 1'b1; endmodule
`default_nettype none
module strict_child(input wire a); endmodule
`default_nettype wire
module restored; assign n = 1'b1; endmodule
`default_nettype none
`resetall
module top;
  early u();
  restored r();
  strict_child s(n);
  assign n = 1'b1;
  initial #1 $display("restore %b %b %b", n, u.n, r.n);
endmodule
"#)
    .unwrap();
    assert_eq!(lines, ["restore 1 1 1"]);
}

#[test]
fn macro_and_same_line_directive_are_applied() {
    let src = r#"
`define STRICT none
module early; endmodule `default_nettype `STRICT module top;
  early u();
  assign ghost = 1'b1;
endmodule
"#;
    assert!(run(src).unwrap_err().contains("Implicit net 'ghost'"));
}

#[test]
fn declared_port_kinds_and_nonansi_redeclarations_are_accepted() {
    run(r#"
`default_nettype none
module top(a, b); input a; wire a; output var b; endmodule
"#)
    .unwrap();
}

#[test]
fn inferred_nonansi_port_kinds_are_rejected_under_none() {
    let error = run("`default_nettype none\nmodule top(a); input a; endmodule").unwrap_err();
    assert!(
        error.contains("port 'a'") && error.contains("default_nettype none"),
        "{error}"
    );
}

#[test]
fn inferred_ansi_port_kinds_warn_and_default_to_wire_under_none() {
    struct TempFile(std::path::PathBuf);
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    let path = std::env::temp_dir().join(format!("xezim_ansi_nettype_{}.sv", std::process::id()));
    let _guard = TempFile(path.clone());
    std::fs::write(
        &path,
        r#"
`default_nettype none

module dut_missing_port_type (
   input        clk,
   input  logic rst_n,
   output logic q
);
   always_ff @(posedge clk or negedge rst_n) begin
      if (!rst_n)
         q <= 1'b0;
      else
         q <= ~q;
   end
endmodule

module dut_defaults (
   input           a,
   input wire      explicit_wire,
   input var logic explicit_var,
   inout           b,
   output          y
);
endmodule

`default_nettype wire

module tb_top;
   logic clk = 1'b0;
   logic rst_n = 1'b0;
   logic q;

   dut_missing_port_type u_dut (
      .clk   (clk),
      .rst_n (rst_n),
      .q     (q)
   );

   dut_defaults u_def();

   initial begin
      #1;
      $display("rst=%b open=%b", q, u_def.a);
      rst_n = 1'b1;
      #1 clk = 1'b1;
      #1;
      $display("clk=%b", q);
      $finish;
   end
endmodule
"#,
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--no-cache")
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("rst=0 open=z"), "{stdout}");
    assert!(stdout.contains("clk=1"), "{stdout}");
    assert!(!stderr.contains("error:"), "{stderr}");
    for port in ["clk", "rst_n", "a", "b", "y"] {
        let needle = format!("port '{port}'");
        assert!(
            stderr
                .lines()
                .any(|line| line.contains("warning:")
                    && line.contains(&needle)
                    && line.contains("`default_nettype none")
                    && line.contains("wire")),
            "missing warning for {port} in:\n{stderr}"
        );
    }
    for explicit in ["'q'", "'explicit_wire'", "'explicit_var'"] {
        assert!(!stderr.contains(explicit), "unexpected warning for {explicit}:\n{stderr}");
    }

    // Genuine implicit nets inside a module with an ANSI port warning still error.
    let error = run(r#"
`default_nettype none
module top(input clk);
   assign ghost = clk;
endmodule
"#)
    .unwrap_err();
    assert!(
        error.contains("ghost") && error.contains("default_nettype none"),
        "{error}"
    );
}

#[test]
fn child_assign_gate_udp_and_switch_use_the_child_default() {
    for body in [
        "assign ghost = 1'b1;",
        "buf (ghost, known);",
        "tran (ghost, known);",
        "pbuf p(ghost, known);",
        "leaf l(ghost);",
        "if (1) begin : g assign ghost = 1'b1; end",
    ] {
        let src = format!(
            r#"
primitive pbuf(o, i); output o; input i; table 0:0; 1:1; endtable endprimitive
module leaf(input wire i); endmodule
`default_nettype none
module child;
  wire known = 1'b1;
  {body}
endmodule
`default_nettype wire
module top; child c(); endmodule
"#
        );
        let error = run(&src).unwrap_err();
        assert!(
            error.contains("ghost") && error.contains("default_nettype none"),
            "{body}: {error}"
        );
    }
}

#[test]
fn nonwire_defaults_apply_to_implicit_nets_and_ports() {
    let lines = run(r#"
`default_nettype wand
module wired;
  assign n=1'b0; assign n=1'b1;
endmodule
`default_nettype tri1
module nonansi(p); input p; endmodule
module ansi(input p); endmodule
`default_nettype wor
module top;
  assign n=1'b0; assign n=1'b1;
  wired u(); nonansi a(); ansi b();
  initial #1 $display("kinds %b %b %b %b", n, u.n, a.p, b.p);
endmodule
"#)
    .unwrap();
    assert_eq!(lines, ["kinds 1 0 1 1"]);
}

#[test]
fn invalid_directive_value_is_diagnosed() {
    for operand in ["none_bogus", "", "foo"] {
        let error = run(&format!(
            "`default_nettype {operand}\nmodule top; endmodule"
        ))
        .unwrap_err();
        assert!(error.contains("invalid `default_nettype value"), "{error}");
    }
}

#[test]
fn an_independent_simulation_starts_with_wire() {
    run("`default_nettype none\nmodule top; endmodule").unwrap();
    run("module top; assign n=1'b0; endmodule").unwrap();
}

#[test]
fn a_directive_inside_a_design_element_is_rejected() {
    let error = run("module top;\n`default_nettype wire\nendmodule").unwrap_err();
    assert!(error.contains("illegal inside a design element"), "{error}");
}

#[test]
fn separate_source_files_preserve_directive_order() {
    let dir = std::env::temp_dir().join(format!("xezim_nettype_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths: Vec<_> = [
        "`default_nettype none\nmodule strict; endmodule",
        "`resetall\nmodule top; strict u(); assign n=1'b1; initial #1 $display(\"restored %b\", n); endmodule",
    ].iter().enumerate().map(|(index, src)| {
        let path = dir.join(format!("file_{index}.sv"));
        std::fs::write(&path, src).unwrap();
        path
    }).collect();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--no-cache")
        .args(paths)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("restored 1"));
}

#[test]
fn a_bound_connection_inherits_the_targets_default() {
    let error = run(r#"
module probe(input wire i); endmodule
`default_nettype none
module target; endmodule
`default_nettype wire
bind target probe b(.i(ghost));
module top; target u(); endmodule
"#)
    .unwrap_err();
    assert!(
        error.contains("ghost") && error.contains("default_nettype none"),
        "{error}"
    );
}
