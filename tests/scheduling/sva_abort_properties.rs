//! IEEE 1800-2023 §16.12.14: abort properties. `accept_on (b) p` holds and
//! `reject_on (b) p` fails when `b` holds during the evaluation of `p`;
//! otherwise each has the result of `p`. The plain forms check the sampled
//! `b` in every time step of the evaluation, the `sync_` forms only at the
//! ticks of the property's clock. An abort at a tick takes precedence over
//! what `p` does at that tick. An accepted attempt is a vacuous success
//! (no pass action). An asynchronous abort between two ticks is reported
//! at the next tick.
use xezim::simulate;

/// Clock ticks at 5, 15, 25 and 35; `start` holds only at the first tick;
/// `$finish` at 40.
fn check(property: &str, setup: &str, expected: &[&str]) {
    let source = format!(
        r#"module top;
 logic clk=0,start=1,b=0,r=0;
 always #5 clk=~clk;
 initial begin $assertvacuousoff; #10 start=0; end
 initial begin {setup} end
 assert property(@(posedge clk) {property})
 $display("T|P %0t",$time); else $display("T|F %0t",$time);
 initial #40 $finish;
 endmodule"#
    );
    let result = simulate(&source, 100).unwrap();
    let lines: Vec<_> = result
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(lines, expected, "`{}` with `{}`", property, setup);
}

/// `r` is 1 from 12 to 13: between two ticks.
const PULSE_BETWEEN_TICKS: &str = "#12 r=1; #1 r=0;";

#[test]
fn accept_on_between_ticks_accepts_a_failing_attempt() {
    check("start |-> ##2 b", ";", &["T|F 25"]);
    check("accept_on (r) start |-> ##2 b", PULSE_BETWEEN_TICKS, &[]);
}

#[test]
fn reject_on_between_ticks_rejects_at_the_next_tick() {
    check("reject_on (r) start |-> ##2 1", ";", &["T|P 25"]);
    check(
        "reject_on (r) start |-> ##2 1",
        PULSE_BETWEEN_TICKS,
        &["T|F 15"],
    );
}

/// The `sync_` forms sample the condition only at the ticks.
#[test]
fn sync_forms_miss_a_pulse_between_ticks() {
    check(
        "sync_accept_on (r) start |-> ##2 b",
        PULSE_BETWEEN_TICKS,
        &["T|F 25"],
    );
    check(
        "sync_reject_on (r) start |-> ##2 1",
        PULSE_BETWEEN_TICKS,
        &["T|P 25"],
    );
}

/// `r` samples 1 at the tick where `p` fails: the abort wins. The attempt
/// that starts at that tick is aborted too (accepted silently, rejected
/// with a second failure).
#[test]
fn an_abort_at_a_tick_takes_precedence() {
    let r_at_15 = "#14 r=1; #2 r=0;";
    check("accept_on (r) start |-> ##1 b", r_at_15, &[]);
    check("sync_accept_on (r) start |-> ##1 b", r_at_15, &[]);
    check(
        "sync_reject_on (r) start |-> ##1 1",
        r_at_15,
        &["T|F 15", "T|F 15"],
    );
}

/// The condition is checked at the tick where the evaluation starts.
#[test]
fn an_abort_at_the_first_tick() {
    check(
        "reject_on (r) 1",
        "r=1; #10 r=0;",
        &["T|F 5", "T|P 15", "T|P 25", "T|P 35"],
    );
}

/// A consequent's evaluation starts at its first tick: a pulse before it
/// does not abort it, one during it does.
#[test]
fn a_nested_abort_covers_only_its_own_evaluation() {
    check("start |=> reject_on (r) ##1 1", PULSE_BETWEEN_TICKS, &["T|P 25"]);
    check("start |=> reject_on (r) ##1 1", "#18 r=1; #1 r=0;", &["T|F 25"]);
}

/// An abort inside one operand of `and` settles that operand only.
#[test]
fn an_abort_inside_and() {
    check(
        "(accept_on (r) start |-> ##2 b) and (start |-> ##1 1)",
        PULSE_BETWEEN_TICKS,
        &["T|P 15"],
    );
}

/// After the last tick: an accepted strong obligation does not fail at the
/// end of simulation, and a rejected weak one does.
#[test]
fn an_abort_after_the_last_tick() {
    check("accept_on (r) start |-> s_eventually b", ";", &["T|F 40"]);
    check(
        "accept_on (r) start |-> s_eventually b",
        "#36 r=1; #1 r=0;",
        &[],
    );
    check("reject_on (r) start |-> ##10 1", ";", &[]);
    check(
        "reject_on (r) start |-> ##10 1",
        "#36 r=1; #1 r=0;",
        &["T|F 40"],
    );
}
