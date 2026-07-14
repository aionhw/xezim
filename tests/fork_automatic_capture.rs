//! IEEE 1800-2017 §9.3.2 + §6.21 — automatic loop-var capture in
//! `fork … join_none`.
//!
//! Each loop iteration's `automatic` local is a distinct variable, and a
//! process forked in that iteration captures ITS iteration's value. These
//! captures used to COLLAPSE: with no call frame the local lived in the
//! shared signal map, was overwritten by later iterations before the
//! children ran at their scheduled time, and every child observed the last
//! iteration's value (the classic UVM spawn-per-agent idiom broke).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// The canonical intermediate-capture idiom: `automatic int li = i;` inside
/// the loop body, forked child reads `li` after a delay.
const CAPTURE_VIA_LOCAL: &str = r#"
module tb;
  int count, mask, fire_time;
  initial begin
    for (int i = 0; i < 3; i++) begin
      automatic int li = i;
      fork
        begin
          #1;
          count = count + 1;
          mask = mask | (1 << li);
          fire_time = $time;
        end
      join_none
    end
    #2;
  end
endmodule
"#;

/// The direct idiom — the declaration lives at the head of the fork block
/// itself (§9.3.2: block declarations execute in the forking process, once
/// per fork, before the children are spawned).
const CAPTURE_VIA_FORK_DECL: &str = r#"
module tb;
  int count, mask;
  initial begin
    for (int i = 0; i < 3; i++)
      fork
        automatic int k = i;
        begin
          #1;
          count = count + 1;
          mask = mask | (1 << k);
        end
      join_none
    #2;
  end
endmodule
"#;

/// foreach index variables are automatic (§12.7.3); each iteration's fork
/// child must see its own index (and hence its own element).
const CAPTURE_FOREACH: &str = r#"
module tb;
  int arr[3];
  int count, sum;
  initial begin
    arr[0] = 10; arr[1] = 20; arr[2] = 30;
    foreach (arr[j]) begin
      fork
        begin
          #1;
          count = count + 1;
          sum = sum + arr[j];
        end
      join_none
    end
    #2;
  end
endmodule
"#;

#[test]
fn fork_join_none_captures_each_iterations_automatic_local() {
    let sim = simulate(CAPTURE_VIA_LOCAL, 100).expect("simulate failed");
    assert_eq!(u(&sim, "count"), 3, "not every forked child ran");
    // mask == 3'b111 means the children saw the DISTINCT values 0, 1, 2 —
    // a collapsed capture yields 3'b100 (all saw 2).
    assert_eq!(u(&sim, "mask"), 0b111, "children collapsed onto one capture");
    // Children still run at their scheduled time, not at spawn time.
    assert_eq!(u(&sim, "fire_time"), 1, "child did not run at its #1 delay");
}

#[test]
fn fork_block_head_declaration_captures_per_spawn() {
    let sim = simulate(CAPTURE_VIA_FORK_DECL, 100).expect("simulate failed");
    assert_eq!(u(&sim, "count"), 3, "not every forked child ran");
    assert_eq!(u(&sim, "mask"), 0b111, "children collapsed onto one capture");
}

#[test]
fn fork_join_none_captures_foreach_index() {
    let sim = simulate(CAPTURE_FOREACH, 100).expect("simulate failed");
    assert_eq!(u(&sim, "count"), 3, "not every forked child ran");
    assert_eq!(u(&sim, "sum"), 60, "children collapsed onto one foreach index");
}
