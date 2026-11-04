//! Post-closure fresh-audit finds — reference-validated.
//!
//! 1. §7.12: a DECLARED iterator (`q.sort(x) with (x)`) was silently
//!    ignored — the filter evaluated 0 for every element, so sort became a
//!    stable no-op and `with`-reductions summed zeros. The iterator name now
//!    binds alongside `item` in sort/rsort/unique and the reductions.
//! 2. §9.4.2/§7.2.1: `@(s.a)` on a PACKED STRUCT field armed a nonexistent
//!    signal (the field is a slice of the base vector) and woke spuriously
//!    at t=0. It now arms the BASE with the field expression as the
//!    value-compare term.

use xezim::simulate;

fn outs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

/// Reference: sorted '{1,2,4,5}; after delete(1): '{1,4,5} sum=10.
#[test]
fn sort_with_declared_iterator() {
    let src = r#"
module tb;
  int q[$] = '{5, 1, 4, 2};
  initial begin
    q.sort(x) with (x);
    $display("T|sorted=%p", q);
    q.delete(1);
    $display("T|afterdel=%p sum=%0d", q, q.sum());
    $display("T|wsum=%0d", q.sum(y) with (y * 2));
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let o = outs(&sim);
    assert!(o.contains(&"T|sorted='{1, 2, 4, 5}".to_string()), "{o:?}");
    assert!(
        o.contains(&"T|afterdel='{1, 4, 5} sum=10".to_string()),
        "{o:?}"
    );
    assert!(
        o.contains(&"T|wsum=20".to_string()),
        "named iterator in reductions: {o:?}"
    );
}

/// Reference: seen=4 — the wait parks until the FIELD changes.
#[test]
fn event_control_on_packed_struct_field() {
    let src = r#"
module tb;
  typedef struct packed { logic a; logic b; } sp_t;
  sp_t s = '0;
  int seen = -1;
  initial begin
    fork
      begin @(s.a); seen = $time; end
      begin #4 s.a = 1; end
    join
    $display("T|seen=%0d", seen);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert!(
        outs(&sim).contains(&"T|seen=4".to_string()),
        "{:?}",
        outs(&sim)
    );
}

/// §6.21 (audit #44, reference-validated): a local variable WITH an
/// initializer in a STATIC-lifetime task/function must be explicitly
/// `static` or `automatic` — the reference rejects the implicit form at
/// compile time, even under an explicit `task static` and in package
/// scope. For-header decls, initial-block locals, no-init locals, and
/// automatic contexts stay accepted.
#[test]
fn implicit_static_local_initializer_is_rejected() {
    let bad = r#"
module tb;
  task t1(); int c = 0; c++; endtask
  initial t1();
endmodule
"#;
    let err = match simulate(bad, 10) {
        Ok(_) => panic!("static-task local with init must be rejected"),
        Err(e) => e,
    };
    assert!(
        err.contains("implicitly static"),
        "diagnostic names the rule, got: {err}"
    );

    let bad_pkg = r#"
package pk;
  function int f(); int c = 0; return c; endfunction
endpackage
module tb;
  initial void'(pk::f());
endmodule
"#;
    assert!(
        simulate(bad_pkg, 10).is_err(),
        "package-scope static function local with init must be rejected"
    );

    let ok = r#"
module tb;
  task t1(); static int c = 0; c++; endtask
  function automatic int f2(); int c = 5; return c; endfunction
  int r = 0;
  initial begin
    int x = 1;                        // block local: fine
    for (int i = 0; i < 2; i++) x++;  // for-header decl: fine
    t1();
    r = f2() + x;
  end
endmodule
"#;
    let sim = simulate(ok, 10).expect("explicit lifetimes and block locals stay accepted");
    let r = sim.get_signal("r").and_then(|v| v.to_u64()).unwrap_or(0);
    assert_eq!(r, 8, "f2()=5 + x=3");
}

/// §21.2.1.7 (audit #44; #566 majority of the reference simulators): %p of an
/// ASSOCIATIVE array closes with a tight `}` like every other aggregate, and
/// an empty assoc prints `'{}`.
#[test]
fn assoc_array_percent_p_closes_tight() {
    let src = r#"
module tb;
  int aa[string];
  int bb[int];
  int ee[string];
  int q[$] = {1, 2};
  initial begin
    aa["k1"] = 5; aa["k2"] = 6;
    bb[3] = 7; bb[10] = 8;
    $display("T|%p", aa);
    $display("T|%p", bb);
    $display("T|%p", ee);
    $display("T|%p", q);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("sim");
    let out = outs(&sim).join("\n");
    for want in [
        "T|'{\"k1\":5, \"k2\":6}",
        "T|'{3:7, 10:8}",
        "T|'{}",
        "T|'{1, 2}",
    ] {
        assert!(out.contains(want), "missing `{}` in:\n{}", want, out);
    }
}
