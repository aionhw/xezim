//! Timing/event semantics locked to a commercial-simulator oracle. Each case
//! is the distilled form of an oracle-diffed audit test; the expected strings
//! are the oracle's output. Covers: §4.5 region order (#0 before NBA), §9.4.5
//! intra-assignment delay + event controls, §10.3.3 inertial cont-assign and
//! gate delays, §15.5.2 `->>` delivery, §21.2.3 $monitor first-print timing,
//! §20.7 array queries, §16.9.3 sampled-value functions with explicit
//! clocking, delayed-NBA signedness, and %06d decimal padding.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn expect(src: &str, wanted: &[&str]) {
    let out = lines(src);
    for w in wanted {
        assert!(
            out.iter().any(|l| l == w),
            "missing {:?} in output {:?}",
            w,
            out
        );
    }
}

#[test]
fn zero_delay_resumes_before_nba_region() {
    // §4.5: the inactive region (#0) activates BEFORE the NBA region.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg x;
  initial begin
    x = 0;
    #5;
    x <= 1;
    #0 $display("C x=%b", x);
    $display("D x=%b", x);
    #1 $display("E x=%b", x);
    $finish;
  end
endmodule
"#,
        &["C x=0", "D x=0", "E x=1"],
    );
}

#[test]
fn blocking_intra_assignment_delay_blocks_scaled() {
    // §9.4.5 + §3.14.3: `b = #3 a` captures now, blocks 3 TIMEUNITS.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg a, b;
  initial begin
    a = 1; b = 0;
    #10 b = #3 a;
    $display("T%0t b=%b", $time, b);
    $finish;
  end
endmodule
"#,
        &["T13000 b=1"],
    );
}

#[test]
fn intra_assignment_repeat_event_control() {
    // §9.4.5: RHS captured at statement time, assigned after the 3rd posedge.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg clk, v, w;
  initial clk = 0;
  always #2 clk = ~clk;
  initial begin
    v = 0; w = 0;
    #1 v = 1;
    w = repeat (3) @(posedge clk) v;
    $display("T%0t w=%b", $time, w);
    $finish;
  end
endmodule
"#,
        &["T10000 w=1"],
    );
}

#[test]
fn cont_assign_delay_scaled_and_inertial() {
    // §10.3.3: `assign #5` counts timeunits; a 2ns pulse is FILTERED.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg a; wire y; integer rises;
  assign #5 y = a;
  initial rises = 0;
  always @(posedge y) rises = rises + 1;
  initial begin
    a = 0;
    #10 a = 1;
    #2  a = 0;   // pulse 2 < 5: cancelled
    #20 a = 1;   // y rises once, at t=37
    #10 $display("rises=%0d", rises);
    $finish;
  end
endmodule
"#,
        &["rises=1"],
    );
}

#[test]
fn gate_delay_scaled_and_inertial() {
    // §28.9: `buf #(4)` delays 4 timeunits and filters a 1ns pulse.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg a; wire y; integer rises;
  buf #(4) g(y, a);
  initial rises = 0;
  always @(posedge y) rises = rises + 1;
  initial begin
    a = 0;
    #10 a = 1;
    #1  a = 0;   // 1ns pulse through a 4ns gate: filtered
    #20 a = 1;   // y rises once, at 35
    #10 $display("T%0t rises=%0d y=%b", $time, rises, y);
    $finish;
  end
endmodule
"#,
        &["T41000 rises=1 y=1"],
    );
}

#[test]
fn nonblocking_trigger_reaches_parked_waiter() {
    // §15.5.2: a bare `->>` (no NBA data queued) still flushes with the NBA
    // region and wakes an already-parked @(ev) waiter.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  event ev;
  initial begin
    #5;
    ->> ev;
    #10 $finish;
  end
  initial begin @(ev) $display("T%0t got ev", $time); end
endmodule
"#,
        &["T5000 got ev"],
    );
}

