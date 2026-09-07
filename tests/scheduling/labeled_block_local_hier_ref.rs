//! A labeled block's local is salted with its LABEL (`p1.v`) so two
//! processes' same-named locals stay distinct while a hierarchical reference
//! into the block (`tb.p1.v`, §23.6) keeps resolving.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
module tb;
  logic [7:0] r1, r2;
  initial begin : p1
    logic [7:0] v;
    v = 8'h10;
    #1 r1 = v;
  end
  initial begin : p2
    logic [7:0] v;
    v = 8'h1;
    #1 r2 = v;
  end
  initial #2 $display("HIER p1.v=%h tb.p1.v=%h p2.v=%h r1=%h r2=%h", p1.v, tb.p1.v, p2.v, r1, r2);
endmodule
"#;

#[test]
fn labeled_block_locals_stay_distinct_and_hierarchically_reachable() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("labeled_block_local");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("tb.sv");
    std::fs::write(&src, DESIGN).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb", src.to_str().unwrap(), "--no-cache", "--max-time", "10"])
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    assert!(
        text.contains("HIER p1.v=10 tb.p1.v=10 p2.v=01 r1=10 r2=01"),
        "labeled-block locals collided or lost hierarchical reach:\n{text}"
    );
}
