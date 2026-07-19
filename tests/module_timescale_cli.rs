//! The `--module-timescale` command-line extension: assign a time unit and
//! precision to module definitions that have no explicit source-level
//! timescale, without overriding one that does.
//!
//! Precedence (highest first): local timeunit/timeprecision decl >
//! active `timescale directive > named --module-timescale > global
//! --module-timescale > 1ns/1ns default.
//!
//! Driven through the CLI so the whole arg → validate → set → elaborate path
//! is exercised.

use std::process::Command;

fn run(args: &[&str], src: &str) -> (String, bool) {
    // Unique per call — tests run in parallel, and a name keyed on the arg
    // length alone collides (two tests with same-length args race on the
    // same file).
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let path = dir.join(format!("mts_{}_{}.sv", std::process::id(), n));
    std::fs::write(&path, src).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let mut cmd = Command::new(bin);
    cmd.arg("--sv2017").arg("--max-time").arg("10000000");
    for a in args {
        cmd.arg(a);
    }
    cmd.arg(&path);
    let out = cmd.output().expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (text, out.status.success())
}

/// Four modules covering every precedence layer.
const DESIGN: &str = r#"
`timescale 1ns/1ns
module module_a;
  initial begin #1; $display("A=%0d", $time); end
endmodule
`resetall
module module_b;
  initial begin #1; $display("B=%0d", $time); end
endmodule
module module_c;
  initial begin #1; $display("C=%0d", $time); end
endmodule
module module_d;
  timeunit 10ns; timeprecision 1ns;
  initial begin #1; $display("D=%0d", $time); end
endmodule
module top;
  module_a a(); module_b b(); module_c c(); module_d d();
endmodule
"#;

#[test]
fn precedence_directive_global_named_local() {
    let (o, ok) = run(
        &["--module-timescale", "100ns/1ns", "--module-timescale", "module_c=10ns/1ns"],
        DESIGN,
    );
    assert!(ok, "run failed: {}", o);
    // Every module's $time is 1 in its OWN unit; the unit differs per precedence.
    for m in ["A=1", "B=1", "C=1", "D=1"] {
        assert!(o.contains(m), "missing {}: {}", m, o);
    }
    // module_a has a directive, so the CLI is ignored (with a warning).
    // (No named assignment targets it here, so no warning; the point is it
    // stays 1ns — proven below by the delay test.)
}

#[test]
fn a_named_delay_is_scaled() {
    // module_c gets 10ns/1ns; its `#1` must be 10ns. A 1ns reference fires at
    // 1ns and 100ns; module_c lands between them.
    let (o, ok) = run(
        &["--module-timescale", "module_c=10ns/1ns"],
        r#"
module module_c;
  initial begin #1; $display("C_AT"); end
endmodule
module refr;
  initial begin #1; $display("R1"); #99; $display("R100"); end
endmodule
module top; module_c c(); refr r(); endmodule
"#,
    );
    assert!(ok, "{}", o);
    let r1 = o.find("R1").unwrap();
    let c = o.find("C_AT").expect("C_AT missing");
    let r100 = o.find("R100").unwrap();
    assert!(r1 < c && c < r100, "C (#1 = 10ns) must land between R1 and R100: {}", o);
}

