//! UVM transaction-level modeling (TLM) against the real Accellera library,
//! on UVM 1.2 and IEEE 1800.2-2020. The suite's older TLM tests in
//! `uvm_integration_tests` run on 1.2 only and cover blocking/nonblocking
//! put/get/peek, a bounded FIFO and `b_transport`. The benches here, in
//! `tests/uvm/`, add:
//!
//! - `uvm_tlm_transport.sv`: TLM-1 transport (blocking and nonblocking on one
//!   imp), `uvm_tlm_req_rsp_channel` with master/slave ports and its
//!   request/response analysis ports, and `uvm_tlm_transport_channel`.
//! - `uvm_tlm_connect.sv`: hierarchical port -> port -> export -> imp
//!   binding, a two-imp port steered with `set_if()`, analysis fan-out
//!   through an export, and `uvm_tlm_fifo`'s `put_ap`/`get_ap`.
//! - `uvm_tlm_connect_errors.sv`: the "Connection Error" checks and the
//!   BUILDERR stop they lead to.
//! - `uvm_tlm2_sockets.sv`: TLM-2 blocking transport through passthrough
//!   sockets and nonblocking transport on the forward and backward paths.
//!
//! Every expected line was derived by hand from the UVM source. Lines that
//! different processes print in the same time step are compared per process,
//! so the tests do not depend on the order of same-time processes.
//!
//! Like `uvm_feature_coverage`, 1800.2-2020 runs with its DPI live and 1.2
//! runs `UVM_NO_DPI` (see that file for why).

use xezim::*;

const VERSIONS: [&str; 2] = ["1.2", "1800.2-2020"];

