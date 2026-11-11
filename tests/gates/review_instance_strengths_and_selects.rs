//! Drive strengths, unsized literals and select drivers on nets inside
//! instances (IEEE 1800-2023 §5.7.1, §6.5, §6.6.5, §6.7, §28.12), found
//! reviewing #1061, #1062 and #1063. Every expected line is the reference
//! simulator's output.

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

/// §6.7, §28.12: a strength on a net declaration assignment, on continuous
/// assignments to a vector, to the bits of a generate loop and to an
/// overlapping part-select resolves the same in a module instance, an
/// instance array, a nested instance and a generate block as it does at the
/// top level, and `%v` reports the strength of a single driver there.
#[test]
fn strengths_resolve_inside_instances() {
    let got = t_lines(
        r#"
module sub;
  logic en = 0;
  wire [7:0] a, b, c, d;
  wire (weak0, weak1) [7:0] nd = 8'hff;
  assign nd = en ? 8'hc3 : 'z;
  assign (pull0, pull1) a = 8'h0f;
  assign (weak0, weak1) a = 8'hf0;
  assign (supply0, supply1) b = 8'haa;
  assign b = en ? 8'h55 : 'z;
  assign (strong0, highz1) c = 8'h0f;
  assign (weak0, weak1) c = 8'hf3;
  assign (weak0, weak1) d = '1;
  assign d = en ? 8'hc3 : 'z;
  wire [3:0] gn;
  for (genvar k = 0; k < 4; k++) begin : gl
    assign (weak0, weak1) gn[k] = 1'b1;
    assign gn[k] = en ? 1'b0 : 1'bz;
  end
  wire [3:0] ps;
  assign (weak0, weak1) ps[3:1] = 3'b111;
  assign ps[2:0] = en ? 3'b000 : 3'bzzz;
  wire one;
  assign (pull0, pull1) one = 1'b1;
  wire (weak0, weak1) wk = 1'b0;
  task show(string tag);
    $display("T|%s nd=%h a=%h b=%h c=%h d=%h gn=%b ps=%b one=%v wk=%v", tag, nd, a, b, c, d, gn, ps, one, wk);
  endtask
endmodule
module mid; sub s(); endmodule
module t;
  sub u();
  sub ua[1:0]();
  mid m();
  for (genvar i = 0; i < 2; i++) begin : gb sub s(); end
  initial begin
    #1; u.show("u"); ua[1].show("ua1"); m.s.show("m"); gb[1].s.show("gb1");
    u.en = 1; ua[1].en = 1; m.s.en = 1; gb[1].s.en = 1;
    #1; u.show("u"); ua[1].show("ua1"); m.s.show("m"); gb[1].s.show("gb1"); ua[0].show("ua0");
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|u nd=ff a=0f b=aa c=03 d=ff gn=1111 ps=111z one=Pu1 wk=We0",
            "T|ua1 nd=ff a=0f b=aa c=03 d=ff gn=1111 ps=111z one=Pu1 wk=We0",
            "T|m nd=ff a=0f b=aa c=03 d=ff gn=1111 ps=111z one=Pu1 wk=We0",
            "T|gb1 nd=ff a=0f b=aa c=03 d=ff gn=1111 ps=111z one=Pu1 wk=We0",
            "T|u nd=c3 a=0f b=aa c=03 d=c3 gn=0000 ps=1000 one=Pu1 wk=We0",
            "T|ua1 nd=c3 a=0f b=aa c=03 d=c3 gn=0000 ps=1000 one=Pu1 wk=We0",
            "T|m nd=c3 a=0f b=aa c=03 d=c3 gn=0000 ps=1000 one=Pu1 wk=We0",
            "T|gb1 nd=c3 a=0f b=aa c=03 d=c3 gn=0000 ps=1000 one=Pu1 wk=We0",
            "T|ua0 nd=ff a=0f b=aa c=03 d=ff gn=1111 ps=111z one=Pu1 wk=We0",
        ]
    );
}

