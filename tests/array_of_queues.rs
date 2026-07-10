//! IEEE 1800-2017 §7.4.5: an unpacked array OF QUEUES — `int q[3][$]`,
//! `int q[2][3][$]`. The trailing `[$]` makes every element a queue of its own.
//!
//! The `[$]` was dropped at elaboration, so `q[i]` was a plain int:
//! `push_back` did nothing, every `size()` was 0 and `%p` printed `x`.
//!
//! Two neighbouring gaps surfaced with it:
//!   - `foreach (m[i, j])` bound only `i`; `j` stayed X and the loop ran over
//!     the first dimension alone. That applies to any multi-dimensional array,
//!     not just arrays of queues (§12.7.3).
//!   - `%p` on a queue printed all 64 slots of its backing buffer instead of
//!     exactly `size()` elements.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  int q1 [3][$];
  int q2 [2][3][$];
  int plain [$];

  int s0, s1, s2;
  int elem21;
  int popped, after_pop;
  int q2_elem, q2_size;
  int iter_count, last_i, last_j;

  initial begin
    plain.push_back(5); plain.push_back(6);

    q1[0].push_back(10); q1[0].push_back(11);
    q1[1].push_back(20);
    q1[2].push_back(30); q1[2].push_back(31); q1[2].push_back(32);

    s0 = q1[0].size();
    s1 = q1[1].size();
    s2 = q1[2].size();
    elem21 = q1[2][1];

    foreach (q2[i, j]) begin
      q2[i][j].push_back((i == j) ? 99 : (i + j + 1));
      q2[i][j].push_back(100 + (i * 10) + j);
      iter_count++;
      last_i = i;
      last_j = j;
    end

    q2_elem = q2[1][2][1];
    q2_size = q2[1][2].size();

    // Print before popping, so %p sees both elements.
    $display("PLAIN=%p", plain);
    $display("Q10=%p", q1[0]);
    $display("Q12=%p", q1[2]);
    $display("Q2_00=%p", q2[0][0]);

    popped  = q2[0][0].pop_front();
    after_pop = q2[0][0].size();
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

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn each_element_of_a_1d_array_of_queues_is_its_own_queue() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "s0"), 2);
    assert_eq!(u(&sim, "s1"), 1);
    assert_eq!(u(&sim, "s2"), 3);
    // `q[i][k]` selects element k of queue q[i], not a bit of a scalar.
    assert_eq!(u(&sim, "elem21"), 31);
}

#[test]
fn two_dimensional_array_of_queues_and_pop() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "q2_elem"), 112, "q2[1][2][1] = 100 + 1*10 + 2");
    assert_eq!(u(&sim, "q2_size"), 2);
    assert_eq!(u(&sim, "popped"), 99, "q2[0][0] head is the i==j value");
    assert_eq!(u(&sim, "after_pop"), 1);
}

#[test]
fn foreach_binds_every_index_variable() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // 2 x 3 = 6 iterations, last tuple (1, 2) — not 2 iterations with j == X.
    assert_eq!(u(&sim, "iter_count"), 6);
    assert_eq!(u(&sim, "last_i"), 1);
    assert_eq!(u(&sim, "last_j"), 2);
}

#[test]
fn p_format_prints_exactly_size_elements_of_a_queue() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // A plain queue must not print its 64-slot backing buffer.
    assert_eq!(line(&sim, "PLAIN="), "PLAIN='{5, 6}");
    // Elements of an array of queues have no declared type of their own.
    assert_eq!(line(&sim, "Q10="), "Q10='{10, 11}");
    assert_eq!(line(&sim, "Q12="), "Q12='{30, 31, 32}");
    assert_eq!(line(&sim, "Q2_00="), "Q2_00='{99, 100}");
}
