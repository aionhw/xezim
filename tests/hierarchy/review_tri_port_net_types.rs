//! A port of net type tri0, tri1 or supply merged with the net connected to
//! it (IEEE 1800-2023 §6.6.5, §6.6.6, §23.3.3.7 and Table 23-1), found
//! reviewing #1060. Every expected line is the reference simulator's output.

use xezim::simulate;

/// The `T|` lines of `src`, in output order.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// Table 23-1: an internal tri0 or tri1 net dominates an external `wire`, so
/// the merged net pulls, seen alike inside and outside: for an inout, an
/// input (also with no driver at all) and an output port, through two
/// levels of hierarchy, on a vector, on one bit of a vector, across an
/// instance array, on an implicit net and in a generate block. An external
/// tri0 or tri1 net keeps its own type, the first of two different pulls on
/// one wire decides, and a port on a variable keeps its pull to itself.
#[test]
fn a_pulled_port_merges_with_the_connected_net() {
    let got = t_lines(
        r#"
module s1 (inout tri1 p); logic en = 0; int wk = 0; assign p = en ? 1'b0 : 1'bz; always @(p) wk++; endmodule
module s0 (inout tri0 p); logic en = 0; int wk = 0; assign p = en ? 1'b1 : 1'bz; always @(p) wk++; endmodule
module si1 (input tri1 p); int wk = 0; always @(p) wk++; endmodule
module so1 (output tri1 p); logic en = 0; assign p = en ? 1'b0 : 1'bz; endmodule
module sv1 (inout tri1 [3:0] p); logic en = 0; assign p = en ? 4'b0101 : 4'bzzzz; endmodule
module sand (inout wand p); logic en = 0; assign p = 1'b1; assign p = en ? 1'b0 : 1'b1; endmodule
module mid (inout wire q); s1 leaf(.p(q)); endmodule
module mid2 (inout q); mid mm(.q(q)); endmodule
module t;
  wire w1, w0, wi, wo, wn, wn2, p, wb0, wimp;
  wire [3:0] wv; wire [3:0] wbit; wire [1:0] warr; wire wshare; wire wa;
  tri0 wt0; tri1 wt1;
  logic r = 1'bz; logic drv_en = 0;
  wire wconf;
  s1 u1(.p(w1));
  s0 u0(.p(w0));
  si1 ui(.p(wi));
  si1 uir(.p(r));
  so1 uo(.p(wo));
  mid um(.q(wn));
  mid2 um2(.q(wn2));
  s1 up(.p(p));
  sv1 uv(.p(wv));
  s1 ub(.p(wbit[2]));
  s1 uarr[1:0](.p(warr));
  s1 ush1(.p(wshare)); s1 ush2(.p(wshare));
  s1 ut0(.p(wt0));
  s0 ut1(.p(wt1));
  s1 uc1(.p(wconf)); s0 uc0(.p(wconf));
  sand ua(.p(wa));
  s1 uimp(.p(wimpl));
  s1 unc();
  for (genvar i = 0; i < 2; i++) begin : g wire gw; s1 s(.p(gw)); end
  task show(string tag);
    $display("T|%s w1=%b/%b w0=%b/%b wi=%b/%b r=%b/%b wo=%b/%b wn=%b/%b wn2=%b/%b p=%b/%b", tag, w1, u1.p, w0, u0.p, wi, ui.p, r, uir.p, wo, uo.p, wn, um.leaf.p, wn2, um2.mm.leaf.p, p, up.p);
    $display("T|%s wv=%b/%b wbit=%b/%b warr=%b/%b%b wshare=%b wt0=%b/%b wt1=%b/%b wconf=%b wa=%b/%b wimpl=%b/%b unc=%b gw=%b%b", tag, wv, uv.p, wbit, ub.p, warr, uarr[1].p, uarr[0].p, wshare, wt0, ut0.p, wt1, ut1.p, wconf, wa, ua.p, wimpl, uimp.p, unc.p, g[1].gw, g[0].gw);
    $display("T|%s wk: u1=%0d u0=%0d ui=%0d up=%0d leaf=%0d", tag, u1.wk, u0.wk, ui.wk, up.wk, um.leaf.wk);
  endtask
  initial begin
    #1; show("a");
    u1.en = 1; u0.en = 1; uo.en = 1; um.leaf.en = 1; um2.mm.leaf.en = 1; up.en = 1; uv.en = 1; ub.en = 1; uarr[0].en = 1; ush2.en = 1; ut0.en = 1; ut1.en = 1; ua.en = 1; uimp.en = 1; g[0].s.en = 1; unc.en = 1;
    #1; show("b");
    u1.en = 0; u0.en = 0; up.en = 0; um.leaf.en = 0;
    #1; show("c");
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|a w1=1/1 w0=0/0 wi=1/1 r=z/1 wo=1/1 wn=1/1 wn2=1/1 p=1/1",
            "T|a wv=1111/1111 wbit=z1zz/1 warr=11/11 wshare=1 wt0=0/0 wt1=1/1 wconf=1 wa=1/1 wimpl=1/1 unc=1 gw=11",
            "T|a wk: u1=1 u0=1 ui=1 up=1 leaf=1",
            "T|b w1=0/0 w0=1/1 wi=1/1 r=z/1 wo=0/0 wn=0/0 wn2=0/0 p=0/0",
            "T|b wv=0101/0101 wbit=z0zz/0 warr=10/10 wshare=0 wt0=0/0 wt1=1/1 wconf=1 wa=0/0 wimpl=0/0 unc=0 gw=10",
            "T|b wk: u1=2 u0=2 ui=1 up=2 leaf=2",
            "T|c w1=1/1 w0=0/0 wi=1/1 r=z/1 wo=0/0 wn=1/1 wn2=0/0 p=1/1",
            "T|c wv=0101/0101 wbit=z0zz/0 warr=10/10 wshare=0 wt0=0/0 wt1=1/1 wconf=1 wa=0/0 wimpl=0/0 unc=0 gw=10",
            "T|c wk: u1=3 u0=3 ui=1 up=3 leaf=3",
        ]
    );
}

