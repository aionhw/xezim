//! CI smoke for the host-CPU benchmark workloads (`src/benchw.rs`,
//! `xezim-bench` binary): each workload runs a SMALL cycle count and its
//! self-check (the Rust mirror of the design arithmetic) must pass. No
//! wall-clock assertions — timing lives in the bench binary, correctness
//! lives here, so a semantics regression can't hide behind a fast number.

#[test]
fn bench_workloads_self_check() {
    for w in xezim::benchw::workloads() {
        let src = (w.source)(500);
        let sim = xezim::simulate(&src, (w.sim_time)(500))
            .unwrap_or_else(|e| panic!("{}: compile/run failed: {}", w.name, e));
        (w.check)(&sim);
    }
}
