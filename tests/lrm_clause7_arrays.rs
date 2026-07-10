//! IEEE 1800-2017 Clause 7 conformance findings.
//!
//! §7.12.1 — the array LOCATOR methods (`find*`, `min`, `max`, `unique`,
//! `unique_index`) return a queue and must not modify the source array.
//!   - `q.min()` parses as a Call, a shape the old dispatch never matched, so
//!     the result was a packed scalar written to a queue container signal that
//!     nothing reads: the destination came back as X.
//!   - `q.unique()` REORDERED AND SHRANK the source instead of returning a
//!     queue, silently corrupting it (a following `q.sum()` then read short).
//!   - With a `with` clause, keys were read from packed element values, so a
//!     queue of unpacked structs selected nothing.
//!
//! §7.5.1 — `d = new[n](src)` copies min(n, src.size()) elements and
//! default-initialises the rest. It parses as a NESTED call,
//! `Call{Call{Ident(new),[n]},[src]}`, which was never recognised: the size
//! came out as 1 and the data was lost.

use xezim::simulate;

const LOCATORS: &str = r#"
module tb;
  int q [$] = '{3, 1, 2, 1};
  int mn [$], mx [$], uq [$], ui [$], fd [$];
  int sum_after;
  int mn0, mx0, q_size_after;
  initial begin
    mn = q.min();
    mx = q.max();
    uq = q.unique();
    ui = q.unique_index();
    fd = q.find with (item > 1);

    // The source must survive every locator call untouched.
    q_size_after = q.size();
    sum_after    = q.sum();

    mn0 = mn[0];
    mx0 = mx[0];
    $display("UQ=%p", uq);
    $display("UI=%p", ui);
    $display("FD=%p", fd);
  end
endmodule
"#;

/// Locators over a queue of unpacked structs, keyed by a member.
const STRUCT_LOCATORS: &str = r#"
module tb;
  typedef struct { int a; string s; } p_t;
  p_t q [$], mn [$], fd [$], uq [$];
  int src_size;
  initial begin
    q.push_back('{3, "c"});
    q.push_back('{1, "a"});
    q.push_back('{2, "b"});
    mn = q.min()    with (item.a);
    fd = q.find     with (item.a > 1);
    uq = q.unique() with (item.a);
    src_size = q.size();
    $display("MN=%p", mn[0]);
    $display("FD=%p", fd);
    $display("UQSZ=%0d", uq.size());
  end
endmodule
"#;

const NEW_COPY: &str = r#"
module tb;
  int f [3] = '{1, 2, 3};
  int d [], e [];
  int d_size, d2, e_size, e1;
  int from_fixed [];
  int ff_size, ff1;
  initial begin
    d = new[4];
    d[2] = 9;
    d = new[6](d);          // grow, preserving contents
    d_size = d.size();
    d2 = d[2];

    e = d;                  // §7.6 whole dynamic-array copy
    e_size = e.size();
    e1 = e[2];

    from_fixed = new[3](f); // source may be a fixed array
    ff_size = from_fixed.size();
    ff1 = from_fixed[1];
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
fn locator_methods_return_a_queue() {
    let sim = simulate(LOCATORS, 100).expect("simulate failed");
    assert_eq!(u(&sim, "mn0"), 1, "q.min() returned X");
    assert_eq!(u(&sim, "mx0"), 3, "q.max() returned X");
    assert_eq!(line(&sim, "UQ="), "UQ='{3, 1, 2}");
    assert_eq!(line(&sim, "UI="), "UI='{0, 1, 2}");
    assert_eq!(line(&sim, "FD="), "FD='{3, 2}");
}

#[test]
fn locator_methods_do_not_modify_the_source() {
    let sim = simulate(LOCATORS, 100).expect("simulate failed");
    assert_eq!(u(&sim, "q_size_after"), 4, "a locator shrank the source queue");
    assert_eq!(u(&sim, "sum_after"), 7, "a locator corrupted the source queue");
}

#[test]
fn locators_work_on_queues_of_unpacked_structs() {
    let sim = simulate(STRUCT_LOCATORS, 100).expect("simulate failed");
    assert_eq!(line(&sim, "MN="), r#"MN='{a:1, s:"a"}"#);
    assert_eq!(line(&sim, "FD="), r#"FD='{'{a:3, s:"c"}, '{a:2, s:"b"}}"#);
    assert_eq!(line(&sim, "UQSZ="), "UQSZ=3");
    assert_eq!(u(&sim, "src_size"), 3);
}

#[test]
fn sized_new_with_a_source_copies_and_resizes() {
    let sim = simulate(NEW_COPY, 100).expect("simulate failed");
    assert_eq!(u(&sim, "d_size"), 6, "new[6](d) did not resize");
    assert_eq!(u(&sim, "d2"), 9, "new[6](d) lost the source contents");
    // Whole dynamic-array assignment.
    assert_eq!(u(&sim, "e_size"), 6);
    assert_eq!(u(&sim, "e1"), 9);
    // The source may be a fixed array.
    assert_eq!(u(&sim, "ff_size"), 3);
    assert_eq!(u(&sim, "ff1"), 2);
}
