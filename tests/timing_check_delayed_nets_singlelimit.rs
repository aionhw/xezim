//! §15.6 vendor-extension single-limit timing checks — `$recovery`/`$hold`
//! with trailing `delayed_reference, delayed_data` args (8-arg form) must
//! create and drive those delayed nets, exactly like the LRM 13-arg
//! `$setuphold`/`$recrem`. Gate libraries wire UDP terminals from these nets
//! (`not (rst_n_int, del_rst)`), so dropping them leaves the flop's clock/reset X — a
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
primitive udp_ff_gate (out, in, clk, clr_);
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
module ff_cell (qo, ck, di, rst);
output qo; input di, ck, rst;
reg notify_reg;
  not g_inv (rst_n_int, del_rst);
  buf g_ck (ck_int, del_ck);
  udp_ff_gate g_ff (ff_q, del_di, ck_int, rst_n_int);
  buf g_q (qo, ff_q);
  specify
    (posedge ck => (qo +: di)) = (1, 1);
    $setuphold(posedge ck &&& (rst == 1'b0), di, 1, 1, notify_reg, , , del_ck, del_di);
    $recovery(negedge rst, posedge ck, 1, notify_reg, , , del_rst, del_ck);
    $hold(posedge ck, negedge rst, 1, notify_reg, , , del_ck, del_rst);
  endspecify
endmodule
`endcelldefine
module tb;
  reg t_ck, t_rst, t_di; wire t_q; integer tog;
  ff_cell dut (.qo(t_q), .ck(t_ck), .di(t_di), .rst(t_rst));
  always #5 t_ck = ~t_ck;
  always @(t_q) tog = tog + 1;
  initial begin
    t_ck = 0; t_rst = 1; t_di = 0; tog = 0;
    #3 t_rst = 0; t_di = 1;
    #10 t_di = 0;
    #10 t_di = 1;
    #10 $display("Q=%b tog_gt0=%0d", t_q, tog > 0);
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
