//! UVM register abstraction layer (RAL) against the real Accellera library,
//! on UVM 1.2 and IEEE 1800.2-2020. The benches live in `tests/uvm/` and log
//! machine-checkable `T|...` lines.
//!
//! - `uvm_ral_access_policies.sv`: the mirror after a write, a second write
//!   and a read, for each of the 25 field access policies. The expected
//!   table was computed from `uvm_reg_field::XpredictX` and the read branch
//!   of `do_predict`, and is the same in both versions.
//! - `uvm_ral_map_rights.sv`: how map rights rewrite a field's effective
//!   access (`uvm_reg_field::get_access`). The rule for "WO" rights CHANGED
//!   between the versions, so each version has its own table.
//! - `uvm_ral_bus.sv`: a register block on a small bus DUT with a driver,
//!   monitor and adapter. It covers frontdoor access (W1C, RC and RO
//!   behaviour, desired-vs-mirror `update()`), the built-in
//!   `uvm_reg_hw_reset_seq`, a `mirror(UVM_CHECK)` mismatch, explicit
//!   prediction through `uvm_reg_predictor`, and `uvm_mem`.
//!
//! Like `uvm_feature_coverage`, 1800.2-2020 runs with its DPI live and 1.2
//! runs `UVM_NO_DPI` (see that file for why).

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

