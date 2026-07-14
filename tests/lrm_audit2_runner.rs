//! LRM audit round 2 (chapters uncovered by round 1) — same ratchet contract
//! as tests/lrm_audit_runner.rs: exact known-gap counts; a new failure or a
//! fixed gap both trip the test.
//!
//! All chapters currently at 0 known gaps.

use xezim::simulate;

fn run_chapter(name: &str, src: &str, expected_fails: usize) {
    let sim = simulate(src, 1_000_000)
        .unwrap_or_else(|e| panic!("{}: simulate failed: {}", name, e));
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("CHECKS DONE")),
        "{}: summary line missing:\n{}",
        name,
        msgs.join("\n")
    );
    let fails: Vec<&String> = msgs.iter().filter(|m| m.starts_with("FAIL[")).collect();
    assert_eq!(
        fails.len(),
        expected_fails,
        "{}: expected {} known gaps, got {}:\n{}",
        name,
        expected_fails,
        fails.len(),
        fails.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn lrm2_ch5_lexical() {
    run_chapter("ch5", include_str!("lrm_audit2/ch5_lexical.sv"), 0);
}

#[test]
fn lrm2_ch14_19_clocking_coverage() {
    run_chapter("ch14_19", include_str!("lrm_audit2/ch14_19_clk_cov.sv"), 0);
}

#[test]
fn lrm2_ch16_assertions() {
    run_chapter("ch16", include_str!("lrm_audit2/ch16_assertions.sv"), 0);
}

#[test]
fn lrm2_ch21_file_io() {
    run_chapter("ch21", include_str!("lrm_audit2/ch21_fileio.sv"), 0);
}

#[test]
fn lrm2_ch22_preprocessor() {
    run_chapter("ch22", include_str!("lrm_audit2/ch22_preproc.sv"), 0);
}

#[test]
fn lrm2_ch25_26_interfaces_packages() {
    run_chapter("ch25_26", include_str!("lrm_audit2/ch25_26_ifc_pkg.sv"), 0);
}

#[test]
fn lrm2_deep_queues_streaming_tasks() {
    run_chapter("deep", include_str!("lrm_audit2/ch7_11_13_deep.sv"), 0);
}
