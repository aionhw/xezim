//! IEEE 1800-2023 §21.7.1.2: the level count applies below EACH specified
//! module instance. A top-module argument must not discard the other scopes.
//! Check the actual VCD scope/variable declarations and decoded FST hierarchy.

use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use fst_reader::{FstHierarchyEntry, FstReader};

const SRC: &str = r#"
`timescale 1ns/1ps
module sub(input logic clk);
  logic [3:0] cnt = 0;
  always @(posedge clk) cnt <= cnt + 1;
endmodule
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  sub u_a(.clk(clk));
  sub u_b(.clk(clk));
  initial begin
    {DUMP}
    #42;
    $display("PASS cnt=%0d,%0d", u_a.cnt, u_b.cnt);
    $finish;
  end
endmodule
"#;

const TOP_A: &[&str] = &["top.clk", "top.u_a.clk", "top.u_a.cnt"];
const CHILDREN: &[&str] = &["top.u_a.clk", "top.u_a.cnt", "top.u_b.clk", "top.u_b.cnt"];
const ALL: &[&str] = &[
    "top.clk",
    "top.u_a.clk",
    "top.u_a.cnt",
    "top.u_b.clk",
    "top.u_b.cnt",
];

fn read_vcd(path: &Path) -> (Vec<String>, Vec<String>) {
    let mut stack = Vec::new();
    let mut scopes = Vec::new();
    let mut vars = Vec::new();
    for line in fs::read_to_string(path).expect("read VCD").lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("$scope") => {
                stack.push(words.nth(1).unwrap().to_string());
                scopes.push(stack.join("."));
            }
            Some("$upscope") => {
                stack.pop().expect("balanced VCD scopes");
            }
            Some("$var") => vars.push(format!("{}.{}", stack.join("."), words.nth(3).unwrap())),
            _ => {}
        }
    }
    assert!(stack.is_empty());
    scopes.sort();
    vars.sort();
    (scopes, vars)
}

fn read_fst(path: &Path) -> (Vec<String>, Vec<String>) {
    let mut reader = FstReader::open_and_read_time_table(BufReader::new(
        fs::File::open(path).expect("open FST"),
    ))
    .expect("decode FST header");
    let mut stack = Vec::new();
    let mut scopes = Vec::new();
    let mut vars = Vec::new();
    reader
        .read_hierarchy(|entry| match entry {
            FstHierarchyEntry::Scope { name, .. } => {
                stack.push(name);
                scopes.push(stack.join("."));
            }
            FstHierarchyEntry::UpScope => {
                stack.pop().expect("balanced FST scopes");
            }
            FstHierarchyEntry::Var { name, .. } => vars.push(format!(
                "{}.{}",
                stack.join("."),
                name.split_whitespace().next().unwrap()
            )),
            _ => {}
        })
        .expect("decode FST hierarchy");
    assert!(stack.is_empty());
    scopes.sort();
    vars.sort();
    (scopes, vars)
}

fn check(call: &str, fst_scopes: Option<&[&str]>, suppress: Option<&str>, expected: &[&str]) {
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_dumpvars_scope_list_{}_{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let dump = if fst_scopes.is_some() {
        String::new()
    } else {
        format!("$dumpfile(\"out.vcd\"); {call}")
    };
    fs::write(dir.join("dut.sv"), SRC.replace("{DUMP}", &dump)).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.current_dir(&dir)
        .args(["--no-cache", "--simulate", "-s", "top", "dut.sv"]);
    if let Some(scopes) = fst_scopes {
        cmd.args(["--fst", "out.fst"]);
        for scope in scopes {
            cmd.args(["--fst-scope", scope]);
        }
    } else {
        cmd.arg("--wave");
    }
    if let Some(scope) = suppress {
        cmd.args(["--wave-suppress", scope]);
    }
    let out = cmd.output().expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    // §4.7: sample at 42 ns, away from the clock edges and NBA updates.
    assert!(text.lines().any(|l| l == "PASS cnt=4,4"), "{text}");
    let (scopes, vars) = if fst_scopes.is_some() {
        read_fst(&dir.join("out.fst"))
    } else {
        read_vcd(&dir.join("out.vcd"))
    };
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(vars, expected, "{call} / {fst_scopes:?}: {text}");
    let mut expected_scopes = vec!["top"];
    if expected.iter().any(|v| v.starts_with("top.u_a.")) {
        expected_scopes.push("top.u_a");
    }
    if expected.iter().any(|v| v.starts_with("top.u_b.")) {
        expected_scopes.push("top.u_b");
    }
    assert_eq!(scopes, expected_scopes, "{call} / {fst_scopes:?}");
}

#[test]
fn vcd_top_and_child_keep_each_scopes_depth_in_either_order() {
    for call in ["$dumpvars(1, top, top.u_a);", "$dumpvars(1, top.u_a, top);"] {
        check(call, None, None, TOP_A);
    }
}

#[test]
fn fst_top_and_child_keep_each_scopes_depth_in_either_order() {
    for scopes in [["1:top", "1:top.u_a"], ["1:top.u_a", "1:top"]] {
        check("", Some(&scopes), None, TOP_A);
    }
}

#[test]
fn vcd_scope_list_guards() {
    for (call, expected) in [
        ("$dumpvars(1, top);", &["top.clk"][..]),
        ("$dumpvars(0, top, top.u_a);", ALL),
        ("$dumpvars(1, top.u_a, top.u_b);", CHILDREN),
        (
            "$dumpvars(1, top.u_a, top.u_b.cnt);",
            &["top.u_a.clk", "top.u_a.cnt", "top.u_b.cnt"][..],
        ),
        ("$dumpvars;", ALL),
        ("$dumpvars(1, top, top, top.u_a, top.u_a);", TOP_A),
    ] {
        check(call, None, None, expected);
    }
    // An exclusion still removes a sibling not explicitly requested.
    check("$dumpvars(0, top, top.u_a);", None, Some("top.u_b"), TOP_A);
    // The existing explicit-include policy re-admits the named child.
    check("$dumpvars(1, top, top.u_a);", None, Some("top.u_a"), TOP_A);
}

#[test]
fn fst_scope_list_guards() {
    for (scopes, expected) in [
        (&["1:top"][..], &["top.clk"][..]),
        (
            &["", "top.", "1:top.u_a"][..],
            &["top.u_a.clk", "top.u_a.cnt"][..],
        ),
        (&["1:top", "0:top.u_a"][..], TOP_A),
        (&["top", "top.u_a"][..], ALL),
        (&[][..], ALL),
        (&["1:top.u_a", "1:top.u_b"][..], CHILDREN),
        (&["1:top", "1:top", "1:top.u_a", "1:top.u_a"][..], TOP_A),
    ] {
        check("", Some(scopes), None, expected);
    }
    check("", Some(&["top", "top.u_a"]), Some("top.u_b"), TOP_A);
    check("", Some(&["1:top", "1:top.u_a"]), Some("top.u_a"), TOP_A);
}
