//! A call to a function/task that no scope in the design declares must be
//! an elaboration error, including inside a sub-instance: those bodies are
//! inlined after the per-module identifier check ran, so an RTL assertion
//! helper bound to a stale header that lacked it silently never executed.
//! Also covers the `include shadowing warning that made the field case hard
//! to see: two search dirs with a same-named, different header.
use std::path::PathBuf;
use std::process::Command;

const CHILD_CALLS_UNDEFINED: &str = r#"
package common_types_pkg;
   typedef logic [0:0] bit_t;
endpackage
module glue(input logic clk, input logic rst_l, input logic cond);
   import common_types_pkg::*;
   always @(posedge clk) begin
      chk_always((rst_l !== 1), |(cond), $sformatf("in %m: cond violated"))
      ;
   end
endmodule
module testbench;
   logic clk = 0, rst_l = 0, cond = 0;
   always #5 clk = ~clk;
   initial begin #10 rst_l = 1; #20 $finish; end
   glue u_glue(.clk(clk), .rst_l(rst_l), .cond(cond));
endmodule
"#;

/// Every legal call shape a sub-instance can make: package function
/// imported only in the child, $unit function, static and instance class
/// methods, interface method, forward-referenced local function, task,
/// generate-block function, typedef and builtin casts, randomize.
const CHILD_LEGAL_CALLS: &str = r#"
function automatic int unit_fn(int a); return a + 1; endfunction
package pk; function automatic int pk_fn(int a); return a * 2; endfunction endpackage
class Cls; static function int sm(int a); return a + 100; endfunction function int im(int a); return a + 7; endfunction endclass
interface ifc; logic [7:0] v; function int get(); return v; endfunction endinterface
typedef logic [7:0] byte_t;
module child(input logic clk, ifc i);
  import pk::*;
  int acc = 0; Cls c = new;
  function automatic int later_defined(int a); return local_helper(a); endfunction
  function automatic int local_helper(int a); return a - 1; endfunction
  task automatic bump(int k); acc += k; endtask
  generate if (1) begin : g
    function automatic int gfn(int a); return a << 1; endfunction
    always @(posedge clk) acc <= acc + gfn(0);
  end endgenerate
  always @(posedge clk) begin
    acc <= acc + pk_fn(1) + unit_fn(1) + Cls::sm(1) + c.im(1) + i.get() + later_defined(3) + byte_t'(acc) + int'(1.5);
    bump(1);
    void'(c.randomize());
  end
  final $display("SMOKE acc=%0d", acc);
endmodule
module top;
  logic clk = 0; ifc i(); assign i.v = 8'd5;
  child u(.clk(clk), .i(i));
  initial begin repeat (2) #5 clk = ~clk; #1 $finish; end
endmodule
"#;

const HEADER_OLD: &str = "package pkg_h; typedef logic t; endpackage\n";
const HEADER_NEW: &str =
    "package pkg_h; typedef logic t; function void helper(input c); endfunction endpackage\n";
const USES_HEADER: &str = r#"
`include "shared.h"
module top;
  import pkg_h::*;
  initial begin helper(1); $finish; end
endmodule
"#;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("undefined_call").join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_xezim")).args(args).output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn undefined_call_inside_a_sub_instance_is_an_error() {
    let d = dir("undef");
    let src = d.join("tb.sv");
    std::fs::write(&src, CHILD_CALLS_UNDEFINED).unwrap();
    let (ok, text) = run(&["--simulate", "-s", "testbench", src.to_str().unwrap()]);
    assert!(!ok, "undefined call was accepted:\n{text}");
    assert!(
        text.contains("Undeclared identifier 'chk_always'"),
        "no diagnostic naming the undefined call:\n{text}"
    );
}

#[test]
fn every_legal_call_shape_in_a_sub_instance_still_elaborates() {
    let d = dir("legal");
    let src = d.join("top.sv");
    std::fs::write(&src, CHILD_LEGAL_CALLS).unwrap();
    let (ok, text) = run(&["--simulate", "-s", "top", src.to_str().unwrap()]);
    assert!(ok, "legal calls rejected:\n{text}");
    assert!(text.contains("SMOKE acc=122"), "wrong result:\n{text}");
}

#[test]
fn shadowed_include_copy_is_reported_and_missing_helper_is_an_error() {
    let d = dir("shadow");
    let old = d.join("old");
    let new = d.join("new");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(old.join("shared.h"), HEADER_OLD).unwrap();
    std::fs::write(new.join("shared.h"), HEADER_NEW).unwrap();
    let src = d.join("top.sv");
    std::fs::write(&src, USES_HEADER).unwrap();
    let inc_old = format!("+incdir+{}", old.display());
    let inc_new = format!("+incdir+{}", new.display());
    let (ok, text) =
        run(&["--simulate", "-s", "top", src.to_str().unwrap(), &inc_old, &inc_new]);
    assert!(!ok, "stale header accepted:\n{text}");
    assert!(text.contains("Undeclared identifier 'helper'"), "no call diagnostic:\n{text}");
    assert!(
        text.contains("[PP] warning: `include \"shared.h\"") && text.contains("is shadowed"),
        "no shadowing warning:\n{text}"
    );
    // Search order reversed: the newer copy wins and the design runs.
    let (ok, text) =
        run(&["--simulate", "-s", "top", src.to_str().unwrap(), &inc_new, &inc_old]);
    assert!(ok, "newer header rejected:\n{text}");
}

