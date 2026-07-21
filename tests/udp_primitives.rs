//! IEEE 1800-2017 §29 User-Defined Primitive (UDP) regression tests.
//!
//! Every expected value below was produced by a reference simulator and byte-matched against xezim. The
//! cases mirror the §29 feature matrix:
//!   1. combinational mux (§29.3) + and/xor
//!   2. edge DFF (§29.5, `(01)` clock)
//!   3. level-sensitive latch (§29.4, `-` hold)
//!   4. edge DFF + async level reset (dominance via row order)
//!   5. `initial` start state (§29.6) — clobbered by an unmatched t=0 edge
//!   6. UDP-based cell adopted from a `-v` library file
//!   7. edge shorthands `r f * (??)`
//!   + instance `#delay` (§29.7)
//!
//! The key empirically-verified semantic: on ANY input change with no
//! matching table row, a UDP output becomes `x` (both combinational and
//! sequential). Holding happens only via an explicit `-` output row, or when
//! no input changed at all (no evaluation event).

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

/// Run xezim on `src` (written to a temp file) with optional extra args, and
/// return the collapsed `t=…` monitor lines: for each timestamp only the LAST
/// emitted value is kept (matching $monitor's end-of-timestep semantics), in
/// first-seen timestamp order.
fn run_monitor(name: &str, src: &str, extra: &[&str]) -> Vec<String> {
    let dir = std::env::temp_dir().join("xezim_udp_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join(format!("{name}.v"));
    std::fs::write(&sv, src).expect("write sv");

    let mut cmd = Command::new(xezim_bin());
    // Elaboration-time warnings vanish on design-cache hits; force a cold run.
    cmd.env("XEZIM_NO_CACHE", "1");
    cmd.arg("--simulate").arg(&sv);
    for a in extra {
        cmd.arg(a);
    }
    let out = cmd.output().expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Collapse to last-value-per-timestamp, preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut val: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("t=") {
            let ts = rest.split_whitespace().next().unwrap_or("").to_string();
            if !val.contains_key(&ts) {
                order.push(ts.clone());
            }
            val.insert(ts, line.to_string());
        }
    }
    order.into_iter().map(|ts| val[&ts].clone()).collect()
}

fn assert_trace(name: &str, src: &str, extra: &[&str], expected: &[&str]) {
    let got = run_monitor(name, src, extra);
    assert_eq!(
        got,
        expected.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "\nUDP trace mismatch for `{name}` (expected = a reference simulator):\n\
         got:\n  {}\nexpected:\n  {}\n",
        got.join("\n  "),
        expected.join("\n  "),
    );
}

// 1a. Combinational mux (§29.3).
#[test]
fn comb_mux() {
    let src = r#"
primitive udp_mux(q, sel, a, b);
  output q; input sel, a, b;
  table
     0 1 ? : 1;   0 0 ? : 0;
     1 ? 1 : 1;   1 ? 0 : 0;
     x 0 0 : 0;   x 1 1 : 1;
  endtable
endprimitive
module tb;
  reg sel,a,b; wire q;
  udp_mux m(q,sel,a,b);
  initial begin
    $monitor("t=%0t sel=%b a=%b b=%b q=%b",$time,sel,a,b,q);
    sel=0;a=0;b=0; #1 a=1; #1 b=1; #1 sel=1; #1 a=0; #1 b=0;
    #1 sel=1'bx;a=1;b=1; #1 a=0;b=0; #1 $finish;
  end
endmodule
"#;
    assert_trace("mux", src, &[], &[
        "t=0 sel=0 a=0 b=0 q=0",
        "t=1 sel=0 a=1 b=0 q=1",
        "t=2 sel=0 a=1 b=1 q=1",
        "t=3 sel=1 a=1 b=1 q=1",
        "t=4 sel=1 a=0 b=1 q=1",
        "t=5 sel=1 a=0 b=0 q=0",
        "t=6 sel=x a=1 b=1 q=1",
        "t=7 sel=x a=0 b=0 q=0",
    ]);
}

