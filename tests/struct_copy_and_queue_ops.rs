//! The remaining gaps from the array-of-collections audit.
//!
//! The headline one: IEEE 1800-2017 §7.2 — assigning one unpacked struct to
//! another copies every member. Their leaves live in separate signals, so
//! evaluating the RHS to a packed value and storing it wrote a container nobody
//! reads: `y = x` left every member of `y` at X. That single defect also broke
//! `push_front`, `pop_front`/`pop_back`, and struct copies through array or
//! queue elements.
//!
//! Also here:
//!   - `d[i] = new[n]` on an array of dynamic arrays never sized the element.
//!   - `int qq[$][$]` — a queue of queues.
//!   - `$size(m)` / `$size(m, 2)` on a multi-dimensional array returned 0.
//!   - `%p` on a PARTIAL index of a multi-dimensional array printed X.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct { int a; string s; } p_t;

  p_t x, y, arr[2], e, popped_f, popped_b;
  p_t pq [$];
  p_t sq [2][$];

  int qq [$][$];    // queue of queues
  int dq [3][];     // array of dynamic arrays
  int q3 [2][2][$]; // partial-index %p
  int m2 [2][3];

  int qq_size, qq_elem;
  int dq_size, dq_elem;
  int sz1, sz2;

  initial begin
    // §7.2 whole-struct assignment, in every storage combination.
    x = '{1, "one"};
    y = x;
    arr[0] = x;
    pq.push_back(x);
    e = pq[0];

    // push_front must shift members, not one packed word.
    pq.push_front('{2, "front"});
    popped_f = pq.pop_front();
    popped_b = pq.pop_back();

    sq[1].push_back('{9, "arr"});

    // Queue of queues.
    qq[0].push_back(10); qq[0].push_back(11);
    qq_size = qq[0].size();
    qq_elem = qq[0][1];

    // new[n] on an element of an array of dynamic arrays.
    dq[0] = new[3];
    dq[0][1] = 42;
    dq_size = dq[0].size();
    dq_elem = dq[0][1];

    // $size over both dimensions.
    sz1 = $size(m2);
    sz2 = $size(m2, 2);

    q3[1][0].push_back(9);

    $display("Y=%p", y);
    $display("A0=%p", arr[0]);
    $display("E=%p", e);
    $display("PF=%p", popped_f);
    $display("PB=%p", popped_b);
    $display("SQ=%p", sq[1][0]);
    $display("Q3=%p", q3[1]);
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
fn assigning_an_unpacked_struct_copies_every_member() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // variable <- variable, array element <- variable, variable <- queue element
    assert_eq!(line(&sim, "Y="), r#"Y='{a:1, s:"one"}"#);
    assert_eq!(line(&sim, "A0="), r#"A0='{a:1, s:"one"}"#);
    assert_eq!(line(&sim, "E="), r#"E='{a:1, s:"one"}"#);
}

#[test]
fn push_front_and_pop_carry_struct_members() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // push_front shifted a packed word, losing members; pop returned one.
    assert_eq!(line(&sim, "PF="), r#"PF='{a:2, s:"front"}"#);
    assert_eq!(line(&sim, "PB="), r#"PB='{a:1, s:"one"}"#);
    assert_eq!(line(&sim, "SQ="), r#"SQ='{a:9, s:"arr"}"#);
}

#[test]
fn queue_of_queues_and_sized_new_on_an_element() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "qq_size"), 2, "int qq[$][$] element is not a queue");
    assert_eq!(u(&sim, "qq_elem"), 11);
    assert_eq!(u(&sim, "dq_size"), 3, "d[i] = new[3] did not size the element");
    assert_eq!(u(&sim, "dq_elem"), 42);
}

#[test]
fn size_reports_each_dimension_of_a_multi_dim_array() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "sz1"), 2);
    assert_eq!(u(&sim, "sz2"), 3);
}

#[test]
fn p_format_renders_a_partial_index_of_a_multi_dim_array() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    // `q3[1]` still names a sub-array; its elements are queues.
    assert_eq!(line(&sim, "Q3="), "Q3='{'{9}, '{}}");
}