/// The field report's shape, verbatim in structure: an RTL module includes a
/// shared header whose package should carry the `chk_*` assertion helpers;
/// a stale copy without them sits in an earlier `+incdir+`.
const GLUE_V: &str = r#"
`include "common_types.h"
module datapath_glue (
   input  logic clk,
   input  logic rst_l,
   input  logic feature_mode,
   input  logic bus4_st_valid,
   input  logic [3:0] ch0_outstanding,
   input  logic [3:0] ch1_outstanding
);
   import common_types_pkg::*;
   localparam int MAX_CREDIT = 12;
   always @(posedge clk) begin
`ifndef SYNTHESIS
      chk_implies((rst_l!==1), |(feature_mode), |(bus4_st_valid), $sformatf("%s:%0d: in %m: ERROR: feature_mode -> bus4_st_valid violated", `__FILE__, `__LINE__))
`endif
      ;
`ifndef SYNTHESIS
      chk_always((rst_l!==1), |(ch0_outstanding + ch1_outstanding <= MAX_CREDIT), $sformatf("%s:%0d: in %m: ERROR: credit violated", `__FILE__, `__LINE__))
`endif
      ;
   end
endmodule
"#;

const TB_SV: &str = r#"
module testbench;
   logic clk, rst_l, feature_mode, bus4_st_valid;
   logic [3:0] ch0_outstanding, ch1_outstanding;
   initial clk = 0;
   always #5 clk = ~clk;
   // feature_mode rises one cycle BEFORE bus4_st_valid: chk_implies must fire.
   initial begin
      rst_l = 0; feature_mode = 0; bus4_st_valid = 0;
      ch0_outstanding = 0; ch1_outstanding = 0;
      #10 rst_l = 1;
      #5  feature_mode = 1;
      #10 bus4_st_valid = 1;
      #10 ch0_outstanding = 3;
      #10 ch1_outstanding = 4;
      #20 $finish;
   end
   datapath_glue u_datapath_glue (.clk(clk), .rst_l(rst_l), .feature_mode(feature_mode),
      .bus4_st_valid(bus4_st_valid), .ch0_outstanding(ch0_outstanding), .ch1_outstanding(ch1_outstanding));
endmodule
"#;

const HDR_STALE: &str = r#"
`ifndef _COMMON_TYPES_H_
`define _COMMON_TYPES_H_
package common_types_pkg;
   typedef logic [0:0] bit_t;
endpackage
`endif
"#;

const HDR_CURRENT: &str = r#"
`ifndef _COMMON_TYPES_H_
`define _COMMON_TYPES_H_
package common_types_pkg;
   typedef logic [0:0] bit_t;
   function void chk_always(input reset_cond, input cond, input string msg);
      assert(reset_cond || !($isunknown(cond) || !cond)) else $error("%s", msg);
   endfunction
   function void chk_implies(input reset_cond, input cond, input expr, input string msg);
      assert(reset_cond || !($isunknown(cond) || cond && $isunknown(expr) || cond && !expr)) else $error("%s", msg);
   endfunction
