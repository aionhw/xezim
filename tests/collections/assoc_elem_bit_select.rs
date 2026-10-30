//! §7.8 / §11.5.1: `m[k][i]` on a ONE-dimensional associative array of
//! vectors selects bit `i` of the element `m[k]`. The read was taken for a
//! two-key lookup `m[k][i]` of a multidimensional array: it found no such
//! key and returned the element type's default (`xxxx`, the whole element
//! width) instead of the bit. A true `m[int][int]` keeps compound keys.

use xezim::simulate;

fn bits(sim: &xezim::compiler::Simulator, n: &str) -> String {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_bin_string()
}

const SRC: &str = r#"
module tb;
  class key_t; endclass
  logic [3:0] by_int[int];
  logic [3:0] by_obj[key_t];
  logic [3:0] two[int][int];
  logic b0, b1, b2, h1;
  logic [3:0] t12, t15;
  initial begin
    key_t k = new;
    by_int[3] = 4'b0z1z;
    by_int[3][0] = 1'b0;
    b0 = by_int[3][0];
    b1 = by_int[3][1];
    b2 = by_int[3][2];
    by_obj[k] = 4'b0z1z;
    h1 = by_obj[k][1];
    two[1][2] = 4'b1010;
    two[1][5] = 4'b0101;
    t12 = two[1][2];
    t15 = two[1][5];
  end
endmodule
"#;

#[test]
fn assoc_element_bit_select() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(bits(&sim, "b0"), "0", "bit 0 after a bit write");
    assert_eq!(bits(&sim, "b1"), "1");
    assert_eq!(bits(&sim, "b2"), "z");
    assert_eq!(bits(&sim, "h1"), "1", "class-handle key");
    assert_eq!(bits(&sim, "t12"), "1010", "a 2-D array keeps compound keys");
    assert_eq!(bits(&sim, "t15"), "0101");
}
