//! §7.4.1, §23.6: a constant scope selector addresses a stored memory;
//! an indexing expression after the memory still selects a packed word.

use std::{fs, process::Command};

#[test]
fn process_loops_compile_flat_and_instance_memory_words() {
    let dir = std::env::temp_dir().join(format!("xezim_process_paths_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.sv");
    fs::write(
        &path,
        r#"
module leaf;
  logic [7:0][39:0] mem;
endmodule
module bank;
  leaf inner();
endmodule
module top;
  bank lanes[0:1]();
  logic [7:0][39:0] flat [0:1];
  logic [63:0] sum = 0;
  initial begin
    for (int i=0; i<8; i++) flat[1][i] = 40'h8a00000000 + i;
    for (int i=0; i<8; i++) lanes[1].inner.mem[i] = 40'h8a00000000 + i;
    for (int i=0; i<8; i++) sum = sum + lanes[1].inner.mem[i];
    if (sum !== 64'h4500000001c || flat[1][7] !== 40'h8a00000007 ||
        lanes[1].inner.mem[0] !== 40'h8a00000000 ||
        lanes[0].inner.mem[7] !== 40'hxxxxxxxxxx) $fatal;
    $display("WORDS_PASS");
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "top"])
        .arg(path)
        .env("XEZIM_PROC_LOOP_STATS", "1")
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && text.lines().any(|l| l == "WORDS_PASS"),
        "{text}"
    );
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("[PROC-LOOP]") && l.contains("compiled=true"))
            .count(),
        3,
        "{text}"
    );
}
