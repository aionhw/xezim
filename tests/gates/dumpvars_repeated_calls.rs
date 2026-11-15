//! IEEE 1800-2023 §21.7.1.2: "The $dumpvars task can be invoked as often as
//! desired throughout the model (for example, within various blocks), but
//! the execution of all the $dumpvars tasks shall be at the same simulation
//! time." Dumping starts at the end of that time unit, so every call made at
//! the start time adds its objects, each call with its own level count. Only
//! the first call used to select anything: a second call at the same time
//! was taken as a checkpoint of the running dump.

use std::fs;
use std::process::Command;

const SRC: &str = r#"
`timescale 1ns/1ps
module leaf;
  logic [3:0] lv = 0;
  always #3 lv <= lv + 1;
endmodule
module mid;
  logic m0 = 0;
  always #4 m0 <= ~m0;
  leaf u_leaf();
endmodule
module tb;
  logic a = 0, b = 0, c = 0;
  for (genvar g = 0; g < 2; g++) begin : gen
    mid u_mid();
  end
  mid u_top();
  initial begin
    $dumpfile("out.vcd");
    {CALLS}
    #20 $finish;
  end
  {OTHER}
  always #7 a <= ~a;
endmodule
"#;

/// Run the design with `calls` in the first initial block and `other` as an
/// extra module item, and return the fully-qualified `$var` names, sorted.
fn dumped(tag: &str, calls: &str, other: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!(
        "xezim_dumpvars_repeat_{tag}_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("dut.sv");
    fs::write(&sv, SRC.replace("{CALLS}", calls).replace("{OTHER}", other)).expect("write sv");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "--wave", "-s", "tb"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = fs::read_to_string(dir.join("out.vcd")).expect("read vcd");
    let _ = fs::remove_dir_all(&dir);
    let mut stack: Vec<String> = Vec::new();
    let mut vars = Vec::new();
    for line in text.lines() {
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
    vars
}

#[test]
fn two_calls_in_one_block_dump_both_selections() {
    assert_eq!(
        dumped("plain", "$dumpvars(0, tb.a); $dumpvars(0, tb.b);", ""),
        ["tb.a", "tb.b"]
    );
    assert_eq!(
        dumped(
            "scope",
            "$dumpvars(0, tb.a); $dumpvars(0, tb.gen[0].u_mid);",
            ""
        ),
        ["tb.a", "tb.gen[0].u_mid.m0", "tb.gen[0].u_mid.u_leaf.lv"]
    );
}

/// Each call keeps its own level count: level 1 of `u_top` is its own
/// variable, level 0 of `gen[1].u_mid` reaches into `u_leaf`.
#[test]
fn each_call_keeps_its_own_levels() {
    assert_eq!(
        dumped(
            "levels",
            "$dumpvars(1, tb.u_top); $dumpvars(0, tb.gen[1].u_mid);",
            ""
        ),
        [
            "tb.gen[1].u_mid.m0",
            "tb.gen[1].u_mid.u_leaf.lv",
            "tb.u_top.m0"
        ]
    );
}

/// Calls in different blocks at the start time add up the same way.
#[test]
fn calls_in_two_blocks_at_the_start_time_add_up() {
    assert_eq!(
        dumped(
            "blocks",
            "$dumpvars(0, tb.a);",
            "initial $dumpvars(0, tb.c);"
        ),
        ["tb.a", "tb.c"]
    );
}

/// A call with no argument still selects the whole design.
#[test]
fn a_call_without_arguments_still_selects_everything() {
    let all = dumped("all", "$dumpvars(0, tb.a); $dumpvars;", "");
    assert!(all.contains(&"tb.b".to_string()), "{all:?}");
    assert!(
        all.contains(&"tb.gen[1].u_mid.u_leaf.lv".to_string()),
        "{all:?}"
    );
}
