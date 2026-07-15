//! Ratchet for the `always_comb` / `always_latch` / `always_ff` cluster of the
//! Icarus `ivtest` regression suite (IEEE 1800-2017 §9.2.2).
//!
//! Two properties are pinned here:
//!
//!  * SENSITIVITY (§9.2.2.2/§9.2.2.3): an inferred-sensitivity process reacts to
//!    EVERY variable read in its evaluation — including a variable read only
//!    inside a function it calls (`hidden` below) — and runs once at time 0.
//!    An empty-sensitivity `always_comb` fires exactly once at time 0 AFTER the
//!    initial procedures have started, so a same-time pre-delay read still sees
//!    the pre-trigger `x`.
//!
//!  * ILLEGAL FORMS (§9.2.2.1/§9.2.2.2/§9.2.2.3/§9.2.2.4): a blocking delay,
//!    an event control, or a `wait` inside `always_comb`/`always_latch`/
//!    `always_ff`; an `always_ff` with no leading event control; an
//!    `always_latch` with an empty inferred sensitivity list; and a plain
//!    `always fork … join_any/join_none` that cannot advance time — all must be
//!    rejected at elaboration.
//!
//! Sources are copied from `ivtest/ivltests/*.v` so this test is self-contained.

use xezim::simulate;

/// A `normal` self-checking design: must run, print PASSED, and never FAILED.
fn assert_passes(name: &str, src: &str) {
    let sim = simulate(src, 100_000)
        .unwrap_or_else(|e| panic!("{name}: expected a clean run, got error: {e}"));
    let out: String = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(out.contains("PASSED"), "{name}: no PASSED in output:\n{out}");
    assert!(!out.contains("FAILED"), "{name}: printed FAILED:\n{out}");
}

/// A `ce` (compile-error) design: elaboration must reject it.
fn assert_rejected(name: &str, src: &str) {
    let r = simulate(src, 100_000);
    assert!(
        r.is_err(),
        "{name}: expected elaboration to REJECT this illegal form, but it compiled"
    );
}

// ---------------------------------------------------------------------------
// A. Sensitivity — must run and print PASSED
// ---------------------------------------------------------------------------

