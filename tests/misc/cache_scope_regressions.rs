//! IEEE 1800-2023 §10.3.3, §10.9.1, §20.6.2: loading prepared tables or
//! elaborated artifacts preserves transition delays and lexical constants.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run simulator");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{args:?}: {text}");
    text
}

fn remove_prepared(dir: &Path) -> usize {
    let mut count = 0;
    for entry in std::fs::read_dir(dir).expect("read cache") {
        let path = entry.expect("cache entry").path();
        if path.is_dir() {
            count += remove_prepared(&path);
        } else if path.extension().is_some_and(|e| e == "xezcomb") {
            std::fs::remove_file(path).expect("remove prepared tables");
            count += 1;
        }
    }
    count
}

fn lifecycle(source: &str, prepared: bool) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_cache_scope_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create scratch");
    let scratch = Scratch(dir);
    std::fs::write(scratch.0.join("top.sv"), source).expect("write source");
    let check = |text: &str| assert!(text.contains("T|PASS") && !text.contains("FAIL"), "{text}");
    check(&run(&scratch.0, &["--no-cache", "-s", "top", "top.sv"]));
    check(&run(
        &scratch.0,
        &["--cache-dir", "cache", "-s", "top", "top.sv"],
    ));
    let warm = run(&scratch.0, &["--cache-dir", "cache", "-s", "top", "top.sv"]);
    check(&warm);
    if prepared {
        assert!(
            warm.contains("prepared-comb hit"),
            "warm path was not exercised: {warm}"
        );
    }
    let removed = remove_prepared(&scratch.0.join("cache"));
    if prepared {
        assert!(removed > 0, "no prepared table was stored");
    }
    check(&run(
        &scratch.0,
        &["--cache-dir", "cache", "-s", "top", "top.sv"],
    ));
    for compression in ["none", "3"] {
        run(
            &scratch.0,
            &[
                "--compile",
                "--no-cache",
                "--artifact-compression",
                compression,
                "-s",
                "top",
                "-o",
                "design.xezbc",
                "top.sv",
            ],
        );
        check(&run(&scratch.0, &["design.xezbc"]));
    }
}

#[test]
fn prepared_tables_keep_falling_and_turnoff_delays() {
    for (rise, fall, off) in [(2, 4, 6), (2, 2, 6), (0, 4, 6), (2, 0, 6), (2, 4, 0)] {
        let fall_check = if fall > 1 {
            format!(
                "#{}; if (w !== 4'h3) $fatal(1,\"FAIL early fall\"); #2;",
                fall - 1
            )
        } else {
            "#1;".to_string()
        };
        let off_check = if off > 1 {
            format!(
                "#{}; if (w !== 4'h0) $fatal(1,\"FAIL early turnoff\"); #2;",
                off - 1
            )
        } else {
            "#1;".to_string()
        };
        lifecycle(
            &format!(
                r#"
`timescale 1ns/1ps
module top;
  logic [3:0] x = 3;
  wire [3:0] w;
  assign #({rise}, {fall}, {off}) w = x;
  initial begin
    #10 x = 0;
    {fall_check}
    if (w !== 4'h0) $fatal(1,"FAIL missing fall");
    x = 'z;
    {off_check}
    if (w !== 4'hz) $fatal(1,"FAIL missing turnoff");
    $display("T|PASS"); $finish;
  end
endmodule
"#
            ),
            true,
        );
    }
}

#[test]
fn loaded_patterns_use_the_design_parameter_bindings() {
    let patterns = [
        (
            "wire [31:0] a[4];",
            "'{P-1:x, P+1:x+1, default:0}",
            "a[0] == 0 && a[1] == 1 && a[2] == 0 && a[3] == 2",
        ),
        (
            "wire [31:0] a[5:2];",
            "'{P:x, P+2:x+1, default:0}",
            "a[2] == 1 && a[3] == 0 && a[4] == 2 && a[5] == 0",
        ),
        (
            "wire [31:0] a[4];",
            "'{P{x,x+1}}",
            "a[0] == 1 && a[1] == 2 && a[2] == 1 && a[3] == 2",
        ),
        (
            "wire [31:0] a[2][4];",
            "'{default:'{P-1:x, default:0}}",
            "a[0][1] == 1 && a[1][1] == 1 && a[0][0] == 0 && a[1][3] == 0",
        ),
        (
            "wire [31:0] a[4];",
            "'{P-1:x, default:x+2}",
            "a[0] == 3 && a[1] == 1 && a[2] == 3 && a[3] == 3",
        ),
    ];
    for (decl, pattern, condition) in patterns {
        lifecycle(
            &format!(
                r#"
module top;
  localparam int P = 2;
  int x = 1;
  {decl}
  assign a = {pattern};
  initial begin
    #5;
    if (!({condition})) $fatal(1,"FAIL parameter pattern");
    $display("T|PASS"); $finish;
  end
endmodule
"#
            ),
            false,
        );
    }
}

#[test]
fn loaded_struct_members_keep_generate_typedef_widths() {
    for (definition, field, width) in [
        ("struct {logic [$bits(W)-1:0] x;}", "x", 8),
        ("struct packed {logic [$bits(W)-1:0] x;}", "x", 8),
        (
            "struct {struct {logic [$bits(W)-1:0] x;} inner;}",
            "inner.x",
            8,
        ),
        (
            "union packed {logic [$bits(W)-1:0] x; logic [7:0] y;}",
            "x",
            8,
        ),
        ("struct {nibble_t [$bits(W)-1:0] x;}", "x", 32),
    ] {
        lifecycle(
            &format!(
                r#"
module top;
  if (1) begin : a
    typedef logic [7:0] W;
    typedef logic [3:0] nibble_t;
    typedef {definition} S;
    S v, w;
    initial begin
      #1 v.{field} = '1;
      w = v;
      #1;
      if (w.{field} !== {{{width}{{1'b1}}}}) $fatal(1,"FAIL truncated member");
      $display("T|PASS"); $finish;
    end
  end
endmodule
"#
            ),
            false,
        );
    }
}