#[test]
fn monitor_first_print_at_end_of_timestep() {
    // §21.2.3: the first $monitor print reflects values written LATER in the
    // same time step by other processes.
    let out = lines(
        r#"
`timescale 1ns/1ps
module top;
  reg [3:0] v;
  initial $monitor("v=%0d", v);
  initial begin
    v = 0;
    #5 v = 1; v = 2; v = 3;
    #5 $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        out.first().map(|s| s.as_str()),
        Some("v=0"),
        "first monitor line must be the settled t0 value: {:?}",
        out
    );
    assert!(out.iter().any(|l| l == "v=3"), "{:?}", out);
    assert!(!out.iter().any(|l| l == "v=x"), "premature X print: {:?}", out);
}

#[test]
fn delayed_nba_same_time_last_wins_unsigned_display() {
    // Two delayed NBAs to the same target+time: last wins; the committed
    // value takes the TARGET's signedness (4-bit 9 is 9, not -7).
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg [3:0] r;
  initial begin
    r = 0;
    #5;
    r <= #3 7;
    r <= #3 9;
    #5 $display("r=%0d", r);
    $finish;
  end
endmodule
"#,
        &["r=9"],
    );
}

#[test]
fn array_query_declared_bounds() {
    // §20.7: declared bounds/order for non-normalized packed and unpacked
    // ranges; $dimensions counts packed dims; $increment is signed.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg [15:8] p; reg [0:7] asc;
  reg [3:0][7:0] p2;
  reg [1:0] m2 [3:1][0:4];
  initial begin
    $display("p %0d %0d %0d %0d", $left(p), $right(p), $low(p), $high(p));
    $display("asc %0d %0d %0d", $left(asc), $right(asc), $increment(asc));
    $display("p2 %0d %0d", $dimensions(p2), $left(p2));
    $display("m2 %0d %0d %0d %0d", $left(m2,1), $right(m2,1), $left(m2,2), $right(m2,2));
    $finish;
  end
endmodule
"#,
        &[
            "p 15 8 8 15",
            "asc 0 7 -1",
            "p2 2 3",
            "m2 3 1 0 4",
        ],
    );
}

#[test]
fn sampled_value_functions_explicit_clocking() {
    // §16.9.3: explicit clocking argument + default clocking, preponed
    // sampling (oracle-matched).
    expect(
        r#"
`timescale 1ns/1ps
module top;
  reg clk, d;
  initial clk = 0;
  always #5 clk = ~clk;
  default clocking cb @(posedge clk); endclocking
  initial begin
    d = 0;
    @(posedge clk); d = 1;
    @(posedge clk);
    $display("A past=%b rose=%b stable=%b", $past(d), $rose(d, @(posedge clk)), $stable(d, @(posedge clk)));
    @(posedge clk); d = 0;
    @(posedge clk);
    $display("B past=%b fell=%b stable=%b", $past(d), $fell(d, @(posedge clk)), $stable(d, @(posedge clk)));
    $finish;
  end
endmodule
"#,
        &["A past=0 rose=1 stable=0", "B past=1 fell=1 stable=0"],
    );
}

#[test]
fn decimal_width_zero_flag_space_pads() {
    // Reference-simulator behavior: `%06d` treats the leading 0 as width,
    // decimals always space-pad.
    expect(
        r#"
`timescale 1ns/1ps
module top;
  initial begin
    $display("[%6d][%-6d][%06d]", 42, 42, 42);
    $finish;
  end
endmodule
"#,
        &["[    42][42    ][    42]"],
    );
}

#[test]
fn observer_always_block_does_not_fire_at_time_zero() {
    // §9.2.2.1 + §6.8: a write-free `always @(sig)` observer suspends at its
    // event control and runs only on a real sensitivity change — declaration
    // init / the X→z settling of an undriven net is NOT an event. So the
    // display fires once (when y transitions z→0 at t=5000ns via the delayed
    // cont-assign), never at t=0. Oracle-verified.
    let out = lines(
        r#"
`timescale 1ns/1ps
module top;
  reg a; wire y;
  assign #5 y = a;
  always @(y) $display("FIRE T%0t y=%b", $realtime, y);
  initial begin a = 0; #40 $finish; end
endmodule
"#,
    );
    let fires: Vec<&String> = out.iter().filter(|l| l.starts_with("FIRE ")).collect();
    assert_eq!(
        fires.len(),
        1,
        "observer must fire exactly once (no t0 pseudo-edge): {:?}",
        fires
    );
    assert_eq!(fires[0], "FIRE T5000 y=0", "the single fire is the z->0 change");
}
