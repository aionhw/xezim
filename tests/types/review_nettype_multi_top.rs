//! IEEE 1800-2023 §6.6.7, §3.12.1: a `$unit` nettype's resolution function
//! runs in a design with two top-level modules (here an unused `spare`). The
//! synthetic root of such a design registered no `$unit` nettype, so the net
//! read its parent's driver alone (2.0); the reference simulator resolves the
//! three drivers to their sum, 5.0.

#[test]
fn unit_resolver_runs_with_two_tops() {
    let out = xezim::simulate(
        r#"
function automatic real rsum(input real d[]); real s = 0; foreach (d[i]) s += d[i]; return s; endfunction
nettype real rnet with rsum;
module drv(input real v, output rnet p); assign p = v; endmodule
module spare; endmodule
module top; real v = 1.5; rnet n; drv a(.v(v), .p(n)); drv b(.v(v), .p(n)); assign n = 2.0;
  initial #1 $display("R %.1f", n); endmodule
"#,
        1_000_000,
    )
    .expect("simulate failed");
    let lines: Vec<_> = out
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert!(lines.iter().any(|l| l == "R 5.0"), "{lines:?}");
}
