//! §6.24.1: a cast to an integer atom type (`int'`, `shortint'`, `longint'`)
//! in a compiled process yields its operand's value at the target width.

use std::path::Path;
use std::process::Command;

fn run(name: &str, source: &str, compile_methods: bool) -> String {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.sv"));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "top"])
        .arg(path)
        .env(
            "XEZIM_COMPILE_METHODS",
            if compile_methods { "1" } else { "0" },
        )
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "simulation failed:\n{text}");
    text
}

/// Integral operands of atom casts in an edge-triggered block, beside size
/// and typedef casts of the same operands.
#[test]
fn atom_casts_of_integral_operands_keep_their_value_in_compiled_blocks() {
    let src = r#"
module top;
  typedef logic [11:0] w12_t;
  logic clk = 0;
  logic [7:0] a = 8'd200, b = 8'd100;
  logic signed [7:0] n = -8'sd3;
  int s; shortint t; longint L; logic [15:0] h; logic [3:0] q; int u; w12_t v;
  int cnt = 0;
  always #5 clk = ~clk;
  always @(posedge clk) begin
    s = int'(a + b);
    t = shortint'(a + b);
    L = longint'(n);
    h = 16'(a) + 16'(b);
    q = 4'(a + b);
    u = int'(n) * 2;
    v = w12_t'(a + b);
    $display("T|c=%0d s=%0d t=%0d L=%0d h=%0d q=%0d u=%0d v=%0d", cnt, s, t, L, h, q, u, v);
    a = a + 8'd1;
    cnt = cnt + 1;
    if (cnt == 3) $finish;
  end
endmodule
"#;
    for compile_methods in [true, false] {
        let text = run("compiled_atom_cast_operand", src, compile_methods);
        for want in [
            "T|c=0 s=300 t=300 L=-3 h=300 q=12 u=-6 v=300",
            "T|c=1 s=301 t=301 L=-3 h=301 q=13 u=-6 v=301",
            "T|c=2 s=302 t=302 L=-3 h=302 q=14 u=-6 v=302",
        ] {
            assert!(
                text.lines().any(|line| line == want),
                "missing `{want}` (compile_methods={compile_methods}):\n{text}"
            );
        }
    }
}