/// §5.7.1, §28.12: an unsized `'0`, `'1`, `'x` or `'z` fills every bit of a
/// strength-resolved net, as a weak or pull driver and as the released value
/// of the strong driver beside a pull and a weak one, on a net wider than 64
/// bits too, at the top level and inside an instance.
#[test]
fn unsized_literals_fill_strength_resolved_nets() {
    let got = t_lines(
        r#"
module sub;
  logic en = 0;
  logic [7:0] r = 8'h01;
  wire [99:0] wide;
  assign wide = en ? {100{1'b0}} : 'z;
  assign (weak0, weak1) wide = '1;
  wire [7:0] f0, fz, fx, f1p, m;
  assign f0 = en ? 8'hc3 : 'z;  assign (weak0, weak1) f0 = '0;
  assign fz = en ? 8'hc3 : 'z;  assign (weak0, weak1) fz = 'z;
  assign fx = en ? 8'hc3 : 'z;  assign (weak0, weak1) fx = 'x;
  assign f1p = en ? 8'hzz : 8'h0f;  assign (pull0, pull1) f1p = '1;
  assign (pull0, pull1) m = r;
  assign (weak0, weak1) m = ~r;
  assign m = en ? {r[3:0], r[7:4]} : 'z;
  task show(string tag);
    $display("T|%s wide=%h f0=%h fz=%h fx=%h f1p=%h m=%h", tag, wide, f0, fz, fx, f1p, m);
  endtask
endmodule
module t;
  logic en = 0;
  logic [7:0] r = 8'h01;
  wire [99:0] wide;
  assign wide = en ? {100{1'b0}} : 'z;
  assign (weak0, weak1) wide = '1;
  wire [7:0] f0, fz, fx, f1p, m;
  assign f0 = en ? 8'hc3 : 'z;  assign (weak0, weak1) f0 = '0;
  assign fz = en ? 8'hc3 : 'z;  assign (weak0, weak1) fz = 'z;
  assign fx = en ? 8'hc3 : 'z;  assign (weak0, weak1) fx = 'x;
  assign f1p = en ? 8'hzz : 8'h0f;  assign (pull0, pull1) f1p = '1;
  assign (pull0, pull1) m = r;
  assign (weak0, weak1) m = ~r;
  assign m = en ? {r[3:0], r[7:4]} : 'z;
  sub u();
  task show(string tag);
    $display("T|%s wide=%h f0=%h fz=%h fx=%h f1p=%h m=%h", tag, wide, f0, fz, fx, f1p, m);
  endtask
  initial begin
    #1; show("top"); u.show("sub");
    r = 8'h12; u.r = 8'h12;
    #1; show("top"); u.show("sub");
    en = 1; u.en = 1;
    #1; show("top"); u.show("sub");
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|top wide=fffffffffffffffffffffffff f0=00 fz=zz fx=xx f1p=0f m=01",
            "T|sub wide=fffffffffffffffffffffffff f0=00 fz=zz fx=xx f1p=0f m=01",
            "T|top wide=fffffffffffffffffffffffff f0=00 fz=zz fx=xx f1p=0f m=12",
            "T|sub wide=fffffffffffffffffffffffff f0=00 fz=zz fx=xx f1p=0f m=12",
            "T|top wide=0000000000000000000000000 f0=c3 fz=c3 fx=c3 f1p=ff m=21",
            "T|sub wide=0000000000000000000000000 f0=c3 fz=c3 fx=c3 f1p=ff m=21",
        ]
    );
}

/// §6.5, §6.6.5, §28.12: bit and part-select drivers inside an instance all
/// drive their bits: overlapping part-selects, a whole-net driver beside
/// selects, wand, wor, tri0 and tri1 nets, output ports of several instances
/// on one bit, and a generate loop. A tri0 or tri1 net whose only drivers
/// are selects pulls the bits they release and the bits none drives.
#[test]
fn select_drivers_resolve_inside_instances() {
    let got = t_lines(
        r#"
module drv(output wire o, input logic v); assign o = v; endmodule
module sub;
  logic en = 0;
  wire [7:0] n;
  assign n[3:0] = 4'b1010;
  assign n[5:2] = en ? 4'b0110 : 4'bzzzz;
  assign n[7:6] = 2'b01;
  wire [3:0] m;
  assign m = 4'b0z1z;
  assign m[0] = 1'b1;
  assign m[2] = en ? 1'b0 : 1'bz;
  wand [1:0] wa; assign wa[0] = 1'b1; assign wa[0] = en ? 1'b0 : 1'b1; assign wa[1] = 1'b1; assign wa[1] = 1'bz;
  wor  [1:0] wo; assign wo[0] = 1'b0; assign wo[0] = en ? 1'b1 : 1'b0; assign wo[1] = 1'b0; assign wo[1] = 1'bz;
  tri1 [1:0] t1; assign t1[0] = 1'bz; assign t1[0] = en ? 1'b0 : 1'bz; assign t1[1] = 1'b0; assign t1[1] = 1'bz;
  tri0 [1:0] t0; assign t0[0] = 1'bz; assign t0[0] = en ? 1'b1 : 1'bz; assign t0[1] = 1'b1; assign t0[1] = 1'bz;
  tri1 [1:0] td; assign td[0] = en ? 1'b0 : 1'bz;
  tri0 [3:0] te; assign te[2:1] = en ? 2'b11 : 2'bzz;
  wire [1:0] bus;
  logic v0 = 1, v1 = 1'bz, v2 = 1'bz;
  drv d0(.o(bus[0]), .v(v0));
  drv d1(.o(bus[0]), .v(v1));
  drv d2[1:0](.o(bus[1]), .v(v2));
  wire [3:0] gn;
  for (genvar k = 0; k < 4; k++) begin : gl
    assign gn[k] = (k % 2) ? 1'b1 : 1'bz;
    assign gn[k] = en ? 1'b0 : 1'bz;
  end
  wire [1:0] dis;
  assign dis[0] = en; assign dis[1] = ~en;
  task show(string tag);
    $display("T|%s n=%b m=%b wa=%b wo=%b t1=%b t0=%b td=%b te=%b bus=%b gn=%b dis=%b", tag, n, m, wa, wo, t1, t0, td, te, bus, gn, dis);
  endtask
endmodule
module mid; sub s(); endmodule
module t;
  tri1 [1:0] td; tri0 [3:0] te; logic en = 0;
  assign td[0] = en ? 1'b0 : 1'bz;
  assign te[2:1] = en ? 2'b11 : 2'bzz;
  sub u(); mid m(); sub ua[1:0]();
  initial begin
    #1; u.show("u"); m.s.show("m"); ua[0].show("ua0"); $display("T|top td=%b te=%b", td, te);
    u.en = 1; m.s.en = 1; ua[0].en = 1; u.v1 = 0; u.v2 = 0; en = 1;
    #1; u.show("u"); m.s.show("m"); ua[0].show("ua0"); ua[1].show("ua1"); $display("T|top td=%b te=%b", td, te);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|u n=01zz1010 m=0z11 wa=11 wo=00 t1=01 t0=10 td=11 te=0000 bus=z1 gn=1z1z dis=10",
            "T|m n=01zz1010 m=0z11 wa=11 wo=00 t1=01 t0=10 td=11 te=0000 bus=z1 gn=1z1z dis=10",
            "T|ua0 n=01zz1010 m=0z11 wa=11 wo=00 t1=01 t0=10 td=11 te=0000 bus=z1 gn=1z1z dis=10",
            "T|top td=11 te=0000",
            "T|u n=01011010 m=0011 wa=10 wo=01 t1=00 t0=11 td=10 te=0110 bus=0x gn=x0x0 dis=01",
            "T|m n=01011010 m=0011 wa=10 wo=01 t1=00 t0=11 td=10 te=0110 bus=z1 gn=x0x0 dis=01",
            "T|ua0 n=01011010 m=0011 wa=10 wo=01 t1=00 t0=11 td=10 te=0110 bus=z1 gn=x0x0 dis=01",
            "T|ua1 n=01zz1010 m=0z11 wa=11 wo=00 t1=01 t0=10 td=11 te=0000 bus=z1 gn=1z1z dis=10",
            "T|top td=10 te=0110",
        ]
    );
}