/// Table 23-1, §6.6.6: supply ports, a part-select actual, a select reached
/// through an intermediate port, both pulls on one net inside an instance,
/// an input tri0 port on one bit, a trireg port, different pulls on
/// neighbouring bits and on one bit, a tri1 port on a bit of a tri0 vector,
/// and an instance array of tri0 ports.
#[test]
fn pulled_ports_on_selects_and_supply_ports() {
    let got = t_lines(
        r#"
module ps1 (inout supply1 p); endmodule
module ps0 (inout supply0 p); endmodule
module pv1 (inout tri1 [3:0] p); logic en = 0; assign p = en ? 4'b0101 : 4'bzzzz; endmodule
module pb1 (inout tri1 p); logic en = 0; assign p = en ? 1'b0 : 1'bz; endmodule
module pb0 (inout tri0 p); logic en = 0; assign p = en ? 1'b1 : 1'bz; endmodule
module pi0 (input tri0 p); endmodule
module ptr (inout trireg p); endmodule
module midsel (inout wire [3:0] q); pb1 leaf(.p(q[1])); endmodule
module midw (inout wire q); pb1 leaf(.p(q)); pb0 leaf0(.p(q)); endmodule
module t;
  wire ws1, ws0; wire [7:0] wps; wire [3:0] wcat_a; wire wcat_b; wire [3:0] wms; wire wmw;
  wire [3:0] wi0; wire wtr; wire [3:0] wmix; tri0 [3:0] wt0sel; wire [2:0] warr0;
  wire [1:0] w2d [0:1];
  ps1 us1(.p(ws1)); ps0 us0(.p(ws0));
  pv1 ups(.p(wps[5:2]));
  pb1 ucat(.p(wcat_b));
  midsel ums(.q(wms));
  midw umw(.q(wmw));
  pi0 ui0(.p(wi0[3]));
  ptr utr(.p(wtr));
  pb1 um1(.p(wmix[0])); pb0 um0(.p(wmix[1])); pb1 um2(.p(wmix[2])); pb0 um3(.p(wmix[2]));
  pb1 ut0(.p(wt0sel[1]));
  pb0 uarr0[2:0](.p(warr0));
  pb1 u2d(.p(w2d[1][0]));
  for (genvar i = 0; i < 3; i++) begin : g wire [2:0] gw; pb1 s(.p(gw[i])); end
  task show(string tag);
    $display("T|%s ws1=%b ws0=%b wps=%b/%b wcat=%b wms=%b/%b wmw=%b wi0=%b/%b wtr=%b wmix=%b wt0sel=%b warr0=%b gw=%b_%b_%b", tag, ws1, ws0, wps, ups.p, wcat_b, wms, ums.leaf.p, wmw, wi0, ui0.p, wtr, wmix, wt0sel, warr0, g[0].gw, g[1].gw, g[2].gw);
  endtask
  initial begin
    #1; show("a");
    ups.en = 1; ucat.en = 1; ums.leaf.en = 1; umw.leaf.en = 1; um1.en = 1; um0.en = 1; um3.en = 1; ut0.en = 1; uarr0[1].en = 1; u2d.en = 1; g[1].s.en = 1;
    #1; show("b");
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|a ws1=1 ws0=0 wps=zz1111zz/1111 wcat=1 wms=zz1z/1 wmw=1 wi0=0zzz/0 wtr=z wmix=z101 wt0sel=0000 warr0=000 gw=zz1_z1z_1zz",
            "T|b ws1=1 ws0=0 wps=zz0101zz/0101 wcat=0 wms=zz0z/0 wmw=0 wi0=0zzz/0 wtr=z wmix=z110 wt0sel=0000 warr0=010 gw=zz1_z0z_1zz",
        ]
    );
}
