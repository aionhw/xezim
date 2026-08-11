//! §13.5.2 `ref` formals ALIAS the actual — reference-validated (H3 audit).
//!
//! The legacy model was copy-in at call, copy-out at return. Three visible
//! divergences: (1) a parallel process observing the actual mid-call never
//! saw the callee's writes; (2) the callee never saw the observer's writes;
//! (3) the return copy-out CLOBBERED whatever the observer wrote during the
//! call. Aliasing is applied where the actual is a plain module-visible
//! variable; caller-frame locals and aggregate elements keep the legacy
//! copy path (still correct for them at return, though not mid-call).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able (x/z?)", n))
}

/// Reference: at5=10 (write visible mid-call), mid=20 (observer's write
/// visible after resume), g=21 (no copy-out clobber of the observer's 20),
/// ali=5 / g=5 (two ref formals of the same variable truly alias).
#[test]
fn ref_writes_and_reads_alias_the_actual() {
    let src = r#"
module top;
  int g;
  int seen_at5 = -1, mid = -1, ali = -1;

  task automatic bump(ref int r);
    r = 10;
    #10;
    mid = r;
    r = r + 1;
  endtask

  task automatic alias2(ref int a, ref int b);
    a = 5;
    ali = b;
  endtask

  initial begin
    g = 1;
    fork
      bump(g);
      begin #5 seen_at5 = g; g = 20; end
    join
    alias2(g, g);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "seen_at5"), 10, "callee write visible to a parallel observer mid-call");
    assert_eq!(u(&sim, "mid"), 20, "observer write visible to the callee after resume");
    assert_eq!(u(&sim, "g"), 5, "no copy-out clobber; alias2 leaves g=5");
    assert_eq!(u(&sim, "ali"), 5, "double-ref of one variable: b reads a's write");
}

/// Reference: foo=8 (formal named like the actual still aliases), loc=8
/// (a caller-LOCAL actual keeps the legacy copy path and still updates),
/// g2=99 / chained=99 (a ref passed on as ref reaches the original storage).
#[test]
fn ref_alias_edge_shapes() {
    let src = r#"
module top;
  int foo;
  int g2;
  int chained_saw = -1;
  int loc_out = -1;

  task automatic t_same(ref int foo);
    foo = 7;
    #1 foo = foo + 1;
  endtask

  task automatic inner(ref int r);
    r = 99;
    #1 chained_saw = r;
  endtask
  task automatic outer(ref int r);
    inner(r);
  endtask

  task automatic t_local();
    int loc;
    loc = 3;
    t_same(loc);
    loc_out = loc;
  endtask

  initial begin
    foo = 1;
    t_same(foo);
    g2 = 0;
    fork
      outer(g2);
      begin #1 ; end
    join
    t_local();
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "foo"), 8, "same-named formal/actual aliases the storage");
    assert_eq!(u(&sim, "g2"), 99, "chained ref writes the original variable");
    assert_eq!(u(&sim, "chained_saw"), 99);
    assert_eq!(u(&sim, "loc_out"), 8, "caller-local actual: legacy copy path still lands");
}
