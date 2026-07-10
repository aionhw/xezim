//! IEEE 1800-2017 §7.12.2: `q.sort()/.rsort()/.unique() with (item.field)`.
//!
//! The old implementation collected each element's packed VALUE, reordered the
//! values, and wrote them back. An unpacked-struct element has no packed value
//! — its members live in separate signals — so the queue was reordered by
//! nothing and `item.field` read X. Now the key is computed per element, the
//! queue is permuted by INDEX, and `item` binds to the element's flat NAME for
//! struct elements (to its value, as before, for scalars and class handles).

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct { int k; }                      inner_t;
  typedef struct { int a; string s; inner_t in; } p_t;

  p_t q [$], u [$], uniq [$];
  int iq [$];
  int uniq_size, src_size;

  class C;
    int v;
    function new(int x); v = x; endfunction
  endclass
  C cq [$];
  int c0, c1;

  initial begin
    q.push_back('{3, "three", '{30}});
    q.push_back('{1, "one",   '{10}});
    q.push_back('{2, "two",   '{20}});

    q.sort() with (item.a);
    $display("SORT=%p", q);
    q.rsort() with (item.a);
    $display("RSORT=%p", q);
    // A nested member as the key: `item.in.k` folds a MemberAccess chain.
    q.sort() with (item.in.k);
    $display("NESTED=%p", q);

    // §7.12.1: unique() RETURNS a queue and leaves the source alone.
    u.push_back('{5, "a", '{0}});
    u.push_back('{5, "b", '{0}});
    u.push_back('{6, "c", '{0}});
    uniq = u.unique() with (item.a);
    uniq_size = uniq.size();
    src_size  = u.size();
    $display("UNIQ=%p", uniq);

    // Scalar elements: `item` still binds by value.
    iq.push_back(3); iq.push_back(1); iq.push_back(2);
    iq.sort() with (item);
    $display("INT=%p", iq);

    // Class-handle elements: `item.prop` goes through the heap, as before.
    cq.push_back(new(3));
    cq.push_back(new(1));
    cq.sort() with (item.v);
    c0 = cq[0].v;
    c1 = cq[1].v;
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
fn sort_and_rsort_with_a_struct_member_key() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        line(&sim, "SORT="),
        concat!(
            "SORT='{'{a:1, s:\"one\", in:'{k:10}}, ",
            "'{a:2, s:\"two\", in:'{k:20}}, ",
            "'{a:3, s:\"three\", in:'{k:30}}}"
        )
    );
    assert_eq!(
        line(&sim, "RSORT="),
        concat!(
            "RSORT='{'{a:3, s:\"three\", in:'{k:30}}, ",
            "'{a:2, s:\"two\", in:'{k:20}}, ",
            "'{a:1, s:\"one\", in:'{k:10}}}"
        )
    );
}

#[test]
fn a_nested_member_can_be_the_sort_key() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        line(&sim, "NESTED="),
        concat!(
            "NESTED='{'{a:1, s:\"one\", in:'{k:10}}, ",
            "'{a:2, s:\"two\", in:'{k:20}}, ",
            "'{a:3, s:\"three\", in:'{k:30}}}"
        )
    );
}

/// §7.12.1: a locator method returns a queue; it must NOT modify the source.
#[test]
fn unique_returns_a_deduped_queue_and_leaves_the_source_alone() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "uniq_size"), 2);
    assert_eq!(u(&sim, "src_size"), 3, "unique() must not shrink its source");
    // First occurrence of each key survives.
    assert_eq!(
        line(&sim, "UNIQ="),
        r#"UNIQ='{'{a:5, s:"a", in:'{k:0}}, '{a:6, s:"c", in:'{k:0}}}"#
    );
}

#[test]
fn scalar_and_class_handle_elements_still_work() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(line(&sim, "INT="), "INT='{1, 2, 3}");
    assert_eq!(u(&sim, "c0"), 1);
    assert_eq!(u(&sim, "c1"), 3);
}
