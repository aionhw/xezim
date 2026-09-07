//! §10.6.2: `release` of a net returns it to its continuous drivers — also
//! when the release runs inside a level-sensitive `always @(en)` / `@*`
//! block or a process resumed by `@(en)`. Those bodies execute while the
//! combinational settle is in progress, and the re-drive of the released
//! net used to be lost there: the net kept the forced value until its
//! driver happened to change again.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
module sub(input logic clk, input logic we, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] r; always @(posedge clk) if (we) r <= d; assign q = r;
endmodule
module mid(input logic clk, input logic we, input logic [7:0] d, output logic [7:0] q);
  sub u_sub(.*);
endmodule
module top;
  logic clk = 0, we = 0, en_lvl = 0, en_star = 0, en_proc = 0, en_top = 0;
  logic [7:0] d = 0, q;
  mid u(.*);
  always #5 clk = ~clk;
  always @(en_lvl) if (en_lvl) force u.q = 8'h0; else release u.q;
  always @* if (en_star) force u.q = '0; else release u.q;
  always begin @(en_proc); if (en_proc) force u.q = 8'h0; else release u.q; end
  always @(en_top) if (en_top) force q = 8'h0; else release q;
  task cycle(string tag, ref logic en);
    en = 1; #1 $display("%s forced q=%h", tag, q);
    en = 0; #1 $display("%s released q=%h", tag, q);
  endtask
  initial begin
    d = 8'ha5; we = 1; @(posedge clk); #1 we = 0;
    cycle("level", en_lvl);
    cycle("star", en_star);
    cycle("process", en_proc);
    cycle("top", en_top);
    $finish;
  end
endmodule
"#;

fn run(jit: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("release_level_sensitive");
    std::fs::create_dir_all(&dir).unwrap();
    // One file per variant: the two variants run on parallel test threads
    // and a shared file can be truncated under the other run.
    let sv = dir.join(if jit { "t_jit.sv" } else { "t_default.sv" });
    std::fs::write(&sv, DESIGN).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--simulate", "-s", "top", "--no-cache", sv.to_str().unwrap()]);
    if jit {
        cmd.env("XEZIM_JIT", "1");
    }
    let output = cmd.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn check(text: &str) {
    for tag in ["level", "star", "process", "top"] {
        assert!(text.contains(&format!("{tag} forced q=00")), "{tag}: force not applied:\n{text}");
        assert!(text.contains(&format!("{tag} released q=a5")), "{tag}: release did not re-drive the net:\n{text}");
    }
}

#[test]
fn release_inside_level_sensitive_block_redrives_the_net() {
    check(&run(false));
}

#[test]
fn release_inside_level_sensitive_block_redrives_the_net_jit() {
    check(&run(true));
}