endpackage
`endif
"#;

const CHILD_READS_UNDECLARED: &str = r#"
module child(input logic clk);
  logic x;
  always @(posedge clk) x <= undefined_sig;
endmodule
module top;
  logic clk = 0;
  child c(.clk(clk));
  initial begin #1 clk = 1; #1 $finish; end
endmodule
"#;

#[test]
fn undeclared_signal_inside_a_sub_instance_is_an_error() {
    let d = dir("undef_sig");
    let src = d.join("top.sv");
    std::fs::write(&src, CHILD_READS_UNDECLARED).unwrap();
    let (ok, text) = run(&["--simulate", "-s", "top", src.to_str().unwrap()]);
    assert!(!ok, "undeclared signal in a child was accepted:\n{text}");
    assert!(
        text.contains("Undeclared identifier 'undefined_sig'") && text.contains("top.sv:4"),
        "no located diagnostic:\n{text}"
    );
}

/// `--dump-merged-sv` must produce a self-contained file: with the current
/// header first the dump carries the helper definitions and re-runs with the
/// same check firing; with the stale header first the run is rejected (and
/// names the RTL file), and the dump it still writes is rejected the same way
/// instead of silently dropping the checks.
#[test]
fn dump_merged_sv_is_self_contained_and_rejected_when_helpers_are_missing() {
    let d = dir("dump");
    let stale = d.join("stale");
    let current = d.join("current");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(stale.join("common_types.h"), HDR_STALE).unwrap();
    std::fs::write(current.join("common_types.h"), HDR_CURRENT).unwrap();
    let tb = d.join("tb.sv");
    let glue = d.join("datapath_glue.v");
    std::fs::write(&tb, TB_SV).unwrap();
    std::fs::write(&glue, GLUE_V).unwrap();
    let inc_stale = format!("+incdir+{}", stale.display());
    let inc_current = format!("+incdir+{}", current.display());
    let common = ["-s", "testbench", "--max-time", "100ns", "--module-timescale", "1ns/1ns"];

    // Control: current header first.
    let good = d.join("merged_full.sv");
    let mut args: Vec<&str> = vec![tb.to_str().unwrap(), glue.to_str().unwrap()];
    args.extend_from_slice(&common);
    args.extend_from_slice(&[&inc_current, &inc_stale, "--dump-merged-sv", good.to_str().unwrap()]);
    let (ok, text) = run(&args);
    assert!(ok, "control run failed:\n{text}");
    assert!(
        text.contains("feature_mode -> bus4_st_valid violated"),
        "chk_implies did not fire in the control run:\n{text}"
    );
    let dumped = std::fs::read_to_string(&good).unwrap();
    assert_eq!(
        dumped.matches("function void chk_").count(),
        2,
        "dump lacks the helper definitions:\n{dumped}"
    );
    let (ok, text) = run(&[good.to_str().unwrap(), "-s", "testbench", "--max-time", "100ns", "--module-timescale", "1ns/1ns"]);
    assert!(ok, "re-running the dump failed:\n{text}");
    assert!(
        text.contains("feature_mode -> bus4_st_valid violated"),
        "chk_implies did not fire from the dump:\n{text}"
    );

    // Field case: stale header first.
    let bad = d.join("merged.sv");
    let mut args: Vec<&str> = vec![tb.to_str().unwrap(), glue.to_str().unwrap()];
    args.extend_from_slice(&common);
    args.extend_from_slice(&[&inc_stale, &inc_current, "--dump-merged-sv", bad.to_str().unwrap()]);
    let (ok, text) = run(&args);
    assert!(!ok, "stale header run was accepted:\n{text}");
    assert!(
        text.contains("Undeclared identifier 'chk_implies'") && text.contains("datapath_glue.v:"),
        "diagnostic missing or unlocated:\n{text}"
    );
    assert!(text.contains("is shadowed by the search order"), "no shadow warning:\n{text}");
    if let Ok(dumped) = std::fs::read_to_string(&bad) {
        assert_eq!(dumped.matches("function void chk_").count(), 0);
        let (ok, text) = run(&[bad.to_str().unwrap(), "-s", "testbench", "--max-time", "100ns", "--module-timescale", "1ns/1ns"]);
        assert!(!ok && text.contains("Undeclared identifier 'chk_implies'"), "broken dump accepted:\n{text}");
    }
}
