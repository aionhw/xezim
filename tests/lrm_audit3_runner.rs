//! LRM audit round 3 — chapters/areas the first two rounds didn't reach:
//! §18 solver depth (dist support/weights, solve-before + implications,
//! `inside {array}`), §20 remaining system functions (math library, the
//! §20.15.2 `$dist_*` family, conversions), and §23 hierarchy/generate.
//! Same ratchet contract as the other rounds: exact known-gap counts.
//! All currently at 0.

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
fn lrm3_ch18_constraints_deep() {
    run_chapter("ch18", include_str!("lrm_audit3/ch18_deep.sv"), 0);
}

#[test]
fn lrm3_ch20_system_functions() {
    run_chapter("ch20", include_str!("lrm_audit3/ch20_misc.sv"), 0);
}

#[test]
fn lrm3_ch23_hierarchy_generate() {
    run_chapter("ch23", include_str!("lrm_audit3/ch23_24_hier.sv"), 0);
}
