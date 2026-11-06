//! §11.4.13 / §18.5.4 — a real-valued bound in an `inside` set keeps the
//! integer values it covers.
//!
//! `2000ns` folds to a real in the current time unit, and an integer operand
//! is promoted to real for the comparison, so the integer set is
//! `ceil(lo)..floor(hi)`. Treating the real end as an unresolvable bound made
//! `int wait_time; wait_time inside {[0:2000ns]}` an empty domain, so every
//! `randomize()` trial was rejected.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn time_literal_range_bound_is_satisfiable() {
    let src = r#"
module top;
  class c;
    rand int wait_time;
    constraint wait_c { wait_time inside {[0:2000ns]}; }
  endclass
  initial begin
    c h = new;
    int ok = 0;
    int in_range = 1;
    repeat (50) begin
      if (!h.randomize()) $display("T|randomize failed");
      else begin
        ok++;
        if (h.wait_time < 0 || h.wait_time > 2000) in_range = 0;
      end
    end
    $display("T|ok=%0d in_range=%0d", ok, in_range);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    assert!(
        got.iter().any(|m| m == "T|ok=50 in_range=1"),
        "expected 50 satisfiable draws inside 0..2000, got {got:?}"
    );
    assert!(
        !got.iter().any(|m| m.starts_with("T|randomize failed")),
        "randomize() rejected a time-literal bound: {got:?}"
    );
}

/// A real low end truncates up; the integer operand still has to clear it.
#[test]
fn fractional_real_range_bound_rounds_inward() {
    let src = r#"
module top;
  class c;
    rand int x;
    constraint x_c { x inside {[1.5:3.5]}; }
  endclass
  initial begin
    c h = new;
    int ok = 0;
    int seen_2 = 0;
    int seen_3 = 0;
    int bad = 0;
    repeat (40) begin
      if (h.randomize()) begin
        ok++;
        if (h.x == 2) seen_2 = 1;
        if (h.x == 3) seen_3 = 1;
        if (h.x < 2 || h.x > 3) bad = 1;
      end
    end
    $display("T|ok=%0d mid=%0d/%0d bad=%0d", ok, seen_2, seen_3, bad);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    assert!(
        got.iter().any(|m| m == "T|ok=40 mid=1/1 bad=0"),
        "expected integer draws 2..3 only, got {got:?}"
    );
}

/// `wait_time inside {2000ns}` (a single value, no range) must still be
/// evaluated as a set member rather than silently dropped.
#[test]
fn single_real_value_in_inside_set_is_satisfiable() {
    let src = r#"
module top;
  class c;
    rand int x;
    constraint x_c { x inside {100, 2.0}; }
  endclass
  initial begin
    c h = new;
    int ok = 0;
    int bad = 0;
    repeat (40) begin
      if (h.randomize()) begin
        ok++;
        if (h.x != 100 && h.x != 2) bad = 1;
      end
    end
    $display("T|ok=%0d bad=%0d", ok, bad);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    assert!(
        got.iter().any(|m| m == "T|ok=40 bad=0"),
        "expected draws of 100 or 2 only, got {got:?}"
    );
}
