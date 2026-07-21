//! Three fixes found via a customer gate-level design run:
//!
//! 1. §23.2.2.1 null ports — `module m (a, b,, c);` is legal (the empty slot
//!    is a port position with no name). The parser previously errored, and a
//!    `-v` library file containing one lost every definition after it,
//!    cascading into thousands of dangling-pin implicit nets.
//! 2. Command-line `+define+`/`-D` macros must reach `-v` library-file
//!    preprocessing, so `ifdef-guarded behavioral-vs-cell branches resolve
//!    the same way they do in the main sources.
//! 3. `-xenowarn` suppresses the §6.10 "implicit 1-bit net created" warnings
//!    (a gate-level design can emit thousands) without changing behavior.

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

#[test]
fn null_ports_keep_positional_alignment() {
    // u1 connects positionally: the 3rd actual lands on the null slot and is
    // dropped; a/b/c/d still line up around it.
    const SRC: &str = r#"
module np (a, b,, c, d);
  input a, b, c;
  output d;
  assign d = a & b & c;
endmodule
module top;
  reg a=1, b=1, c=1; wire d; wire d2;
  np u1 (a, b, 1'b0, c, d);
  np u2 (.a(a), .b(b), .c(c), .d(d2));
  initial begin #1; $display("D=%b D2=%b", d, d2); end
endmodule
"#;
    let out = output_of(&simulate(SRC, 100).expect("sim"));
    assert!(out.contains("D=1 D2=1"), "null port broke hookup:\n{}", out);
}

#[test]
fn cli_defines_reach_library_file_preprocessing() {
    let dir = std::env::temp_dir().join("xezim_libdef_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("cell_lib.v"),
        "module syncff (input clk, input d, output reg q);\n\
         `ifdef SIM_BEHAVIORAL\n\
           always @(posedge clk) q <= d;\n\
         `else\n\
           MISSING_HARD_CELL u0 (.CK(clk), .D(d), .Q(q));\n\
         `endif\n\
         endmodule\n",
    )
    .expect("write lib");
    std::fs::write(
        dir.join("top.v"),
        "`timescale 1ns/1ns\n\
         module top;\n\
           reg clk = 0, d = 0; wire q;\n\
           syncff u_ff (.clk(clk), .d(d), .q(q));\n\
           always #5 clk = ~clk;\n\
           initial begin d = 1; @(posedge clk); #1; $display(\"Q=%b\", q); $finish; end\n\
         endmodule\n",
    )
    .expect("write top");
    let out = std::process::Command::new(xezim_bin())
        .env("XEZIM_NO_CACHE", "1")
        .arg(dir.join("top.v"))
        .arg("-v")
        .arg(dir.join("cell_lib.v"))
        .arg("-D")
        .arg("SIM_BEHAVIORAL")
        .arg("--max-time")
        .arg("100")
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("Q=1"),
        "define did not select the behavioral branch in the -v file:\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        !stderr.contains("MISSING_HARD_CELL"),
        "cell branch was taken despite the define:\n{}",
        stderr
    );
}

#[test]
fn xenowarn_suppresses_implicit_net_warnings() {
    let dir = std::env::temp_dir().join("xezim_xenowarn_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("impnet.v");
    std::fs::write(
        &sv,
        "`timescale 1ns/1ns\n\
         module top;\n\
           assign undeclared_net = 1'b1;\n\
           initial begin #1; $display(\"N=%b\", undeclared_net); $finish; end\n\
         endmodule\n",
    )
    .expect("write sv");

    // Without the flag: warning present. XEZIM_NO_CACHE on every
    // invocation: the warning is emitted at ELABORATION time, so a design-
    // cache hit would legitimately skip it and flake the assertion.
    let out = std::process::Command::new(xezim_bin())
        .env("XEZIM_NO_CACHE", "1")
        .arg(&sv)
        .arg("--max-time")
        .arg("10")
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("implicit 1-bit net"),
        "expected the warning without -xenowarn:\n{}",
        stderr
    );

    // With -xenowarn: no warning, identical result.
    let out = std::process::Command::new(xezim_bin())
        .env("XEZIM_NO_CACHE", "1")
        .arg(&sv)
        .arg("-xenowarn")
        .arg("--max-time")
        .arg("10")
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("implicit 1-bit net"),
        "-xenowarn did not suppress the warning:\n{}",
        stderr
    );
    assert!(stdout.contains("N=1"), "behavior changed under -xenowarn:\n{}", stdout);
}