fn run(version: &str, src: &str) -> Vec<String> {
    let src_dir = crate::uvm_integration_tests::uvm_dir()
        .join(version)
        .join("src");
    let uvm_pkg = std::fs::read_to_string(src_dir.join("uvm_pkg.sv"))
        .unwrap_or_else(|e| panic!("read {}/src/uvm_pkg.sv: {}", version, e));
    let mut defines = vec![("UVM_REPORT_DISABLE_FILE_LINE".to_string(), None)];
    if version == "1.2" {
        defines.push(("UVM_NO_DPI".to_string(), None));
    }
    let sim = simulate_multi(
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
        &[],
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
    .unwrap_or_else(|e| panic!("UVM {} bench failed to simulate: {}", version, e));
    sim.output
        .iter()
        .flat_map(|o| o.message.lines().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// The bench's tagged lines that start with `prefix`, in order.
fn with_prefix<'a>(out: &'a [String], prefix: &str) -> Vec<&'a str> {
    out.iter()
        .map(String::as_str)
        .filter(|l| l.starts_with(prefix))
        .collect()
}

/// The end-of-run summary, and nothing reported by the simulator itself.
fn assert_summary(version: &str, out: &[String], errors: u32, fatals: u32) {
    for want in [
        format!("UVM_WARNING :    0"),
        format!("UVM_ERROR :    {errors}"),
        format!("UVM_FATAL :    {fatals}"),
    ] {
        assert!(
            out.iter().any(|l| l.trim_start() == want),
            "UVM {version}: expected {want:?} in:\n{out:#?}"
        );
    }
    assert!(
        !out.iter()
            .any(|l| l.contains("[xezim][error]") || l.contains("[xezim][fatal]")),
        "UVM {version}: simulator error:\n{out:#?}"
    );
}

pub(super) fn uvm_tlm1_transport_and_channels() {
    let src = include_str!("../uvm/uvm_tlm_transport.sv");
    for v in VERSIONS {
        let out = run(v, src);
        // the blocking call holds the target for 3 units; a nonblocking call
        // meanwhile is refused, and one after it answers at once
        assert_eq!(
            with_prefix(&out, "T|transport|"),
            [
                "T|transport|nb busy ok=0 t=1",
                "T|transport|b sum=7 t=3",
                "T|transport|nb idle ok=1 product=12 t=3",
            ],
            "UVM {v}"
        );
        // the channel's FIFOs hold one item each: the third request is
        // accepted only when the slave takes the second (t=5), and the
        // responses follow at the slave's 2-unit pace
        assert_eq!(
            with_prefix(&out, "T|req_rsp|slave"),
            [
                "T|req_rsp|slave peek a=10 t=3",
                "T|req_rsp|slave put sum=11 t=5",
                "T|req_rsp|slave peek a=20 t=5",
                "T|req_rsp|slave put sum=22 t=7",
                "T|req_rsp|slave peek a=30 t=7",
                "T|req_rsp|slave put sum=33 t=9",
            ],
            "UVM {v}"
        );
        let master: Vec<&str> = with_prefix(&out, "T|req_rsp|")
            .into_iter()
            .filter(|l| !l.starts_with("T|req_rsp|slave"))
            .collect();
        assert_eq!(
            master,
            [
                "T|req_rsp|3 requests put t=5",
                "T|req_rsp|master got sum=11 t=5",
                "T|req_rsp|master got sum=22 t=7",
                "T|req_rsp|master got sum=33 t=9",
                "T|req_rsp|request_ap=3 response_ap=3",
            ],
            "UVM {v}"
        );
        assert_eq!(
            with_prefix(&out, "T|tchan|"),
            [
                "T|tchan|slave peek a=100 t=9",
                "T|tchan|slave put sum=101 t=11",
                "T|tchan|transport sum=101 t=11",
            ],
            "UVM {v}"
        );
        assert_summary(v, &out, 0, 0);
    }
}

pub(super) fn uvm_tlm1_connection_topology() {
    let src = include_str!("../uvm/uvm_tlm_connect.sv");
    for v in VERSIONS {
        let out = run(v, src);
        assert_eq!(
            with_prefix(&out, "T|"),
            [
                "T|hier|prod.port size=1 agt.port size=1 senv.exp size=1",
                "T|hier|resolves to uvm_test_top.senv.leaf.imp",
                "T|multi|size=2 if0=uvm_test_top.ma.imp if1=uvm_test_top.mb.imp",
                "T|fanout|ap size=5 lonely size=0",
                "T|hier|leaf got '{11, 22, 33} t=3",
                "T|multi|ma='{1, 4} mb='{2, 3}",
                "T|fanout|d=2,2,2 subs=2,2 last=8,8,8,8,8",
                "T|fifo_ap|log= P1 P2 P3 G1 G2 used=1",
            ],
            "UVM {v}"
        );
        assert_summary(v, &out, 0, 0);
    }
}

pub(super) fn uvm_tlm1_connection_errors() {
    let src = include_str!("../uvm/uvm_tlm_connect_errors.sv");
    for v in VERSIONS {
        let out = run(v, src);
        // connect_phase carries on past the connect-time checks; the BUILDERR
        // stop keeps end_of_elaboration_phase and run_phase from running
        assert_eq!(
            with_prefix(&out, "T|"),
            [
                "T|connect|imp.connect returned",
                "T|connect|export.connect(port) returned",
            ],
            "UVM {v}"
        );
        let errors: Vec<&str> = with_prefix(&out, "UVM_ERROR @ 0: ");
        assert_eq!(
            errors,
            [
                "UVM_ERROR @ 0: uvm_test_top.s1.imp [Connection Error] Cannot call an imp port's \
                 connect method. An imp is connected only to the component passed in its \
                 constructor. (You attempted to bind this imp to uvm_test_top.s2.imp)",
                "UVM_ERROR @ 0: uvm_test_top.bad_exp.exp [Connection Error] Cannot connect \
                 exports to ports Try calling port.connect(export) instead. (You attempted to \
                 bind this export to uvm_test_top.bad_exp.port).",
                "UVM_ERROR @ 0: uvm_test_top.over.port [Connection Error] connection count of 2 \
                 exceeds maximum of 1",
                "UVM_ERROR @ 0: uvm_test_top.unconnected.port [Connection Error] connection \
                 count of 0 does not meet required minimum of 1",
            ],
            "UVM {v}: the optional (min 0) port must not be flagged"
        );
        assert!(
            out.iter()
                .any(|l| l == "UVM_FATAL @ 0: reporter [BUILDERR] stopping due to build errors"),
            "UVM {v}: no BUILDERR stop:\n{out:#?}"
        );
        assert_summary(v, &out, 4, 1);
    }
}

pub(super) fn uvm_tlm2_passthrough_and_nonblocking_sockets() {
    let src = include_str!("../uvm/uvm_tlm2_sockets.sv");
    for v in VERSIONS {
        let out = run(v, src);
        assert_eq!(
            with_prefix(&out, "T|b|"),
            [
                "T|b|write resp=OK delay=5.0 t=2",
                "T|b|read resp=OK ok=1 data=0000a0a1a2a30000 delay=10.0 t=4",
                "T|b|overrun resp=ADDRESS_ERROR ok=0 calls=3",
            ],
            "UVM {v}"
        );
        // the target's backward call and the initiator's handling of it run
        // in the target's process; the forward calls in the test's
        assert_eq!(
            with_prefix(&out, "T|nb|"),
            [
                "T|nb|fw addr=10 phase=BEGIN_REQ t=6",
                "T|nb|fw returned UVM_TLM_ACCEPTED phase=BEGIN_REQ t=6",
                "T|nb|bw addr=10 phase=BEGIN_RESP resp=OK t=9",
                "T|nb|bw returned UVM_TLM_COMPLETED phase=END_RESP t=9",
                "T|nb|fw addr=20 phase=BEGIN_REQ t=10",
                "T|nb|fw returned UVM_TLM_UPDATED phase=END_REQ delay=7.0 t=10",
                "T|nb|fw addr=30 phase=BEGIN_REQ t=10",
                "T|nb|fw returned UVM_TLM_COMPLETED resp=OK t=10",
            ],
            "UVM {v}"
        );
        assert_summary(v, &out, 0, 0);
    }
}