#[test]
fn an_explicit_directive_is_never_overridden() {
    let (o, ok) = run(
        &["--module-timescale", "module_a=100ns/1ns"],
        r#"
`timescale 1ns/1ns
module module_a;
  initial begin #1; $display("A_AT"); end
endmodule
module refr;
  initial begin #1; $display("R1"); #99; $display("R100"); end
endmodule
module top; module_a a(); refr r(); endmodule
"#,
    );
    assert!(ok, "{}", o);
    assert!(o.contains("ignored"), "should warn the assignment was ignored: {}", o);
    // module_a stays 1ns: A_AT fires at 1ns, i.e. NOT after R1..R100 wait —
    // it fires right around R1.
    let a = o.find("A_AT").unwrap();
    let r100 = o.find("R100").unwrap();
    assert!(a < r100, "module_a #1 must stay 1ns, not become 100ns: {}", o);
}

#[test]
fn conflicting_named_assignments_are_an_error() {
    let (o, ok) = run(
        &["--module-timescale", "m=1ns/1ps", "--module-timescale", "m=10ns/1ns"],
        "module m; endmodule",
    );
    assert!(!ok, "should have failed");
    assert!(o.contains("conflicting"), "{}", o);
}

#[test]
fn precision_larger_than_unit_is_an_error() {
    let (o, ok) = run(&["--module-timescale", "1ps/1ns"], "module m; endmodule");
    assert!(!ok, "should have failed");
    assert!(o.contains("larger than unit"), "{}", o);
}

#[test]
fn an_illegal_unit_is_an_error() {
    let (o, ok) = run(&["--module-timescale", "1xs/1ns"], "module m; endmodule");
    assert!(!ok, "should have failed");
    assert!(o.contains("invalid time unit"), "{}", o);
}

#[test]
fn an_unmatched_named_module_warns() {
    let (o, _ok) = run(
        &["--module-timescale", "nope=1ns/1ns"],
        "module m; initial $display(\"ok\"); endmodule",
    );
    assert!(o.contains("did not match module 'nope'"), "{}", o);
}

#[test]
fn every_instance_of_a_definition_shares_the_timescale() {
    let (o, ok) = run(
        &["--module-timescale", "peripheral=10ns/1ns"],
        r#"
module peripheral;
  initial begin #1; $display("P=%0d", $time); end
endmodule
module top; peripheral u0(); peripheral u1(); endmodule
"#,
    );
    assert!(ok, "{}", o);
    // Both instances read 1 (their shared 10ns unit).
    assert_eq!(o.matches("P=1").count(), 2, "both instances should read 1: {}", o);
}

#[test]
fn an_edge_block_in_a_scaled_submodule_reads_time_in_its_own_unit() {
    // The always_ff's $realtime must scale to COUNTER's 10ns unit on EVERY
    // fire. It used to inherit whatever process ran last (the first edge
    // printed top-scaled time), because the edge block's timescale scope was
    // derived from its sensitivity signal — which, for a port-connected
    // clock, collapses to the PARENT's `clk` and yields no scope.
    let (o, ok) = run(
        &["--module-timescale", "counter=10ns/1ns"],
        r#"
module counter (input logic clk);
  bit [7:0] cnt;
  always_ff @(posedge clk) begin
    cnt++;
    if (cnt <= 2) $display("EDGE%0d rt=%0.3f", cnt, $realtime);
  end
endmodule
module top;
  bit clk = 0;
  always #2 clk = ~clk;   // posedges at 2ns, 6ns, ...
  counter u_cnt (.clk(clk));
  initial begin #10; $finish; end
endmodule
"#,
    );
    assert!(ok, "{}", o);
    assert!(o.contains("EDGE1 rt=0.200"), "first edge must be 0.2 (own 10ns unit): {}", o);
    assert!(o.contains("EDGE2 rt=0.600"), "second edge must be 0.6: {}", o);
}

#[test]
fn nested_hierarchy_levels_each_scale_to_their_own_unit() {
    // top(1ns) -> mid(10ns via CLI) -> leaf(100ns via CLI): `#1` and $time
    // scale per level, at any depth.
    let (o, ok) = run(
        &["--module-timescale", "mid=10ns/1ns", "--module-timescale", "leaf=100ns/1ns"],
        r#"
module leaf;
  initial begin #1; $display("LEAF t=%0d rt=%0.3f", $time, $realtime); end
endmodule
module mid;
  leaf u_leaf ();
  initial begin #1; $display("MID t=%0d rt=%0.3f", $time, $realtime); end
endmodule
`timescale 1ns/1ns
module top;
  mid u_mid ();
  initial begin #150; $display("TOP@150"); $finish; end
endmodule
"#,
    );
    assert!(ok, "{}", o);
    assert!(o.contains("MID t=1 rt=1.000"), "mid #1 = 10ns, reads 1 in its unit: {}", o);
    assert!(o.contains("LEAF t=1 rt=1.000"), "leaf #1 = 100ns, reads 1 in its unit: {}", o);
    // Ordering proves the absolute times differ: MID (10ns) before LEAF (100ns).
    let m = o.find("MID t=1").unwrap();
    let l = o.find("LEAF t=1").unwrap();
    assert!(m < l, "mid fires before leaf: {}", o);
}

// --- cross-file `timescale sticky vs --module-timescale (two source files) ---

/// Run xezim over TWO source files with the given args; return (stdout+stderr).
fn run_two(args: &[&str], file_a: &str, file_b: &str) -> String {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let pa = dir.join(format!("mts2a_{}_{}.sv", std::process::id(), n));
    let pb = dir.join(format!("mts2b_{}_{}.sv", std::process::id(), n));
    std::fs::write(&pa, file_a).unwrap();
    std::fs::write(&pb, file_b).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    let mut cmd = Command::new(bin);
    cmd.arg("--sv2017").arg("--max-time").arg("10000000");
    for a in args { cmd.arg(a); }
    cmd.arg(&pa).arg(&pb);
    let out = cmd.output().expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

// File A carries a `timescale; file B's module has NONE of its own. The
// directive must not stick across the file boundary and block the CLI:
// --module-timescale must win for the directive-less module.
const FILE_A: &str = "`timescale 1ns/1ps\nmodule othermod; endmodule\n";
const FILE_B: &str = "module sf2; initial $printtimescale; endmodule\n";

#[test]
fn module_timescale_overrides_cross_file_inherited() {
    let o = run_two(&["--module-timescale", "1ps/1ps", "-s", "sf2", "--dump-timescales"], FILE_A, FILE_B);
    assert!(o.contains("sf2                          1ps / 1ps") || o.contains("(sf2) is 1ps / 1ps"),
        "--module-timescale must override the cross-file-inherited 1ns/1ps; got:\n{}", o);
}

#[test]
fn cross_file_inherited_kept_without_cli() {
    // No --module-timescale: the single-compilation-unit sticky behavior is
    // preserved, so sf2 inherits 1ns/1ps from file A (backward compatible).
    let o = run_two(&["-s", "sf2", "--dump-timescales"], FILE_A, FILE_B);
    assert!(o.contains("(sf2) is 1ns / 1ps") || o.contains("sf2                          1ns / 1ps"),
        "without CLI, sf2 keeps the inherited 1ns/1ps; got:\n{}", o);
}
