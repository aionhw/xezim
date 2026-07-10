//! `std::randomize` on packed and unpacked struct variables with inline
//! constraints (IEEE 1800-2023 §18.12). Previously a scope randomize of a
//! struct left its members unrandomized (a flat assign can't reach the fields)
//! and field-level constraints (`pkt.data < 100`) were ignored. Now each
//! member is randomized and the constraints bind.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct packed { bit [31:0] data; bit [31:0] addr; bit vld; } pkt_t;
  typedef struct { rand bit [31:0] data; rand bit [31:0] addr; } upkt_t;
  pkt_t  p;
  upkt_t u;
  int bad = 0;
  int distinct_p = 0;
  int distinct_u = 0;
  int prev_pd = -1;
  int prev_ud = -1;
  initial begin
    for (int i = 0; i < 20; i++) begin
      void'(std::randomize(p) with { p.data < 100; p.addr inside {[35:255]}; });
      void'(std::randomize(u) with { u.data < 100; u.addr inside {[35:255]}; });
      if (!(p.data < 100 && p.addr >= 35 && p.addr <= 255)) bad = bad + 1;
      if (!(u.data < 100 && u.addr >= 35 && u.addr <= 255)) bad = bad + 1;
      if (p.data != prev_pd) distinct_p = distinct_p + 1;
      if (u.data != prev_ud) distinct_u = distinct_u + 1;
      prev_pd = p.data;
      prev_ud = u.data;
    end
  end
endmodule
"#;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> u64 {
    sim.get_signal(name)
        .or_else(|| sim.get_signal(&format!("tb.{}", name)))
        .unwrap_or_else(|| panic!("signal not found: {}", name))
        .to_u64()
        .unwrap_or_else(|| panic!("signal {} not u64-able", name))
}

#[test]
fn struct_scope_randomize_honors_constraints() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    assert_eq!(get(&sim, "bad"), 0, "some randomize result violated its constraint");
    // Fields must actually vary across draws (not stuck at a constant).
    assert!(
        get(&sim, "distinct_p") >= 2,
        "packed struct data never changed across 20 draws (not randomized)"
    );
    assert!(
        get(&sim, "distinct_u") >= 2,
        "unpacked struct data never changed across 20 draws (not randomized)"
    );
}
