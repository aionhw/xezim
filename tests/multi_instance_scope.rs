//! An AST-evaluated process (here a constrained `std::randomize`) inside a
//! *multiply-instantiated* module must resolve its bare signal names against
//! ITS OWN instance, and `%m` must report that instance's scope. Previously
//! every instance's process shared one name-resolution scope, so the 2nd
//! instance's `std::randomize(idx)` wrote the 1st instance's signal (leaving
//! its own X) and `%m` always printed the top module name.

use xezim::simulate;

const SRC: &str = r#"
module probe();
  bit [3:0] idx;
  initial begin
    #1;
    void'(std::randomize(idx) with { idx <= 6; });
    $display("%m idx=%0d", idx);
  end
endmodule
module TB;
  probe p0();
  probe p1();
  initial #5 $finish;
endmodule
"#;

fn get(sim: &xezim::compiler::Simulator, name: &str) -> Option<u64> {
    sim.get_signal(name).and_then(|v| v.to_u64())
}

#[test]
fn multiply_instantiated_randomize_and_m_are_per_instance() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // Both instances' own signal must be randomized within the constraint —
    // neither left unwritten (X ⇒ to_u64 None), and both <= 6.
    let a = get(&sim, "p0.idx").expect("p0.idx unresolved/X");
    let b = get(&sim, "p1.idx").expect("p1.idx unresolved/X — 2nd instance not randomized");
    assert!(a <= 6, "p0.idx={} violates constraint", a);
    assert!(b <= 6, "p1.idx={} violates constraint", b);
    // %m must report each instance's scope, not just the top.
    let joined: String = sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("TB.p0"), "%m did not report TB.p0 scope:\n{}", joined);
    assert!(joined.contains("TB.p1"), "%m did not report TB.p1 scope:\n{}", joined);
}
