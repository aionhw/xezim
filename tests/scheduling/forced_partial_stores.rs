//! IEEE 1800-2023 §10.6.2: a forced variable keeps its forced value under
//! procedural writes until it is released, also under a part-select or a
//! bit-select store. The VM's dynamic bit and range stores of a wide vector,
//! and the JIT's constant range store, wrote the signal table directly and
//! changed a forced value. Expected T| lines are the reference simulator's.

use std::process::Command;

const SRC: &str = r#"
module top;
  logic clk = 0;
  logic [7:0] v = 8'h00;
  logic [15:0] w = 16'h0000;
  logic [71:0] z = '0;
  logic [3:0] n = 0;
  logic [2:0] m = 0;
  always #5 clk = ~clk;
  // Constant range stores: a compiled (and, with the JIT, native) block.
  always @(posedge clk) begin
    n = n + 1;
    v[3:0] = n;
    w[11:4] = {n, n};
  end
  // Dynamic bit and range stores of a wide vector.
  always @(posedge clk) begin
    m = m + 1;
    z[70:67] = {1'b0, m};
    z[m] = 1'b1;
    z[m+8 +: 2] = 2'b11;
  end
  initial begin
    #12 force v = 8'hA5;
    force w = 16'hBEEF;
    force z = 72'h5;
    #20 $display("T|forced|v=%h w=%h z=%h", v, w, z);
    release v;
    release w;
    release z;
    #1 $display("T|released|v=%h w=%h z=%h", v, w, z);
    #10 $display("T|after|v=%h w=%h z=%h n=%0d", v, w, z, n);
    $finish;
  end
endmodule
"#;

const EXPECTED: [&str; 3] = [
    "T|forced|v=a5 w=beef z=000000000000000005",
    "T|released|v=a5 w=beef z=000000000000000005",
    "T|after|v=a4 w=b44f z=200000000000003015 n=4",
];

fn run(jit: bool) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!(
        "xezim_forced_partial_stores_{}_{}",
        std::process::id(),
        jit
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("forced.sv");
    std::fs::write(&path, SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", path.to_str().unwrap()])
        .env("XEZIM_JIT", if jit { "1" } else { "0" })
        .env("XEZIM_CACHE_DIR", dir.join("cache"))
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn forced_partial_stores_keep_the_forced_value() {
    assert_eq!(run(false), EXPECTED);
}

#[cfg(feature = "jit")]
#[test]
fn forced_partial_stores_keep_the_forced_value_under_the_jit() {
    assert_eq!(run(true), EXPECTED);
}