fn tagged(out: &[String]) -> Vec<&str> {
    out.iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// The end-of-run summary reports `errors` UVM_ERRORs and no fatals, and
/// the simulator itself reported nothing (a null dereference inside the
/// library is logged and execution continues, so it must be caught here).
fn assert_summary(version: &str, out: &[String], errors: u32) {
    let want = format!("UVM_ERROR :    {}", errors);
    assert!(
        out.iter().any(|l| l.trim_start() == want),
        "UVM {version}: expected {want:?} in:\n{out:#?}"
    );
    assert!(
        out.iter().any(|l| l.trim_start() == "UVM_FATAL :    0"),
        "UVM {version}: fatal reported:\n{out:#?}"
    );
    assert!(
        !out.iter()
            .any(|l| l.contains("[xezim][error]") || l.contains("[xezim][fatal]")),
        "UVM {version}: simulator error:\n{out:#?}"
    );
}

/// `T|<policy>|<after reset>|<after write 0F>|<after write F0>|<after read 3C>`
const ACCESS_POLICIES: &str = "\
T|RO|a5|a5|a5|3c
T|RW|a5|0f|f0|3c
T|RC|a5|a5|a5|00
T|RS|a5|a5|a5|ff
T|WC|a5|00|00|3c
T|WS|a5|ff|ff|3c
T|WRC|a5|0f|f0|00
T|WRS|a5|0f|f0|ff
T|WSRC|a5|ff|ff|00
T|WCRS|a5|00|00|ff
T|W1C|a5|a0|00|3c
T|W1S|a5|af|ff|3c
T|W1T|a5|aa|5a|3c
T|W0C|a5|05|00|3c
T|W0S|a5|f5|ff|3c
T|W0T|a5|55|5a|3c
T|W1SRC|a5|af|ff|00
T|W1CRS|a5|a0|00|ff
T|W0SRC|a5|f5|ff|00
T|W0CRS|a5|05|00|ff
T|WO|a5|0f|f0|f0
T|WOC|a5|00|00|00
T|WOS|a5|ff|ff|ff
T|W1|a5|0f|0f|3c
T|WO1|a5|0f|0f|0f";

#[test]
fn uvm_ral_field_access_policies() {
    let src = include_str!("../uvm/uvm_ral_access_policies.sv");
    let want: Vec<&str> = ACCESS_POLICIES.lines().collect();
    for v in VERSIONS {
        let out = lines(&run(v, src, &[]));
        assert_eq!(tagged(&out), want, "UVM {v}: access-policy table");
        assert_summary(v, &out, 0);
    }
}

/// Rows both versions agree on: rights RO, rights RW, and the WO rows whose
/// declared policy is RW, WO or a read-only one.
const RIGHTS_COMMON: &str = "\
T|RO|RW|RO|3c
T|RO|RO|RO|3c
T|RO|W1C|RO|3c
T|RO|W1|RO|3c
T|RO|RC|RC|00
T|RO|WRC|RC|00
T|RO|WSRC|RC|00
T|RO|RS|RS|ff
T|RO|WRS|RS|ff
T|RO|WCRS|RS|ff
T|RO|WO|NOACCESS|a5
T|RO|WOC|NOACCESS|a5
T|RO|WO1|NOACCESS|a5
T|WO|RW|WO|f0
T|WO|WO|WO|f0
T|WO|RO|NOACCESS|a5
@
T|WO|RC|NOACCESS|a5
T|WO|RS|NOACCESS|a5
@@
T|RW|W1C|W1C|3c";

/// UVM 1.2: under WO rights everything except RW and WO is NOACCESS.
const RIGHTS_WO_1_2: [&str; 9] = [
    "T|WO|W1C|NOACCESS|a5",
    "T|WO|WRC|NOACCESS|a5",
    "T|WO|WRS|NOACCESS|a5",
    "T|WO|W1SRC|NOACCESS|a5",
    "T|WO|W1CRS|NOACCESS|a5",
    "T|WO|WCRS|NOACCESS|a5",
    "T|WO|WSRC|NOACCESS|a5",
    "T|WO|W1|NOACCESS|a5",
    "T|WO|WO1|NOACCESS|a5",
];

/// 1800.2-2020: WO rights strip only the read side of the policy.
const RIGHTS_WO_2020: [&str; 9] = [
    "T|WO|W1C|W1C|3c",
    "T|WO|WRC|WO|f0",
    "T|WO|WRS|WO|f0",
    "T|WO|W1SRC|W1S|3c",
    "T|WO|W1CRS|W1C|3c",
    "T|WO|WCRS|WC|3c",
    "T|WO|WSRC|WS|3c",
    "T|WO|W1|W1|3c",
    "T|WO|WO1|WO1|0f",
];

#[test]
fn uvm_ral_map_rights() {
    let src = include_str!("../uvm/uvm_ral_map_rights.sv");
    for v in VERSIONS {
        let wo = if v == "1.2" {
            RIGHTS_WO_1_2
        } else {
            RIGHTS_WO_2020
        };
        // The bench's order: the common rows with `@` standing for the first
        // version-specific row and `@@` for the remaining eight.
        let mut want: Vec<&str> = Vec::new();
        for l in RIGHTS_COMMON.lines() {
            match l {
                "@" => want.push(wo[0]),
                "@@" => want.extend_from_slice(&wo[1..]),
                _ => want.push(l),
            }
        }
        let out = lines(&run(v, src, &[]));
        assert_eq!(tagged(&out), want, "UVM {v}: map-rights table");
        assert_summary(v, &out, 0);
    }
}

fn bus(v: &str, test: &str) -> Vec<String> {
    let src = include_str!("../uvm/uvm_ral_bus.sv");
    lines(&run(v, src, &[&format!("+UVM_TESTNAME={test}")]))
}

/// A NO_MAP register's SECOND backdoor `write` trampolines its resumed
/// `do_write` continuation (an inlined `foreach (m_fields[i])` that must
/// resolve the live `this`) through the event queue; the resumed parent was
/// once mistaken for a fork child and its live context clobbered, so the
/// field loop read through a phantom null `this` and xezim (internally
/// catching it) logged `map_info.frontdoor` null-derefs. 25 fields plus a
/// `read`+`peek` before the two writes push the inlined chain past the
/// trampoline depth. 1800.2-2020 only: UVM 1.2's `UVM_NO_DPI` build has
/// backdoor uvm_hdl compiled off and aborts before the loop.
#[test]
fn uvm_ral_backdoor_second_write_keeps_this() {
    let src = include_str!("../uvm/uvm_ral_backdoor_toggle_write2.sv");
    let out = lines(&run("1800.2-2020", src, &[]));
    assert_eq!(
        tagged(&out),
        ["T|write1|mirror=0000000000000000", "T|write2|mirror=0000000000000000"]
    );
    // The bug signal is a null dereference xezim logs (and catches) when the
    // second write's field loop reads through the clobbered `this`. The
    // `UVM/DPI/HDL_GET` errors are expected noise (no DUT behind the NO_MAP
    // backdoor), so reject only the simulator's own error/fatal lines.
    assert!(
        !out.iter().any(|l| l.contains("[xezim][error]") || l.contains("[xezim][fatal]")),
        "1800.2-2020: simulator error:
{out:#?}"
    );
}

#[test]
fn uvm_ral_frontdoor_policies_and_update() {
    for v in VERSIONS {
        let out = bus(v, "ral_fd_test");
        assert_eq!(
            tagged(&out),
            [
                // set() writes nothing; update() writes SCRATCH and the
                // volatile EVENTS, which still needs update afterwards
                "T|upd|before scratch=1 events=1 ctrl=0 bus_writes=0 mirror=0",
                "T|upd|after scratch=0 events=1 block=1 bus_writes=2 mirror=beef",
                "T|w1c|read=3c mirror=30",
                "T|rc|first=55 mirror=0",
                "T|rc|second=0",
                "T|ro|read=cafe0001 mirror=cafe0001",
            ],
            "UVM {v}"
        );
        assert_summary(v, &out, 0);
    }
}

#[test]
fn uvm_ral_hw_reset_seq() {
    for v in VERSIONS {
        let out = bus(v, "ral_reset_seq_test");
        assert_eq!(tagged(&out), ["T|reset_seq|done"], "UVM {v}");
        assert_summary(v, &out, 0);
    }
}

#[test]
fn uvm_ral_mirror_check_mismatch() {
    for v in VERSIONS {
        let out = bus(v, "ral_mismatch_test");
        assert_eq!(
            tagged(&out),
            ["T|mismatch|mirror=11", "T|mismatch|done"],
            "UVM {v}"
        );
        // exactly the one mismatch the test provokes
        let mism: Vec<&String> = out
            .iter()
            .filter(|l| l.starts_with("UVM_ERROR @"))
            .collect();
        assert_eq!(mism.len(), 1, "UVM {v}: {mism:#?}");
        assert!(
            mism[0].contains("\"blk.CTRL\"")
                && mism[0].contains(
                    "(0x0000000000000011) does not match mirrored value (0x00000000000000a5)"
                ),
            "UVM {v}: {}",
            mism[0]
        );
        assert_summary(v, &out, 1);
    }
}

/// With auto-prediction off, only `uvm_reg_predictor` keeps the mirror
/// current. Its `write()` holds a `uvm_reg_bus_op rw` struct local while the
/// register task that started the access waits with a `uvm_reg_item rw`;
/// the struct's member layout once leaked onto the task's handle, and the
/// task's `rw.status = ...` then wrote a bit slice into the handle (the read
/// data was lost and the library reported null dereferences).
#[test]
fn uvm_ral_explicit_predictor() {
    for v in VERSIONS {
        let out = bus(v, "ral_pred_test");
        assert_eq!(
            tagged(&out),
            [
                "T|pred|raw scratch mirror=1234",
                "T|pred|ctrl mirror=77",
                "T|pred|events read=9 mirror=0",
            ],
            "UVM {v}"
        );
        assert_summary(v, &out, 0);
    }
}

#[test]
fn uvm_ral_mem_access() {
    for v in VERSIONS {
        let out = bus(v, "ral_mem_test");
        assert_eq!(
            tagged(&out),
            [
                "T|mem|0=a000",
                "T|mem|1=0",
                "T|mem|2=0",
                "T|mem|3=a003",
                "T|mem|4=0",
                "T|mem|5=0",
                "T|mem|6=a006",
                "T|mem|7=0",
                "T|mem|addr3=4c size=8",
            ],
            "UVM {v}"
        );
        assert_summary(v, &out, 0);
    }
}
