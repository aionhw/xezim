//! §7.4.1 — a CA writing `d[i][hi:lo]` on a packed 2-D vector strides by
//! the DECLARED element width, not the slice width. Reference-validated
//! (pipeline-stage TB: instance data landed in the wrong element whenever
//! slice width != 64).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
module tb;
  logic [4:0][63:0] d;
  logic [31:0] r1;
  logic [15:0] r4;
  assign d[1][31:0] = 32'haabbccdd;
  assign d[4][15:0] = 16'h1234;
  assign d[3][63:0] = 64'h5555;
  initial begin
    #1;
    r1 = d[1][31:0];
    r4 = d[4][15:0];
  end
endmodule
"#;

#[test]
fn packed2d_slice_ca_uses_element_stride() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    assert_eq!(u(&sim, "r1"), 0xaabbccdd, "d[1][31:0] lands in element 1");
    assert_eq!(u(&sim, "r4"), 0x1234, "d[4][15:0] lands in element 4");
}