#[test]
fn always_comb_hidden_function_read_and_time0() {
    // §9.2.2.2: `always_comb` is sensitive to `hidden`, read only inside f_and.
    const SRC: &str = r#"
module top;
  reg y, a, b, flip, hidden;
  reg pass;

  function f_and (input i1, i2);
    reg  partial;
    begin
      partial = i1 & i2;
      f_and = partial | hidden;
    end
  endfunction

  reg intr;
  always_comb begin
    intr = flip;
    y = f_and(a, b) ^ intr;
  end

  initial begin
    pass = 1'b1;
    flip = 1'b0; hidden = 1'b0; a = 1'b0; b = 1'b0; #1;
    if (y !== 1'b0) begin $display("FAILED a=0 b=0 h=0"); pass = 1'b0; end
    a = 1'b0; b = 1'b1; #1;
    if (y !== 1'b0) begin $display("FAILED a=0 b=1 h=0"); pass = 1'b0; end
    a = 1'b1; b = 1'b0; #1;
    if (y !== 1'b0) begin $display("FAILED a=1 b=0 h=0"); pass = 1'b0; end
    a = 1'b1; b = 1'b1; #1;
    if (y !== 1'b1) begin $display("FAILED a=1 b=1 h=0"); pass = 1'b0; end
    hidden = 1'b0; a = 1'b0; b = 1'b0; #1;
    if (y !== 1'b0) begin $display("FAILED a=0 b=0 h=0 (2)"); pass = 1'b0; end
    hidden = 1'b1; a = 1'b0; b = 1'b0; #1;
    if (y !== 1'b1) begin $display("FAILED a=0 b=0 h=1"); pass = 1'b0; end
    if (pass) $display("PASSED");
  end
endmodule
"#;
    assert_passes("always_comb", SRC);
}

#[test]
fn always_comb_no_sens_infers_and_fires_at_time0() {
    // §9.2.2.2: no explicit list; y is x before the first delay, 0 after.
    const SRC: &str = r#"
module test;
   reg passed;
   logic y;
   always_comb begin
      y = 1'b0;
   end
  initial begin
    passed = 1'b1;
    if (y !== 1'bx) begin $display("FAILED: expected 1'bx, got %b", y); passed = 1'b0; end
    #1;
    if (y !== 1'b0) begin $display("FAILED: expected 1'b0, got %b", y); passed = 1'b0; end
    if (passed) $display("PASSED");
  end
endmodule
"#;
    assert_passes("always_comb_no_sens", SRC);
}

#[test]
fn always_latch_hidden_function_read_and_hold() {
    // §9.2.2.3: inferred sensitivity (incl. `hidden` via f_and) and latch hold.
    const SRC: &str = r#"
module top;
  reg y, a, b, flip, hidden, en;
  reg pass;

  function f_and (input i1, i2);
    reg  partial;
    begin
      partial = i1 & i2;
      f_and = partial | hidden;
    end
  endfunction

  reg intr;
  always_latch begin
    if (en) begin
      intr = flip;
      y <= f_and(a, b) ^ intr;
    end
  end

  initial begin
    pass = 1'b1;
    en = 1'b1; flip = 1'b0; hidden = 1'b0; a = 1'b0; b = 1'b0; #1;
    if (y !== 1'b0) begin $display("FAILED 1"); pass = 1'b0; end
    a = 1'b0; b = 1'b1; #1;
    if (y !== 1'b0) begin $display("FAILED 2"); pass = 1'b0; end
    a = 1'b1; b = 1'b0; #1;
    if (y !== 1'b0) begin $display("FAILED 3"); pass = 1'b0; end
    a = 1'b1; b = 1'b1; #1;
    if (y !== 1'b1) begin $display("FAILED 4"); pass = 1'b0; end
    hidden = 1'b0; a = 1'b0; b = 1'b0; #1;
    if (y !== 1'b0) begin $display("FAILED 5"); pass = 1'b0; end
    hidden = 1'b1; a = 1'b0; b = 1'b0; #1;
    if (y !== 1'b1) begin $display("FAILED 6"); pass = 1'b0; end
    en = 1'b0; hidden = 1'b0; #1;
    if (y !== 1'b1) begin $display("FAILED hold"); pass = 1'b0; end
    if (pass) $display("PASSED");
  end
endmodule
"#;
    assert_passes("always_latch", SRC);
}

// ---------------------------------------------------------------------------
// B. Illegal forms — must be rejected at elaboration
// ---------------------------------------------------------------------------

#[test]
fn reject_always4a_fork_join_any_zero_delay() {
    // §9.2.2.1: join_any unblocks on the shortest child (#0) → zero-delay loop.
    const SRC: &str = r#"
module top;
  always fork
    #0;
    #1;
  join_any
  initial begin $display("FAILED"); #1; $finish; end
endmodule
"#;
    assert_rejected("always4A", SRC);
}

#[test]
fn reject_always4b_fork_join_none_zero_delay() {
    // §9.2.2.1: join_none never blocks the parent → zero-delay loop.
    const SRC: &str = r#"
module top;
  always fork
    #2;
    #1;
  join_none
  initial begin $display("FAILED"); #1; $finish; end
endmodule
"#;
    assert_rejected("always4B", SRC);
}

#[test]
fn reject_always_comb_blocking_delay() {
    // §9.2.2.2: a blocking delay (#0) is not allowed in always_comb.
    const SRC: &str = r#"
module top;
  reg q, d;
  always_comb begin
    #0 q = d;
  end
  initial $display("Expected compile failure!");
endmodule
"#;
    assert_rejected("always_comb_fail", SRC);
}

#[test]
fn reject_always_comb_event_control() {
    // §9.2.2.2: an event control (@foo) is not allowed in always_comb.
    const SRC: &str = r#"
module top;
  reg q, d;
  event foo;
  always_comb begin
    @foo q = d;
  end
  initial $display("Expected compile failure!");
endmodule
"#;
    assert_rejected("always_comb_fail3", SRC);
}

#[test]
fn reject_always_comb_event_and_wait() {
    // §9.2.2.2: event controls and a wait statement are not allowed.
    const SRC: &str = r#"
module top;
  reg a, b;
  reg q, d;
  event foo;
  always_comb begin
    q = d;
    fork $display("fork/join 1"); join
    fork $display("fork/join_any 1"); join_any
    fork $display("fork/join_none 1"); join_none
    a <= @foo 1'b1;
    @(b) a <= repeat(2) @foo 1'b0;
    wait (!a) $display("wait");
  end
  initial #1 $display("Expect compile errors!");
endmodule
"#;
    assert_rejected("always_comb_fail4", SRC);
}

#[test]
fn reject_always_ff_no_event_control() {
    // §9.2.2.4: always_ff's first statement must be an event control.
    const SRC: &str = r#"
module test;
   logic y;
   always_ff begin
      y = 1'b0;
   end
  initial $display("FAILED");
endmodule
"#;
    assert_rejected("always_ff_no_sens", SRC);
}

#[test]
fn reject_always_latch_blocking_delay() {
    // §9.2.2.3: a blocking delay is not allowed in always_latch.
    const SRC: &str = r#"
module top;
  reg q, en, d;
  always_latch begin
    if (en) #0 q <= d;
  end
  initial $display("Expected compile failure!");
endmodule
"#;
    assert_rejected("always_latch_fail", SRC);
}

#[test]
fn reject_always_latch_event_control() {
    // §9.2.2.3: an event control is not allowed in always_latch.
    const SRC: &str = r#"
module top;
  reg q, en, d;
  event foo;
  always_latch begin
    if (en) @foo q <= d;
  end
  initial $display("Expected compile failure!");
endmodule
"#;
    assert_rejected("always_latch_fail3", SRC);
}

#[test]
fn reject_always_latch_event_and_wait() {
    // §9.2.2.3: event controls and a wait statement are not allowed.
    const SRC: &str = r#"
module top;
  reg a, b;
  reg q, d;
  event foo;
  always_latch begin
    q <= d;
    fork $display("fork/join 1"); join
    fork $display("fork/join_any 1"); join_any
    fork $display("fork/join_none 1"); join_none
    a <= @foo 1'b1;
    @(b) a <= repeat(2) @foo 1'b0;
    wait (!a) $display("wait");
  end
  initial #1 $display("Expect compile errors!");
endmodule
"#;
    assert_rejected("always_latch_fail4", SRC);
}

#[test]
fn reject_always_latch_no_event_control() {
    // §9.2.2.3: an always_latch with an empty inferred sensitivity is illegal.
    const SRC: &str = r#"
module test;
   logic y;
   always_latch begin
      y = 1'b0;
   end
  initial $display("FAILED");
endmodule
"#;
    assert_rejected("always_latch_no_sens", SRC);
}
