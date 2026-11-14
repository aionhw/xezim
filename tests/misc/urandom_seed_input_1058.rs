//! §18.13.1: the seed of `$urandom(seed)` is an input argument — the call
//! reads it and never writes the generated value back into it, whatever the
//! argument's storage (local, formal, property, array, queue or associative
//! element, struct member, hierarchical name, `ref` formal) and whatever the
//! calling context. The seed initializes the generator: the calls without a
//! seed that follow continue the sequence that seed determines, and only in
//! the calling thread (§18.14.2) (#1058). Expected values are the reference
//! simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// §18.13.1, §18.14.2: the seed stays, the same seed replays the same
/// following stream, and another thread's stream is untouched.
#[test]
fn urandom_seed_is_read_and_reseeds_the_calling_thread() {
    assert_eq!(
        lines(
            r#"module u1;
  int unsigned s, a1, a2, b1, b2, c1;
  int unsigned x[2], y[2], z[2];
  int unsigned q[$];
  initial begin
    s = 77;
    void'($urandom(s)); a1 = $urandom; a2 = $urandom_range(1000);
    void'($urandom(s)); b1 = $urandom; b2 = $urandom_range(1000);
    $display("A %0d %0d %0d", s, a1 == b1, a2 == b2);
    $display("B %0d %0d", $urandom(s) == $urandom(s), s);
    void'($urandom(78)); c1 = $urandom;
    $display("C %0d", c1 != a1);
    q.push_back(5); void'($urandom(q[0]));
    $display("D %0d", q[0]);
    fork
      begin process::self().srandom(9); x[0] = $urandom; #1; x[1] = $urandom; end
      begin process::self().srandom(9); y[0] = $urandom; #1; void'($urandom(1234)); y[1] = $urandom; end
      begin process::self().srandom(9); z[0] = $urandom; #2; z[1] = $urandom; end
    join
    $display("E %0d %0d %0d %0d", x[0] == y[0], x[0] == z[0], x[1] == z[1], y[1] != x[1]);
  end
endmodule"#
        ),
        ["A 77 1 1", "B 1 77", "C 1", "D 5", "E 1 1 1 1"]
    );
}

/// The reporter's 72 checks (#1058), run with `+SEED=12345` as reported.
/// E11 compares the sign of one random value, which §18.13.1 leaves to the
/// implementation; every other check must match.
#[test]
fn urandom_seed_argument_reporter_checks() {
    let src = include_str!("urandom_seed_arg_1058.sv").to_string();
    let plusargs = vec!["SEED=12345".to_string()];
    let sim = xezim::simulate_multi(
        &[src],
        100,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &plusargs,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect();
    let checks: Vec<&String> = out
        .iter()
        .filter(|l| l.contains(" exp=") && l.contains(" got="))
        .collect();
    assert_eq!(checks.len(), 72, "{out:#?}");
    let bad: Vec<&&String> = checks
        .iter()
        .filter(|l| !l.ends_with("MATCH") || l.contains("MISMATCH"))
        .filter(|l| !l.starts_with("E11 "))
        .collect();
    assert!(bad.is_empty(), "mismatches: {bad:#?}");
    assert!(
        out.iter()
            .any(|l| l == "R00 info: seed as read from the command line = 12345"),
        "{out:#?}"
    );
}
