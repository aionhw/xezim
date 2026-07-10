//! The last of the queue-op fallout from unpacked-struct storage, plus a `dist`
//! width bug found alongside it.
//!
//! An unpacked-struct element has no container signal — its members live in
//! separate signals. Every queue operation that moved elements by reading one
//! packed value and writing it back therefore moved nothing: `insert`,
//! `delete(i)`, `reverse` and `shuffle`.
//!
//! `r = q` between two queues was never implemented at all, for ANY element
//! type: the RHS was read as a scalar container signal that a queue does not
//! have, so the destination stayed empty.
//!
//! And `dist` built its buckets at a hardcoded 32 bits, so a 64-bit range
//! produced only truncated values.

use xezim::simulate;

const QUEUES: &str = r#"
module tb;
  typedef struct { int a; string s; } p_t;
  p_t pq [$];
  p_t sq [$];
  int iq [$], ir [$];
  int isize;

  initial begin
    // insert into the middle of a queue of structs
    pq.push_back('{1, "one"});
    pq.push_back('{3, "three"});
    pq.insert(1, '{2, "two"});
    $display("I0=%p", pq[0]);
    $display("I1=%p", pq[1]);
    $display("I2=%p", pq[2]);

    // delete(idx) must shift the tail down
    pq.delete(0);
    $display("D0=%p", pq[0]);
    $display("D1=%p", pq[1]);

    // reverse must move members
    sq.push_back('{1, "a"});
    sq.push_back('{2, "b"});
    sq.reverse();
    $display("R0=%p", sq[0]);
    $display("R1=%p", sq[1]);

    // whole-queue assignment, scalar elements
    iq.push_back(3); iq.push_back(1);
    ir = iq;
    isize = ir.size();
    $display("IR=%p", ir);
  end
endmodule
"#;

/// Whole-queue assignment must deep-copy struct elements too.
const QCOPY: &str = r#"
module tb;
  typedef struct { int a; string s; } p_t;
  p_t q [$], r [$];
  int rsize;
  initial begin
    q.push_back('{7, "seven"});
    q.push_back('{8, "eight"});
    r = q;
    rsize = r.size();
    $display("C0=%p", r[0]);
    $display("C1=%p", r[1]);
  end
endmodule
"#;

/// A `dist` range wider than 32 bits must not truncate its buckets.
const DIST64: &str = r#"
module tb;
  bit [63:0] wide;
  int out_of_range;
  int in_range;
  initial begin
    for (int i = 0; i < 300; i++) begin
      void'(std::randomize(wide) with { wide dist { [64'h1_0000_0000 : 64'h1_0000_00FF] :/ 1 }; });
      if (wide < 64'h1_0000_0000 || wide > 64'h1_0000_00FF) out_of_range++;
      else in_range++;
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

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn insert_shifts_struct_elements_member_wise() {
    let sim = simulate(QUEUES, 100).expect("simulate failed");
    assert_eq!(line(&sim, "I0="), r#"I0='{a:1, s:"one"}"#);
    assert_eq!(line(&sim, "I1="), r#"I1='{a:2, s:"two"}"#);
    assert_eq!(line(&sim, "I2="), r#"I2='{a:3, s:"three"}"#);
}

#[test]
fn delete_at_index_shifts_the_tail_down() {
    let sim = simulate(QUEUES, 100).expect("simulate failed");
    assert_eq!(line(&sim, "D0="), r#"D0='{a:2, s:"two"}"#);
    assert_eq!(line(&sim, "D1="), r#"D1='{a:3, s:"three"}"#);
}

#[test]
fn reverse_moves_struct_members() {
    let sim = simulate(QUEUES, 100).expect("simulate failed");
    assert_eq!(line(&sim, "R0="), r#"R0='{a:2, s:"b"}"#);
    assert_eq!(line(&sim, "R1="), r#"R1='{a:1, s:"a"}"#);
}

#[test]
fn whole_queue_assignment_copies_scalar_elements() {
    let sim = simulate(QUEUES, 100).expect("simulate failed");
    assert_eq!(u(&sim, "isize"), 2, "r = q left the queue empty");
    assert_eq!(line(&sim, "IR="), "IR='{3, 1}");
}

#[test]
fn whole_queue_assignment_deep_copies_struct_elements() {
    let sim = simulate(QCOPY, 100).expect("simulate failed");
    assert_eq!(u(&sim, "rsize"), 2);
    assert_eq!(line(&sim, "C0="), r#"C0='{a:7, s:"seven"}"#);
    assert_eq!(line(&sim, "C1="), r#"C1='{a:8, s:"eight"}"#);
}

#[test]
fn dist_buckets_are_as_wide_as_their_bounds() {
    let sim = simulate(DIST64, 100).expect("simulate failed");
    assert_eq!(u(&sim, "out_of_range"), 0, "dist truncated a 64-bit range to 32 bits");
    assert_eq!(u(&sim, "in_range"), 300);
}
