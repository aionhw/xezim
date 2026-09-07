//! §3.12.1 / §33.3: a subroutine declared at the top of a `-v` LIBRARY file
//! (an `include`d simulation-helper header, outside any module) is a `$unit`
//! subroutine of that file and callable unqualified from the file's modules.
//! Adoption used to drop those items, so the design-wide identifier check
//! rejected the call as undeclared. Reference-validated: q=6 / q=0.
use std::path::PathBuf;
use std::process::Command;

const HDR: &str = r#"
`ifndef RND_WRAP_H_DEFINED
`define RND_WRAP_H_DEFINED
`ifndef SYNTHESIS
  task rnd_get (input int n, output [31:0] r);
    begin r = 32'hfffffff0 | n[31:0]; end
  endtask
  function int rnd_mul (input int a, input int b);
    begin rnd_mul = a * b; end
  endfunction
`endif
`endif
"#;

const LEAF_FN: &str = r#"
`include "rnd_wrap.h"
module leaf_fn (input wire clk, input wire d, output reg [7:0] q);
  initial q = 0;
  always @(posedge clk) q <= rnd_mul(2, 3) + d;
endmodule
"#;

const LEAF_TASK: &str = r#"
`include "rnd_wrap.h"
module leaf_task (input wire clk, input wire d, output reg q);
  reg [31:0] rnd;
  initial q = 1'b0;
  always @(posedge clk) begin
    rnd_get(1, rnd);
    q <= d ^ rnd[0];
  end
endmodule
"#;

fn tb(leaf: &str) -> String {
    format!(
        r#"
module testbench;
  reg clk = 0, d = 0;
  wire [7:0] q;
  {leaf} u (.clk(clk), .d(d), .q(q));
  always #5 clk = ~clk;
  initial begin #22 $display("DONE q=%0d", q); $finish; end
endmodule
"#
    )
}

fn run(tb_src: &str, lib_src: &str, name: &str, dump: bool) -> (bool, String, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("unit_scope_lib").join(name);
    std::fs::create_dir_all(dir.join("incdir")).unwrap();
    std::fs::write(dir.join("incdir/rnd_wrap.h"), HDR).unwrap();
    let tb = dir.join("tb.sv");
    let lib = dir.join("leaf.v");
    std::fs::write(&tb, tb_src).unwrap();
    std::fs::write(&lib, lib_src).unwrap();
    let inc = format!("+incdir+{}", dir.join("incdir").display());
    let merged = dir.join("merged.sv");
    let mut args: Vec<String> = vec![
        tb.to_str().unwrap().into(), "-v".into(), lib.to_str().unwrap().into(),
        "-s".into(), "testbench".into(), "--max-time".into(), "50ns".into(),
        "--module-timescale".into(), "1ns/1ns".into(), inc,
    ];
    if dump {
        args.push("--dump-merged-sv".into());
        args.push(merged.to_str().unwrap().into());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_xezim")).args(&args).output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text, merged)
}

#[test]
fn unit_scope_function_in_a_library_file_is_callable() {
    let (ok, text, merged) = run(&tb("leaf_fn"), LEAF_FN, "fn", true);
    assert!(ok, "library $unit function rejected:\n{text}");
    assert!(text.contains("DONE q=6"), "wrong result:\n{text}");
    // The merged dump must carry the adopted library file, helper included.
    let dumped = std::fs::read_to_string(&merged).unwrap_or_default();
    assert!(dumped.contains("function int rnd_mul"), "dump lacks the library helper:\n{dumped}");
}

#[test]
fn unit_scope_task_in_a_library_file_is_callable() {
    let (ok, text, _) = run(&tb("leaf_task"), LEAF_TASK, "task", false);
    assert!(ok, "library $unit task rejected:\n{text}");
    assert!(text.contains("DONE q=1"), "task result did not land (d=0, rnd[0]=1):\n{text}");
}
