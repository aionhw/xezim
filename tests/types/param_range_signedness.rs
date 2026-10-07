//! Parameter/localparam value semantics for an EXPLICIT type or range
//! (IEEE 1800 §6.20.2 / §10.7): the initializer — and every override — is
//! an ASSIGNMENT to the declared type: evaluated in the declared width
//! context, wrapped to that width, with the DECLARED signedness.
//!
//! Before the fix, the elaborated value kept the RHS's own width and
//! signedness:
//! * A bare range `[3:0]` is UNSIGNED, but `localparam [1:0] C = 1 + P`
//!   read the 2-bit pattern as SIGNED (-2) and sign-extended at every
//!   use — `assign y = C` drove 126, generate-if/ternary conditions
//!   mis-evaluated, `C + C` gave -4, and `logic [C-1:0]` sized 4 wide.
//!   Parameter defaults/overrides were not wrapped either: `parameter
//!   [3:0] P = -1` kept -1, `[7:0] P = 256` kept 256.
//! * `signed [3:0]` wraps: `P = 15` must be -1 (default and override).
//! * A typedef'd SIGNED type lost its sign: `typedef logic signed [1:0]
//!   T; localparam T C = …` read +2 instead of -2 — the local typedef
//!   was invisible to the width/signedness lookups.
//! * An UNTYPED parameter keeps the RHS value and sign (§6.20.2) — pinned
//!   by [control] checks, along with signed wrap, typedef'd unsigned,
//!   `int`, real, shift/replication amounts, and the width-context
//!   behavior of `localparam [7:0] C = P + P` (30, not the 8-bit wrap).
//!
//! The .sv is self-checking (SVTEST macros): it prints `TEST_PASS`, or
//! `FAIL @…` lines and `TEST_FAIL count=N` on any regression. Covers
//! header defaults, named `.P(-1)` / positional `#(-1)` overrides,
//! localparams in the parameter port list, body localparams of
//! instantiated modules, and hierarchical reads `u.P` / `u.C`.

use xezim::simulate;

const PARAM_RANGE_SIGNEDNESS: &str = include_str!("../lrm_9_value_param/param_range_signedness.sv");

#[test]
fn param_range_signedness_value_semantics() {
    let sim = simulate(PARAM_RANGE_SIGNEDNESS, 1000).expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("TEST_PASS")),
        "expected TEST_PASS in output\nfull output: {msgs:?}"
    );
    assert!(
        !msgs.iter().any(|m| m.contains("FAIL")),
        "unexpected FAIL in output\nfull output: {msgs:?}"
    );
}