//! §15.4.1 — a BOUNDED mailbox (`new(N)`, N>0) enforces its capacity: `try_put`
//! returns 0 when full, `num()` never exceeds the bound, and a blocking `put`
//! on a full box suspends until a `get`/`try_get` frees a slot. An unbounded
//! `new()` keeps the old behavior (never fails, never blocks). Previously the
//! bound was discarded entirely (every mailbox was unbounded).

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect::<Vec<_>>().join("\n")
}

/// try_put rejects when full; num() tracks the bound.
#[test]
fn bounded_try_put_rejects_when_full() {
    let o = out(r#"
module t;
  mailbox mb = new(2);
  initial begin
    $display("A=%0d", mb.try_put(10));  // 1
    $display("B=%0d", mb.try_put(20));  // 1
    $display("C=%0d", mb.try_put(30));  // 0 (full)
    $display("N=%0d", mb.num());        // 2
  end
endmodule
"#);
    assert!(o.contains("A=1") && o.contains("B=1"), "first two try_puts succeed; got: {}", o);
    assert!(o.contains("C=0"), "try_put on a full bounded mailbox must return 0; got: {}", o);
    assert!(o.contains("N=2"), "num() must not exceed the bound; got: {}", o);
}

/// A blocking put on a full box suspends until a get frees a slot (top level).
#[test]
fn bounded_put_blocks_until_get() {
    let o = out(r#"
module t;
  mailbox mb = new(2);
  int x;
  initial begin
    mb.put(1); mb.put(2);     // full
    mb.put(3);                // blocks until the get at #5
    $display("UNBLOCKED=%0t", $time);
    $display("FINAL_NUM=%0d", mb.num());
  end
  initial begin #5 mb.get(x); end
endmodule
"#);
    assert!(o.contains("UNBLOCKED=5"), "put must block until a slot frees at t=5; got: {}", o);
    assert!(o.contains("FINAL_NUM=2"), "box stays at its bound after the blocked put lands; got: {}", o);
}

/// The same, but the blocking put is nested inside a fork branch (must still
/// route through the suspend-aware runner).
#[test]
fn bounded_put_blocks_inside_fork() {
    let o = out(r#"
module t;
  mailbox mb = new(2);
  int x;
  initial begin
    mb.put(1); mb.put(2);
    fork
      begin mb.put(3); $display("FUNBLOCKED=%0t", $time); end
      begin #5 mb.get(x); end
    join
  end
endmodule
"#);
    assert!(o.contains("FUNBLOCKED=5"), "put in a fork branch must block until t=5; got: {}", o);
}

/// An unbounded mailbox never fails try_put and never blocks put.
#[test]
fn unbounded_mailbox_unaffected() {
    let o = out(r#"
module t;
  mailbox mb = new();
  initial begin
    $display("P=%0d", mb.try_put(1));
    $display("Q=%0d", mb.try_put(2));
    $display("R=%0d", mb.try_put(3));  // still 1 — unbounded
    $display("M=%0d", mb.num());       // 3
  end
endmodule
"#);
    assert!(o.contains("R=1"), "unbounded try_put never fails; got: {}", o);
    assert!(o.contains("M=3"), "unbounded mailbox grows freely; got: {}", o);
}
