//! `$dumpvars(0, tb_top.u)` called from a DIFFERENT top-level
//! module dumped nothing.
//!
//! IEEE 1800-2023 §21.7.1.2 lets `$dumpvars` name any scope, and §23.6
//! resolves a hierarchical name that starts with a top-level module's name
//! from any scope. Elaboration resolves such a cross-top name to its
//! absolute path and marks it rooted (`$root`), and the `$dumpvars` argument
//! renderer rejected every rooted name: the argument was "not a scope or
//! variable name", and the dump selected nothing. The same call from inside
//! `tb_top`, or a bare `tb_top`, always worked. `$fsdbDumpvars` went through
//! the signal resolver instead, which cut the path to `u`.

use std::fs;
use std::process::Command;

const SRC: &str = r#"
`timescale 1ns/1ps
module leaf(input logic clk);
  logic [1:0] lv = 0;
  always @(posedge clk) lv <= lv + 1;
endmodule
module counter(input logic clk, output logic [3:0] cnt);
  initial cnt = 0;
  always @(posedge clk) cnt <= cnt + 1;
  leaf ul (.clk(clk));
endmodule
module tb_top;
  logic clk = 0;
  logic [3:0] c;
  always #5 clk = ~clk;
  counter u (.clk(clk), .cnt(c));
  for (genvar g = 0; g < 2; g++) begin : gen
    leaf ug (.clk(clk));
  end
  {IN_TB}
  initial begin #30; $finish; end
endmodule
module dump_ctl;
  {IN_CTL}
endmodule
"#;

/// Run the two-top design with `in_tb` added to `tb_top` and `in_ctl` to
/// `dump_ctl`, and return the run's output and the fully-qualified `$var`
/// names of `out.vcd`, sorted.
fn dumped(tag: &str, in_tb: &str, in_ctl: &str) -> (String, Vec<String>) {
    let dir =
        std::env::temp_dir().join(format!("xezim_dumpvars_xtop_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("dut.sv");
    fs::write(
        &sv,
        SRC.replace("{IN_TB}", in_tb).replace("{IN_CTL}", in_ctl),
    )
    .expect("write sv");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "--wave"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    let vcd = fs::read_to_string(dir.join("out.vcd")).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    let mut stack: Vec<String> = Vec::new();
    let mut vars = Vec::new();
    for line in vcd.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("$scope") => stack.push(it.nth(1).unwrap_or_default().to_string()),
            Some("$upscope") => {
                stack.pop();
            }
            Some("$var") => {
                let name = it.nth(3).unwrap_or_default();
                vars.push(format!("{}.{}", stack.join("."), name));
            }
            _ => {}
        }
    }
    vars.sort();
    (text, vars)
}

fn from_ctl(tag: &str, args: &str) -> (String, Vec<String>) {
    dumped(
        tag,
        "",
        &format!("initial begin $dumpfile(\"out.vcd\"); $dumpvars({args}); end"),
    )
}

#[test]
fn sub_instance_of_another_top_is_dumped() {
    let (text, vars) = from_ctl("sub", "0, tb_top.u");
    assert!(!text.contains("was ignored"), "{text}");
    assert_eq!(
        vars,
        [
            "tb_top.u.clk",
            "tb_top.u.cnt",
            "tb_top.u.ul.clk",
            "tb_top.u.ul.lv"
        ],
        "{text}"
    );
    // The same call made inside `tb_top` is the oracle.
    let (_, inside) = dumped(
        "inside",
        "initial begin $dumpfile(\"out.vcd\"); $dumpvars(0, tb_top.u); end",
        "",
    );
    assert_eq!(vars, inside);
}

/// §21.7.1.4: the level count still applies below the named scope.
#[test]
fn level_count_applies_below_another_tops_scope() {
    let (text, vars) = from_ctl("level", "1, tb_top.u");
    assert_eq!(vars, ["tb_top.u.clk", "tb_top.u.cnt"], "{text}");
}

/// A generate-block instance, a variable, and an explicit `$root.` path of
/// the other top.
#[test]
fn generate_scope_variable_and_root_paths_of_another_top() {
    let (text, vars) = from_ctl("mixed", "0, tb_top.gen[1].ug, tb_top.c, $root.tb_top.u.ul");
    assert!(!text.contains("was ignored"), "{text}");
    assert_eq!(
        vars,
        [
            "tb_top.c",
            "tb_top.gen[1].ug.clk",
            "tb_top.gen[1].ug.lv",
            "tb_top.u.ul.clk",
            "tb_top.u.ul.lv"
        ],
        "{text}"
    );
}

/// `$fsdbDumpvars` takes the same scope from another top.
#[test]
fn fsdb_dumpvars_takes_another_tops_sub_instance() {
    let (text, _) = dumped("fsdb", "", "initial $fsdbDumpvars(0, tb_top.u);");
    assert!(text.contains("[FST] dumping 4 signals"), "{text}");
    assert!(!text.contains("matched no signals"), "{text}");
}
