//! §16.6/§16.8 — named properties and sequences declared inside an INLINED
//! instance (an interface or sub-module, including one attached by `bind`).
//!
//! The inliner never registered them, so `assert property (p(a))` in an
//! assertion interface found no body and degraded to an unclocked probe that
//! never ran its pass action: an AVIP's `$info("...: ASSERTED")` lines were
//! missing. Each instance also shares the assertion's source span, and the
//! clocked-site registry deduplicated by span alone, so only the FIRST
//! instance of an assertion interface ever evaluated.
//!
//! The expected lines were cross-checked against the reference simulator.

use xezim::simulate;

const SRC: &str = r#"
interface chk(input aclk, input aresetn, input valid, input ready);
  bit defaultState;
  property p_valid(logic v);
    @(posedge aclk) (aresetn === 0) |-> (v === 0);
  endproperty
  property p_ready(logic r);
    @(posedge aclk) (aresetn === 0) |-> (r === defaultState);
  endproperty
  // The formal shadows the interface port of the same name.
  property p_shadow(logic valid);
    @(posedge aclk) (aresetn === 0) |-> (valid === 1);
  endproperty
  sequence s_rst;
    (aresetn === 0);
  endsequence
  A_VALID: assert property (p_valid(valid)) $info("A_VALID : ASSERTED"); else $error("A_VALID : NOT ASSERTED");
  A_READY: assert property (p_ready(ready)) $display("%0t A_READY pass", $time); else $display("%0t A_READY fail", $time);
  A_SHADOW: assert property (p_shadow(ready)) $display("%0t A_SHADOW pass", $time); else $display("%0t A_SHADOW fail", $time);
  A_SEQ: assert property (@(posedge aclk) s_rst |-> (valid === 0)) $display("%0t A_SEQ pass", $time); else $display("%0t A_SEQ fail", $time);
endinterface
module mon(input aclk, input aresetn, input valid, input ready);
endmodule
module top;
  initial $assertvacuousoff(0);
  bit aclk; bit aresetn; logic valid; logic ready;
  always #10 aclk = ~aclk;
  initial begin
    aresetn = 1; valid = 0; ready = 1;
    #20 aresetn = 0;
    #20 aresetn = 1;
    #20 $finish;
  end
  mon m(.aclk(aclk), .aresetn(aresetn), .valid(valid), .ready(ready));
  bind mon chk c(.aclk(aclk), .aresetn(aresetn), .valid(valid), .ready(ready));
  chk direct(.aclk(aclk), .aresetn(aresetn), .valid(valid), .ready(1'b0));
endmodule
"#;

#[test]
fn named_properties_run_in_every_instance() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let mut got: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.contains("ASSERTED") || m.starts_with("30 "))
        .collect();
    got.sort();
    // One line per instance; the two instances differ only in `ready`.
    let mut want = vec![
        "** Info: A_VALID : ASSERTED",
        "** Info: A_VALID : ASSERTED",
        "30 A_READY fail",
        "30 A_READY pass",
        "30 A_SEQ pass",
        "30 A_SEQ pass",
        "30 A_SHADOW fail",
        "30 A_SHADOW pass",
    ];
    want.sort();
    assert_eq!(got, want);
}
