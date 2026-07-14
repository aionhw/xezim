//! Parser-level LRM gaps fixed together: things that were parsed and
//! silently DISCARDED or mis-shaped.
//!
//!   4'(x)                 literal size cast lost its width (§6.24.1)
//!   for (int j=0, k=10;)  the k entry parsed as an assignment to an
//!                         undeclared name instead of continuing the
//!                         declaration (§12.7.1)
//!   block-local event     `event e;` in a begin block was dropped, so
//!                         `->e` errored (§6.17)

use xezim::simulate;

const SRC: &str = r#"
module tb;
  int cast_down, cast_up, cast_var;
  int for_total;
  int ev_got;
  logic [7:0] v = 8'hAB;
  initial begin
    cast_down = 4'(8'hAB);
    cast_up = 16'(v);
    cast_var = 4'(v);
    for_total = 0;
    for (int j = 0, k = 10; j < k; j += 3) for_total++;
    begin
      event e;
      fork
        begin @(e); ev_got = 1; end
        begin #1; ->e; end
      join
    end
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn literal_size_casts_resize() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "cast_down"), 0xB, "4'(8'hAB) truncates");
    assert_eq!(u(&sim, "cast_up"), 0xAB, "16'(v) zero-extends");
    assert_eq!(u(&sim, "cast_var"), 0xB, "4'(v) truncates a variable too");
}

#[test]
fn for_init_comma_continues_the_declaration() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "for_total"), 4);
}

#[test]
fn block_local_event_triggers() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ev_got"), 1);
}