// 1b. Combinational and/xor.
#[test]
fn comb_and_xor() {
    let src = r#"
primitive udp_and(o,a,b); output o; input a,b;
 table 1 1:1; 0 ?:0; ? 0:0; endtable
endprimitive
primitive udp_xor(o,a,b); output o; input a,b;
 table 0 0:0; 0 1:1; 1 0:1; 1 1:0; endtable
endprimitive
module tb;
 reg a,b; wire oa,ox; udp_and ua(oa,a,b); udp_xor ux(ox,a,b);
 initial begin
  $monitor("t=%0t a=%b b=%b and=%b xor=%b",$time,a,b,oa,ox);
  a=0;b=0; #1 a=1; #1 b=1; #1 a=0; #1 a=1'bx; #1 b=0; #1 $finish;
 end
endmodule
"#;
    assert_trace("andxor", src, &[], &[
        "t=0 a=0 b=0 and=0 xor=0",
        "t=1 a=1 b=0 and=0 xor=1",
        "t=2 a=1 b=1 and=1 xor=0",
        "t=3 a=0 b=1 and=0 xor=1",
        "t=4 a=x b=1 and=x xor=x",
        "t=5 a=x b=0 and=0 xor=x",
    ]);
}

// 2. Edge DFF (§29.5).
#[test]
fn edge_dff() {
    let src = r#"
primitive udp_dff(q, clk, d);
  output q; reg q; input clk, d;
  table
    (01) 0 : ? : 0 ;   (01) 1 : ? : 1 ;
    (0x) 1 : 1 : 1 ;   (0x) 0 : 0 : 0 ;
    (?0) ? : ? : - ;    ? (??): ? : - ;
  endtable
endprimitive
module tb;
  reg clk,d; wire q; udp_dff u(q,clk,d);
  initial begin
    $monitor("t=%0t clk=%b d=%b q=%b",$time,clk,d,q);
    clk=0; d=0; #1 d=1; #1 clk=1; #1 clk=0; #1 d=0;
    #1 clk=1; #1 clk=0; #1 d=1; #1 clk=1; #1 $finish;
  end
endmodule
"#;
    assert_trace("dff", src, &[], &[
        "t=0 clk=0 d=0 q=x",
        "t=1 clk=0 d=1 q=x",
        "t=2 clk=1 d=1 q=1",
        "t=3 clk=0 d=1 q=1",
        "t=4 clk=0 d=0 q=1",
        "t=5 clk=1 d=0 q=0",
        "t=6 clk=0 d=0 q=0",
        "t=7 clk=0 d=1 q=0",
        "t=8 clk=1 d=1 q=1",
    ]);
}

// 3. Level-sensitive latch (§29.4, `-` hold).
#[test]
fn level_latch() {
    let src = r#"
primitive udp_latch(q, en, d);
  output q; reg q; input en, d;
  table
    0 ? : ? : - ;
    1 0 : ? : 0 ;
    1 1 : ? : 1 ;
    1 x : ? : x ;
  endtable
endprimitive
module tb;
  reg en,d; wire q; udp_latch u(q,en,d);
  initial begin
    $monitor("t=%0t en=%b d=%b q=%b",$time,en,d,q);
    en=0;d=0; #1 d=1; #1 en=1; #1 d=0; #1 d=1; #1 en=0; #1 d=0; #1 en=1; #1 $finish;
  end
endmodule
"#;
    assert_trace("latch", src, &[], &[
        "t=0 en=0 d=0 q=x",
        "t=1 en=0 d=1 q=x",
        "t=2 en=1 d=1 q=1",
        "t=3 en=1 d=0 q=0",
        "t=4 en=1 d=1 q=1",
        "t=5 en=0 d=1 q=1",
        "t=6 en=0 d=0 q=1",
        "t=7 en=1 d=0 q=0",
    ]);
}

// 4. Edge DFF + async level reset (dominance via row order).
#[test]
fn async_reset_dff() {
    let src = r#"
primitive udp_adff(q, clk, d, rst);
  output q; reg q; input clk, d, rst;
  table
    ?     ?  1  : ? : 0 ;
   (01)   0  0  : ? : 0 ;
   (01)   1  0  : ? : 1 ;
   (0x)   1  0  : 1 : 1 ;
   (0x)   0  0  : 0 : 0 ;
   (?0)   ?  0  : ? : - ;
    ?   (??) 0  : ? : - ;
    ?     ? (?0): ? : - ;
  endtable
endprimitive
module tb;
  reg clk,d,rst; wire q; udp_adff u(q,clk,d,rst);
  initial begin
    $monitor("t=%0t clk=%b d=%b rst=%b q=%b",$time,clk,d,rst,q);
    clk=0;d=0;rst=1; #1 rst=0; #1 d=1; #1 clk=1; #1 clk=0;
    #1 rst=1; #1 clk=1; #1 rst=0; #1 clk=0; #1 d=0; #1 clk=1; #1 $finish;
  end
endmodule
"#;
    assert_trace("adff", src, &[], &[
        "t=0 clk=0 d=0 rst=1 q=0",
        "t=1 clk=0 d=0 rst=0 q=0",
        "t=2 clk=0 d=1 rst=0 q=0",
        "t=3 clk=1 d=1 rst=0 q=1",
        "t=4 clk=0 d=1 rst=0 q=1",
        "t=5 clk=0 d=1 rst=1 q=0",
        "t=6 clk=1 d=1 rst=1 q=0",
        "t=7 clk=1 d=1 rst=0 q=0",
        "t=8 clk=0 d=1 rst=0 q=0",
        "t=9 clk=0 d=0 rst=0 q=0",
        "t=10 clk=1 d=0 rst=0 q=0",
    ]);
}

