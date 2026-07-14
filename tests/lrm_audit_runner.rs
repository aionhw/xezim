//! Per-chapter IEEE 1800-2017 compliance audit, run as a RATCHET: every
//! chapter testbench self-checks and prints one `FAIL[..]` line per violated
//! rule plus a `CHECKS DONE fails=N` summary. Each test asserts the EXACT
//! known-failure count for its chapter:
//!
//!   - a regression (new failure) trips the test with the offending rule name;
//!   - fixing a known gap ALSO trips it — lower the expected count so the
//!     ratchet never loosens.
//!
//! The known gaps are tracked as findings (see the FAIL names in each .sv):
//! block-local packed typedefs, named struct patterns, fork capture collapse,
//! intra-assignment delay, NBA-after-#0, super.method dispatch, class value
//! params, size-cast-down, 11.4.11 x-merge, nested unpacked concat.

use xezim::simulate;

fn run_chapter(name: &str, src: &str, expected_fails: usize) {
    let sim = simulate(src, 100_000)
        .unwrap_or_else(|e| panic!("{}: simulate failed: {}", name, e));
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let done = msgs.iter().any(|m| m.contains("CHECKS DONE"));
    assert!(
        done,
        "{}: never reached the summary line — simulation died early:\n{}",
        name,
        msgs.join("\n")
    );
    let fails: Vec<&String> = msgs.iter().filter(|m| m.starts_with("FAIL[")).collect();
    assert_eq!(
        fails.len(),
        expected_fails,
        "{}: expected exactly {} known LRM gaps, got {} — a new regression \
         (or a fixed finding: lower the count). Failing checks:\n{}",
        name,
        expected_fails,
        fails.len(),
        fails
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn lrm_ch6_data_types() {
    run_chapter("ch6", include_str!("lrm_audit/ch6_types.sv"), 0);
}

#[test]
fn lrm_ch7_aggregates() {
    run_chapter("ch7", include_str!("lrm_audit/ch7_aggregates.sv"), 0);
}

#[test]
fn lrm_ch7_packed_module_scope() {
    run_chapter("ch7b", include_str!("lrm_audit/ch7b_packed_scope.sv"), 0);
}

#[test]
fn lrm_ch7_packed_local_scope() {
    run_chapter("ch7c", include_str!("lrm_audit/ch7c_local_typedef.sv"), 0);
}

#[test]
fn lrm_ch8_classes() {
    run_chapter("ch8", include_str!("lrm_audit/ch8_classes.sv"), 0);
}

#[test]
fn lrm_ch9_processes() {
    run_chapter("ch9", include_str!("lrm_audit/ch9_processes.sv"), 0);
}

#[test]
fn lrm_ch10_assignments() {
    run_chapter("ch10", include_str!("lrm_audit/ch10_assignments.sv"), 0);
}

#[test]
fn lrm_ch11_operators() {
    run_chapter("ch11", include_str!("lrm_audit/ch11_operators.sv"), 0);
}

#[test]
fn lrm_ch12_procedural() {
    run_chapter("ch12", include_str!("lrm_audit/ch12_procedural.sv"), 0);
}

#[test]
fn lrm_ch13_subroutines() {
    run_chapter("ch13", include_str!("lrm_audit/ch13_subroutines.sv"), 0);
}

#[test]
fn lrm_ch15_18_20_ipc_rand_sysfns() {
    run_chapter("chX", include_str!("lrm_audit/ch15_18_20.sv"), 0);
}
