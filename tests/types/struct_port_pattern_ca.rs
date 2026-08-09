//! §10.9.2/§23.3 pattern CAs over struct ports + §13.3 paren-less task
//! enable. Reference-validated (agentJ audit; customer prebuf-stall
//! suspect class).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
module s (input pair_t p, output pair_t r);
  assign r = '{hi: p.lo, lo: p.hi};
endmodule
module tb;
  pair_t a, b;
  s u(.p(a), .r(b));
  int ran = 0;
  task automatic mark; ran = 1; endtask
  logic [7:0] snap;
  initial begin
    a = 8'hA5;
    #1 snap = b;
    mark;
  end
endmodule
"#;

#[test]
fn struct_port_pattern_and_noparen_task() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    assert_eq!(u(&sim, "snap"), 0x5a, "pattern CA swaps struct-port fields");
    assert_eq!(u(&sim, "ran"), 1, "paren-less task enable runs");
}