// 5. `initial` start state (§29.6): the t=0 clk x->0 edge is unmatched, so
// a reference simulator clobbers the initial 1 to x immediately.
#[test]
fn initial_state() {
    let src = r#"
primitive udp_tff(q, clk);
  output q; reg q; input clk;
  initial q = 1'b1;
  table
    (01) : ? : 0 ;
    (10) : 0 : 0 ;
    (10) : 1 : 1 ;
  endtable
endprimitive
module tb;
  reg clk; wire q; udp_tff u(q,clk);
  initial begin
    $monitor("t=%0t clk=%b q=%b",$time,clk,q);
    clk=0; #1 clk=1; #1 clk=0; #1 clk=1; #1 $finish;
  end
endmodule
"#;
    assert_trace("initst", src, &[], &[
        "t=0 clk=0 q=x",
        "t=1 clk=1 q=0",
        "t=2 clk=0 q=0",
        "t=3 clk=1 q=0",
    ]);
}

// 6. UDP-based cell adopted from a `-v` library file (vendor stdcell).
#[test]
fn library_v_file_udp() {
    let dir = std::env::temp_dir().join("xezim_udp_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let vfile = dir.join("vendor_lib.v");
    std::fs::write(&vfile, r#"
primitive udp_dff(q, clk, d);
  output q; reg q; input clk, d;
  table
    (01) 0 : ? : 0 ;   (01) 1 : ? : 1 ;
    (?0) ? : ? : - ;    ? (??): ? : - ;
    (0x) 1 : 1 : 1 ;   (0x) 0 : 0 : 0 ;
  endtable
endprimitive
module DFFX1(Q, CK, D);
  output Q; input CK, D;
  udp_dff u(Q, CK, D);
endmodule
"#).expect("write vfile");

    let src = r#"
module tb;
  reg ck,d; wire q;
  DFFX1 dff(q, ck, d);
  initial begin
    $monitor("t=%0t ck=%b d=%b q=%b",$time,ck,d,q);
    ck=0; d=0; #1 d=1; #1 ck=1; #1 ck=0; #1 d=0; #1 ck=1; #1 ck=0; #1 $finish;
  end
endmodule
"#;
    let vpath = vfile.to_str().unwrap();
    assert_trace("libtop", src, &["-v", vpath], &[
        "t=0 ck=0 d=0 q=x",
        "t=1 ck=0 d=1 q=x",
        "t=2 ck=1 d=1 q=1",
        "t=3 ck=0 d=1 q=1",
        "t=4 ck=0 d=0 q=1",
        "t=5 ck=1 d=0 q=0",
        "t=6 ck=0 d=0 q=0",
    ]);
}

#[test]
fn primitive_verbose_is_scoped_to_explicit_v_files() {
    let dir = std::env::temp_dir().join("xezim_udp_verbose");
    let lib_dir = dir.join("lib");
    std::fs::create_dir_all(&lib_dir).expect("mkdir");
    let vfile = lib_dir.join("vendor.v");
    std::fs::write(&vfile, r#"primitive udp_buf(q, a);
  output q; input a;
  table 0 : 0; 1 : 1; endtable
endprimitive
module BUFX1(output q, input a); udp_buf u(q, a); endmodule
"#).expect("write library");
    let top = dir.join("top.sv");
    std::fs::write(&top, "module top; wire q; BUFX1 u(q, 1'b1); endmodule\n")
        .expect("write top");

    let run = |args: &[&str]| {
        let out = Command::new(xezim_bin())
            .env("XEZIM_NO_CACHE", "1")
            .current_dir(&dir)
            .args(args)
            .output()
            .expect("run xezim");
        format!("{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr))
    };

    let verbose = run(&[
        "--compile",
        "--primitive-verbose",
        "-v",
        "lib/vendor.v",
        "top.sv",
    ]);
    for expected in [
        "[primitive-verbose] parsed UDP 'udp_buf'",
        "[primitive-verbose] parsed module 'BUFX1'",
        "[primitive-verbose] adopting module 'BUFX1'",
        "[primitive-verbose] adopting UDP 'udp_buf'",
        "[primitive-verbose] -v resolution summary",
    ] {
        assert!(verbose.contains(expected), "missing {:?}:\n{}", expected, verbose);
    }

    std::fs::write(
        dir.join("verbose.f"),
        "--primitive-verbose\n-v lib/vendor.v\ntop.sv\n",
    ).expect("write verbose filelist");
    let via_filelist = run(&["--compile", "-f", "verbose.f"]);
    assert!(
        via_filelist.contains("[primitive-verbose] parsed UDP 'udp_buf'"),
        "filelist did not enable primitive diagnostics:\n{}",
        via_filelist
    );

    let quiet = run(&["--compile", "-v", "lib/vendor.v", "top.sv"]);
    assert!(!quiet.contains("[primitive-verbose]"), "diagnostics must be opt-in:\n{}", quiet);

    let y_only = run(&["--compile", "--primitive-verbose", "-y", "lib", "top.sv"]);
    assert!(
        !y_only.contains("[primitive-verbose]"),
        "--primitive-verbose must not trace -y files:\n{}",
        y_only
    );

    let bad_v = lib_dir.join("bad.v");
    std::fs::write(
        &bad_v,
        "primitive broken(q, a); output q; input a; THIS_IS_ILLEGAL !!! endprimitive\n",
    ).expect("write malformed primitive");
    let parse_issue = run(&[
        "--compile",
        "--primitive-verbose",
        "-v",
        "lib/bad.v",
        "top.sv",
    ]);
    assert!(
        parse_issue.contains("[primitive-verbose] detailed parser diagnostics")
            && parse_issue.contains("THIS_IS_ILLEGAL")
            && parse_issue.contains('^'),
        "missing detailed primitive parse context:\n{}",
        parse_issue
    );
}

// 7. Edge shorthands `r f * (??)`.
#[test]
fn edge_shorthands() {
    let src = r#"
primitive udp_r(q,c); output q; reg q; input c;
 table r:?:1; f:?:0; endtable
endprimitive
primitive udp_star(q,c,d); output q; reg q; input c,d;
 table
   * 0 : ? : 0 ;
   * 1 : ? : 1 ;
   ? (??): ? : - ;
 endtable
endprimitive
module tb;
 reg c,d; wire q1,q2; udp_r ur(q1,c); udp_star us(q2,c,d);
 initial begin
  $monitor("t=%0t c=%b d=%b q1=%b q2=%b",$time,c,d,q1,q2);
  c=0;d=0; #1 c=1; #1 d=1; #1 c=0; #1 c=1; #1 d=0; #1 c=0; #1 $finish;
 end
endmodule
"#;
    assert_trace("short", src, &[], &[
        "t=0 c=0 d=0 q1=x q2=x",
        "t=1 c=1 d=0 q1=1 q2=0",
        "t=2 c=1 d=1 q1=1 q2=0",
        "t=3 c=0 d=1 q1=0 q2=1",
        "t=4 c=1 d=1 q1=1 q2=1",
        "t=5 c=1 d=0 q1=1 q2=1",
        "t=6 c=0 d=0 q1=0 q2=0",
    ]);
}

// 8. Instance `#delay` (§29.7): the buffered output is x until the first
// delayed drive lands, then follows the input with the instance delay.
#[test]
fn instance_delay() {
    let src = r#"
primitive udp_buf(o,a); output o; input a;
 table 0:0; 1:1; x:x; endtable
endprimitive
module tb;
 reg a; wire o;
 udp_buf #(3) u(o,a);
 initial begin
  $monitor("t=%0t a=%b o=%b",$time,a,o);
  a=0; #5 a=1; #5 a=0; #5 $finish;
 end
endmodule
"#;
    assert_trace("delay", src, &[], &[
        "t=0 a=0 o=x",
        "t=3 a=0 o=0",
        "t=5 a=1 o=0",
        "t=8 a=1 o=1",
        "t=10 a=0 o=1",
        "t=13 a=0 o=0",
    ]);
}

// Fail-loud: an unparseable table row must warn and leave the output undriven
// (not silently produce a wrong value), and must not crash the run.
#[test]
fn malformed_table_fails_loud() {
    let dir = std::env::temp_dir().join("xezim_udp_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("bad.v");
    std::fs::write(&sv, r#"
primitive udp_bad(q,a,b); output q; input a,b;
 table
   0 1 : 1 ;
   @ 1 : 0 ;
 endtable
endprimitive
module tb; reg a,b; wire q; udp_bad u(q,a,b);
 initial begin a=0; b=1; #1 $display("done q=%b",q); $finish; end
endmodule
"#).expect("write");
    let out = Command::new(xezim_bin())
        .env("XEZIM_NO_CACHE", "1")
        .arg("--simulate").arg(&sv)
        .output().expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stderr.contains("UNSUPPORTED UDP TABLE") && stderr.contains("udp_bad"),
        "expected a loud UNSUPPORTED-UDP-TABLE warning naming the primitive:\n{stderr}"
    );
    // Simulation still completes; the undriven output is x/z, never a wrong value.
    assert!(
        stdout.contains("done q="),
        "simulation should complete despite the malformed table:\n{stdout}"
    );
}

/// A chain of sequential UDPs (DFF cells) must behave as a shift register:
/// each stage captures the PREVIOUS stage's OLD value on the clock edge, not
/// the new one. A comb-style immediate output write collapses the chain
/// (every stage sees the new value in one edge). Verified against a commercial
/// simulator AND a reference simulator: `xxx1 -> xx10 -> x100 -> 1000`. Regression guard for
/// the NBA deferral of sequential UDP outputs.
#[test]
fn sequential_udp_chain_shifts_one_stage_per_clock() {
    let src = r#"
primitive dff(q,clk,d); output q; reg q; input clk,d;
table (01) 0:?:0; (01) 1:?:1; (0?) 1:1:1; (0?) 0:0:0; (?0) ?:?:-; ? (??):?:-; endtable endprimitive
module t; reg clk,din; wire q0,q1,q2,q3;
 dff a(q0,clk,din); dff b(q1,clk,q0); dff c(q2,clk,q1); dff d(q3,clk,q2);
 task tick; begin #1 clk=1; #1 clk=0; end endtask
 initial begin clk=0;din=1;
   tick; $display("C1 %b%b%b%b",q3,q2,q1,q0);
   din=0; tick; $display("C2 %b%b%b%b",q3,q2,q1,q0);
   tick; $display("C3 %b%b%b%b",q3,q2,q1,q0);
   tick; $display("C4 %b%b%b%b",q3,q2,q1,q0);
   $finish; end
endmodule
"#;
    let sim = xezim::simulate(src, 100).expect("simulate");
    let o: String = sim.output.iter().map(|m| m.message.clone()).collect::<Vec<_>>().join("\n");
    assert!(o.contains("C1 xxx1"), "stage 1:\n{}", o);
    assert!(o.contains("C2 xx10"), "stage 2:\n{}", o);
    assert!(o.contains("C3 x100"), "stage 3:\n{}", o);
    assert!(o.contains("C4 1000"), "stage 4 (shifted through):\n{}", o);
}

#[test]
fn primitive_verbose_reports_kind_values_and_implicit_nets() {
    // --primitive-verbose v2: header carries sequential/comb + row count +
    // init, terminals show their current value, §6.10 implicit nets are
    // flagged (the dropped-cell signature), and a summary line closes.
    let dir = std::env::temp_dir().join("xezim_pv2_test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("pv2.v");
    std::fs::write(
        &sv,
        "`timescale 1ns/1ns\n\
         primitive udp_b (q, a); output q; input a;\n\
           table 0 : 1 ; 1 : 0 ; endtable\n\
         endprimitive\n\
         module top;\n\
           reg a = 0; wire q;\n\
           assign dangling = floating;\n\
           udp_b u0 (q, floating);\n\
           initial #2 $finish;\n\
         endmodule\n",
    )
    .expect("write sv");
    let out = std::process::Command::new(xezim_bin())
        .env("XEZIM_NO_CACHE", "1")
        .arg(&sv)
        .arg("--primitive-verbose")
        .arg("-xenowarn")
        .arg("--max-time")
        .arg("10")
        .output()
        .expect("run xezim");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("combinational, 2 table row(s), init=x"),
        "missing kind/rows/init on the header:\n{}",
        stderr
    );
    assert!(
        stderr.contains("[IMPLICIT NET"),
        "implicit-net terminal not flagged:\n{}",
        stderr
    );
    assert!(
        stderr.contains("UDP summary: 1 instance(s)")
            && stderr.contains("1 terminal(s) on implicit nets"),
        "missing summary line:\n{}",
        stderr
    );
}
