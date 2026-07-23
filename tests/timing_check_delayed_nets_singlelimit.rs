//! §15.6 vendor-extension single-limit timing checks — `$recovery`/`$hold`
//! with trailing `delayed_reference, delayed_data` args (8-arg form) must
//! create and drive those delayed nets, exactly like the LRM 13-arg
//! `$setuphold`/`$recrem`. Gate libraries wire UDP terminals from these nets
//! (`not (xRN, dR)`), so dropping them leaves the flop's clock/reset X — a
//! dead-clock at gate level.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn extended_recovery_hold_delayed_nets_drive_udp() {
    let src = r#"
primitive udp_dff_x (out, in, clk, clr_);
   output out; input in, clk, clr_; reg out;
   table
   0 r 1 : ? : 0 ;
   1 r 1 : ? : 1 ;
   ? f ? : ? : - ;
   * b ? : ? : - ;
   ? ? 0 : ? : 0 ;
   endtable
endprimitive
`celldefine
module DFF_cell (Q, CK, D, R);
output Q; input D, CK, R;
reg NOTIFIER;
  not XX0 (xRN, dR);
  buf IC (clk, dCK);
  udp_dff_x I0 (n0, dD, clk, xRN);
  buf IQ (Q, n0);
  specify
    (posedge CK => (Q +: D)) = (1, 1);
    $setuphold(posedge CK &&& (R == 1'b0), D, 1, 1, NOTIFIER, , , dCK, dD);
    $recovery(negedge R, posedge CK, 1, NOTIFIER, , , dR, dCK);
    $hold(posedge CK, negedge R, 1, NOTIFIER, , , dCK, dR);
  endspecify
endmodule
`endcelldefine
module tb;
  reg CK, R, D; wire Q; integer tog;
  DFF_cell dut (.Q(Q), .CK(CK), .D(D), .R(R));
  always #5 CK = ~CK;
  always @(Q) tog = tog + 1;
  initial begin
    CK = 0; R = 1; D = 0; tog = 0;
    #3 R = 0; D = 1;
    #10 D = 0;
    #10 D = 1;
    #10 $display("Q=%b tog_gt0=%0d", Q, tog > 0);
    $finish;
  end
endmodule
"#;
    let out = lines(src);
    assert!(
        out.iter().any(|l| l == "Q=1 tog_gt0=1"),
        "extended $recovery/$hold delayed nets not driven (flop dead): {:?}",
        out
    );
}
