//! §20.6.2 — `$bits(<signal>)` inside a typedef's packed dims sizes from
//! the signal (`typedef logic[$bits(sig)-1:0] t;` collapsed to 1 bit, so
//! casts through it and part-selects of its variables read x).
//! Reference-validated (customer by_hand probe).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
package P;
  typedef struct packed {
    logic [1:0][63:0] d;
    logic [1:0][7:0]  m;
    logic [1:0]       a;
  } wt_t; // 146 bits
endpackage
module tb;
  import P::*;
  P::wt_t [0:0][1:0] w;
  typedef logic [$bits(w)-1:0] flat_t;
  flat_t f;
  logic [7:0] plain;
  typedef logic [$bits(plain)-1:0] tp_t;
  tp_t a8;
  int wb, w_sel;
  initial begin
    w = '0;
    w[0][0].d[0] = 64'h22e0000;
    f = flat_t'(w);
    wb = $bits(f);
    w_sel = f[81:18];
    a8 = 8'hff;
  end
endmodule
"#;

#[test]
fn typedef_bits_of_signal_dims() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    assert_eq!(u(&sim, "wb"), 292, "typedef sized by $bits(292-bit signal)");
    // d[0] occupies exactly bits [81:18] of the flat value (18 = amask 2
    // + mask 16), so the select reads the raw stored 64'h22e0000.
    assert_eq!(u(&sim, "w_sel") as u32, 0x22e0000, "part-select through cast");
    assert_eq!(u(&sim, "a8"), 0xff, "simple $bits(sig) typedef holds value");
}
