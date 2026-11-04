//! UVM feature coverage against the real Accellera library, on both UVM 1.2
//! and IEEE 1800.2-2020 (the `nitronis/UVM` checkout `uvm_integration_tests`
//! locates). Each bench in `tests/uvm/` logs machine-checkable `T|...` lines.
//!
//! The areas here were not pinned elsewhere in the suite: phase ordering
//! across a hierarchy, objection drain time, report verbosity / ID actions /
//! severity overrides, `+UVM_TESTNAME` and `+UVM_VERBOSITY`, virtual
//! sequences, request/response routing, sequencer `lock()`, `uvm_callbacks`
//! and the global timeout.
//!
//! UVM 1800.2-2020 runs with its DPI LIVE. Its component-name check
//! (`uvm_component_name_check_visitor`) hands `uvm_is_match` a POSIX regex,
//! and the `UVM_NO_DPI` fallback of `uvm_re_match` is glob-only ("does not
//! match regular expressions"), so without DPI the library itself warns
//! `UVM/COMP/NAME` on every component, `uvm_test_top` included. That is UVM's
//! behaviour, not the simulator's; with xezim's native DPI the regex is
//! honoured and no component is flagged. UVM 1.2 runs `UVM_NO_DPI`, as most
//! of the suite does: its version of the check notices the missing DPI and
//! skips itself, logging an INFO under `UVM/COMP/NAMECHECK` instead.

use xezim::*;

const VERSIONS: [&str; 2] = ["1.2", "1800.2-2020"];

