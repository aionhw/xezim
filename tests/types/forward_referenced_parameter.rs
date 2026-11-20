//! IEEE 1800-2023 §6.5 and §6.20: declare parameter dependencies before
//! using them to size a packed range. The relaxed CLI fixture separately
//! covers the parameter fixpoint used for legacy forward references.

use xezim::simulate;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

const SRC: &str = r#"
`define MACRO_SIZE 4096

package sz_pkg;
  localparam int PKG_SIZE = 4096;
  localparam int PKG_LOG  = $clog2(PKG_SIZE);
endpackage

function automatic int log2_fn(input int v);
  int r; begin r = 0; while ((1 << r) < v) r = r + 1; log2_fn = r; end
endfunction

module tb;
  import sz_pkg::*;
  localparam int LOCAL_SIZE = 4096;

  localparam int L1 = $clog2(LOCAL_SIZE) - 6;        // backward ref
  localparam int L2 = $clog2(sz_pkg::PKG_SIZE) - 6;  // package-scoped
  localparam int L3 = sz_pkg::PKG_LOG - 6;           // package param
  localparam int L4 = $clog2(`MACRO_SIZE) - 6;       // macro
  localparam int L5 = log2_fn(LOCAL_SIZE) - 6;       // user sizing function
  localparam int L6 = $clog2(LOCAL_SIZE/64);
  localparam int IMPORTED_SIZE = PKG_SIZE;
  localparam int L7 = $clog2(IMPORTED_SIZE) - 6;
  localparam int MID = LOCAL_SIZE / 64;
  localparam int L8  = $clog2(MID);

  // Widths built from each: these collapse when the parameter reads 0.
  logic [L1-1:0] w1; logic [L2-1:0] w2; logic [L3-1:0] w3; logic [L4-1:0] w4;
  logic [L5-1:0] w5; logic [L6-1:0] w6; logic [L7-1:0] w7; logic [L8-1:0] w8;

  int v1, v2, v3, v4, v5, v6, v7, v8;
  int b1, b2, b3, b4, b5, b6, b7, b8;
  initial begin
    #1;
    v1 = L1; v2 = L2; v3 = L3; v4 = L4; v5 = L5; v6 = L6; v7 = L7; v8 = L8;
    b1 = $bits(w1); b2 = $bits(w2); b3 = $bits(w3); b4 = $bits(w4);
    b5 = $bits(w5); b6 = $bits(w6); b7 = $bits(w7); b8 = $bits(w8);
  end
endmodule
"#;

#[test]
fn parameter_values_and_dependent_widths() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Every form evaluates to 6 ...
    for n in ["v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8"] {
        assert_eq!(get(&sim, n) as u32, 6, "{} should be 6", n);
    }
    // ... and every dependent declaration is 6 bits wide.
    for n in ["b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8"] {
        assert_eq!(get(&sim, n) as u32, 6, "{} should be 6 bits", n);
    }
}

/// In relaxed mode, resolve a forward chain before sizing its users. The
/// subtraction used to underflow when the size was provisionally zero.
#[test]
fn relaxed_forward_references_resolve_before_sizing() {
    const CHAIN: &str = r#"
module tb;
  localparam int W = $clog2(SIZE) - 6;
  localparam int SIZE = 1 << B;
  localparam int A = B + 1;
  localparam int B = C + 1;
  localparam int C = D + 1;
  localparam int D = 10;
  logic [W-1:0] bus;
  logic [A-1:0] chain_bus;
  initial $display("P|%0d %0d %0d %0d %0d %0d %0d %0d",
                   D, C, B, A, SIZE, W, $bits(bus), $bits(chain_bus));
endmodule
"#;
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("relaxed_forward_params");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("forward_params.sv");
    std::fs::write(&path, CHAIN).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "--no-cache",
            "--relax-use-before-decl",
            "-s",
            "tb",
        ])
        .arg(path)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line == "P|10 11 12 13 4096 6 6 13"),
        "{stdout}"
    );
}