fn run(version: &str, src: &str, plusargs: &[&str]) -> compiler::Simulator {
    let src_dir = crate::uvm_integration_tests::uvm_dir()
        .join(version)
        .join("src");
    let uvm_pkg = std::fs::read_to_string(src_dir.join("uvm_pkg.sv"))
        .unwrap_or_else(|e| panic!("read {}/src/uvm_pkg.sv: {}", version, e));
    let mut defines = vec![("UVM_REPORT_DISABLE_FILE_LINE".to_string(), None)];
    if version == "1.2" {
        defines.push(("UVM_NO_DPI".to_string(), None));
    }
    let plusargs: Vec<String> = plusargs.iter().map(|s| s.to_string()).collect();
    simulate_multi(
        &[uvm_pkg, src.to_string()],
        10_000,
        Some("top"),
        &[src_dir.to_str().unwrap().to_string()],
        &[],
        None,
        false,
        None,
        None,
        &defines,
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
    .unwrap_or_else(|e| panic!("UVM {} bench failed to simulate: {}", version, e))
}

fn lines(sim: &compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .flat_map(|o| o.message.lines().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// The bench's own tagged lines, in order.
fn tagged(out: &[String]) -> Vec<&str> {
    out.iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// The end-of-run summary must report no errors and no fatals, and no
/// component may have been flagged by the 2020 name check.
fn assert_clean(version: &str, out: &[String]) {
    for want in ["UVM_ERROR :    0", "UVM_FATAL :    0"] {
        assert!(
            out.iter().any(|l| l == want),
            "UVM {version}: expected `{want}` in the report summary:\n{}",
            out.join("\n")
        );
    }
    assert!(
        // The bracketed tag: UVM 1.2 run without DPI logs an unrelated INFO
        // under `[UVM/COMP/NAMECHECK]` saying the check needs DPI.
        !out.iter().any(|l| l.contains("[UVM/COMP/NAME]")),
        "UVM {version}: a legal component name was flagged:\n{}",
        out.join("\n")
    );
}

/// build and final are top-down, every other function phase bottom-up, and
/// siblings run in NAME order (`mon` is created before `drv`).
pub(super) fn uvm_phases_run_in_hierarchy_order() {
    let src = include_str!("../uvm/uvm_phase_order.sv");
    let td = ["uvm_test_top", "e", "agt", "drv", "mon"];
    let bu = ["drv", "mon", "agt", "e", "uvm_test_top"];
    let mut want: Vec<String> = Vec::new();
    for n in td {
        want.push(format!("T|build|{n}"));
    }
    for ph in ["connect", "eoe", "sos"] {
        for n in bu {
            want.push(format!("T|{ph}|{n}"));
        }
    }
    want.push("T|run|uvm_test_top".to_string());
    for ph in ["extract", "check", "report"] {
        for n in bu {
            want.push(format!("T|{ph}|{n}"));
        }
    }
    for n in td {
        want.push(format!("T|final|{n}"));
    }
    for v in VERSIONS {
        let sim = run(v, src, &[]);
        let out = lines(&sim);
        assert_eq!(tagged(&out), want, "UVM {v}: phase order");
        assert_clean(v, &out);
        assert_eq!(sim.time, 10, "UVM {v}: run_phase holds for #10");
    }
}

/// run_phase ends at the LAST drop (t=70) plus the drain time (25).
pub(super) fn uvm_objection_drain_time_extends_the_phase() {
    let src = include_str!("../uvm/uvm_objection_drain.sv");
    for v in VERSIONS {
        let sim = run(v, src, &[]);
        let out = lines(&sim);
        assert_eq!(
            tagged(&out),
            vec!["T|drop|a|30", "T|drop|b|70", "T|extract_at|95"],
            "UVM {v}: objections and drain"
        );
        assert_clean(v, &out);
        assert_eq!(sim.time, 95, "UVM {v}: 70 + drain time 25");
    }
}

/// Verbosity filtering, an ID set to UVM_NO_ACTION, and a severity override.
pub(super) fn uvm_report_verbosity_actions_and_overrides() {
    let src = include_str!("../uvm/uvm_report_controls.sv");
    for v in VERSIONS {
        let sim = run(v, src, &[]);
        let out = lines(&sim);
        let infos: Vec<&str> = out
            .iter()
            .map(String::as_str)
            .filter(|l| l.starts_with("UVM_INFO") && l.contains("[V]"))
            .collect();
        assert_eq!(
            infos,
            vec![
                "UVM_INFO @ 0: uvm_test_top [V] at LOW",
                "UVM_INFO @ 0: uvm_test_top [V] at MEDIUM",
                "UVM_INFO @ 0: uvm_test_top [V] HIGH after raise",
            ],
            "UVM {v}: HIGH and DEBUG are filtered until the level is raised"
        );
        assert!(
            !out.iter().any(|l| l.contains("NOISY")),
            "UVM {v}: an ID set to UVM_NO_ACTION must not display"
        );
        assert!(
            out.iter()
                .any(|l| l == "UVM_WARNING @ 0: uvm_test_top [SOFT] demoted to a warning"),
            "UVM {v}: the overridden ERROR must print as a WARNING"
        );
        // NOISY is uncounted; SOFT counts as a warning, not an error.
        assert_eq!(
            tagged(&out),
            vec!["T|err=0 warn=1"],
            "UVM {v}: severity counts"
        );
    }
}

/// `run_test()` with no argument takes +UVM_TESTNAME; +UVM_VERBOSITY sets
/// the starting level — and without it a HIGH message stays filtered.
pub(super) fn uvm_cmdline_testname_and_verbosity() {
    let src = include_str!("../uvm/uvm_cmdline_select.sv");
    let vb = |out: &[String]| {
        out.iter()
            .any(|l| l == "UVM_INFO @ 0: uvm_test_top [VB] HIGH message")
    };
    for v in VERSIONS {
        let out = lines(&run(
            v,
            src,
            &["+UVM_TESTNAME=test_b", "+UVM_VERBOSITY=UVM_HIGH"],
        ));
        assert_eq!(
            tagged(&out),
            vec!["T|ran|test_b"],
            "UVM {v}: +UVM_TESTNAME=test_b"
        );
        assert!(
            vb(&out),
            "UVM {v}: +UVM_VERBOSITY=UVM_HIGH must let HIGH through"
        );
        assert_clean(v, &out);

        let out = lines(&run(v, src, &["+UVM_TESTNAME=test_b"]));
        assert!(
            !vb(&out),
            "UVM {v}: without +UVM_VERBOSITY, HIGH stays filtered"
        );

        let out = lines(&run(v, src, &["+UVM_TESTNAME=test_a"]));
        assert_eq!(
            tagged(&out),
            vec!["T|ran|test_a"],
            "UVM {v}: +UVM_TESTNAME=test_a"
        );
    }
}

/// A virtual sequence drives two sequencers in parallel; each driver gets its
/// own share, in order, and the virtual sequence completes after both.
pub(super) fn uvm_virtual_sequence_drives_two_agents() {
    let src = include_str!("../uvm/uvm_virtual_sequence.sv");
    for v in VERSIONS {
        let sim = run(v, src, &[]);
        let out = lines(&sim);
        let t = tagged(&out);
        let of =
            |p: &str| -> Vec<&str> { t.iter().copied().filter(|l| l.starts_with(p)).collect() };
        assert_eq!(
            of("T|da|"),
            vec!["T|da|100", "T|da|101", "T|da|102"],
            "UVM {v}"
        );
        assert_eq!(
            of("T|db|"),
            vec!["T|db|200", "T|db|201", "T|db|202", "T|db|203"],
            "UVM {v}"
        );
        assert_eq!(
            t.last(),
            Some(&"T|vseq_done|4"),
            "UVM {v}: fork/join waits for both"
        );
        assert_clean(v, &out);
    }
}

/// get_response() receives the response the driver tagged with
/// set_id_info(req) — the one for the request just sent.
pub(super) fn uvm_sequence_receives_matching_responses() {
    let src = include_str!("../uvm/uvm_seq_response.sv");
    for v in VERSIONS {
        let out = lines(&run(v, src, &[]));
        assert_eq!(
            tagged(&out),
            vec![
                "T|req=1 rsp=10 match=1",
                "T|req=2 rsp=20 match=1",
                "T|req=3 rsp=30 match=1",
                "T|req=4 rsp=40 match=1",
            ],
            "UVM {v}: request/response routing"
        );
        assert_clean(v, &out);
    }
}

/// A sequence holding lock() gets its items through back to back.
///
/// Only that contiguity is guaranteed. Where the unlocked sequence's items
/// fall around the locked block depends on the library's grant logic, and it
/// does differ: UVM 1.2 delivers `A0 A1 B0 B1 B2 A2`, 1800.2-2020
/// `A0 B0 B1 B2 A1 A2`. So the exact interleave is deliberately not pinned.
pub(super) fn uvm_locked_sequence_items_are_contiguous() {
    let src = include_str!("../uvm/uvm_seq_lock.sv");
    for v in VERSIONS {
        let out = lines(&run(v, src, &[]));
        let t = tagged(&out);
        let mut sorted = t.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            vec!["T|A0", "T|A1", "T|A2", "T|B0", "T|B1", "T|B2"],
            "UVM {v}: every item reaches the driver exactly once"
        );
        let b0 = t.iter().position(|l| *l == "T|B0").expect("B0 delivered");
        assert_eq!(
            &t[b0..b0 + 3],
            &["T|B0", "T|B1", "T|B2"],
            "UVM {v}: the locked sequence must not be interleaved: {t:?}"
        );
        let a: Vec<&str> = t.iter().copied().filter(|l| l.starts_with("T|A")).collect();
        assert_eq!(
            a,
            vec!["T|A0", "T|A1", "T|A2"],
            "UVM {v}: A keeps its own order"
        );
        assert_clean(v, &out);
    }
}

/// A registered callback runs through `uvm_do_callbacks; deleted, it no
/// longer does.
pub(super) fn uvm_callbacks_add_and_delete() {
    let src = include_str!("../uvm/uvm_callbacks.sv");
    for v in VERSIONS {
        let out = lines(&run(v, src, &[]));
        assert_eq!(
            tagged(&out),
            vec!["T|none=5", "T|with=1005", "T|deleted=5"],
            "UVM {v}: callback lifecycle"
        );
        assert_clean(v, &out);
    }
}

/// A run_phase that never drops its objection is ended by set_timeout with a
/// PH_TIMEOUT fatal at exactly the configured time.
pub(super) fn uvm_global_timeout_ends_a_hung_run_phase() {
    let src = include_str!("../uvm/uvm_global_timeout.sv");
    for v in VERSIONS {
        let sim = run(v, src, &[]);
        let out = lines(&sim);
        assert!(
            out.iter()
                .any(|l| l.starts_with("UVM_FATAL @ 300: reporter [PH_TIMEOUT]")),
            "UVM {v}: expected a PH_TIMEOUT fatal at t=300:\n{}",
            out.join("\n")
        );
        assert!(
            out.iter().any(|l| l == "UVM_FATAL :    1"),
            "UVM {v}: one fatal"
        );
        assert_eq!(
            sim.time, 300,
            "UVM {v}: the timeout, not max time, ends the run"
        );
    }
}
