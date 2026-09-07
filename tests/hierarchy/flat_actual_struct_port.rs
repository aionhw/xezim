//! §7.2.1/§23.3.3: a PACKED-STRUCT input port driven by a FLAT actual — a
//! plain vector net, a concat, a part-select, or a flat parent port — and
//! read member-wise inside the child (`din.f0`).
//!
//! An input port is normally substituted away by its actual. For a struct
//! formal that handed the body a name with NO member layout, so every member
//! read went x while the whole-port read stayed right — and the identical
//! conversion block worked whenever the parent net happened to be
//! struct-typed. Such a port now keeps its own signal (registered with the
//! formal's layout, driven by the connection assign) unless the actual is a
//! whole net that carries a layout itself.
//!
//! Surfaced as a struct-conversion hub whose flat-driven channels failed
//! every burst check while the struct-driven ones passed. Every expected
//! value is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn flat_actuals_on_struct_ports_read_members_correctly() {
    let o = out(r#"
package pk;
  typedef struct packed { logic [39:0] f0; logic [12:0] f1; logic f2; logic f3; logic f4; } s3_t;
endpackage
module child (input pk::s3_t din, output logic [39:0] o0, output logic [12:0] o1, output logic o2, o3, o4,
              output logic [55:0] whole, output logic [39:0] c0);
  assign o0 = din.f0; assign o1 = din.f1; assign o2 = din.f2; assign o3 = din.f3; assign o4 = din.f4;
  assign whole = din;
  always_comb c0 = din.f0;
endmodule
module grand (input pk::s3_t din, output logic [55:0] whole2, output logic [12:0] g1);
  child u (.din(din), .o0(), .o1(g1), .o2(), .o3(), .o4(), .whole(whole2), .c0());
endmodule
module thru (input logic [55:0] pin, output logic [12:0] t1);
  child u (.din(pin), .o0(), .o1(t1), .o2(), .o3(), .o4(), .whole(), .c0());
endmodule
module tb;
  logic [55:0] flat = {40'hDEADBEEF01, 13'h1234, 1'b1, 1'b0, 1'b1};
  logic [39:0] hi = 40'hDEADBEEF01; logic [15:0] lo16 = {13'h1234, 1'b1, 1'b0, 1'b1};
  pk::s3_t st = '{f0:40'hDEADBEEF01, f1:13'h1234, f2:1'b1, f3:1'b0, f4:1'b1};
  logic [39:0] fo0, so0, fc0; logic [12:0] fo1, so1, gg1, cc1, pp1, tt1;
  logic fo2, fo3, fo4, so2, so3, so4; logic [55:0] fw, sw, gw;
  child uf (.din(flat), .o0(fo0), .o1(fo1), .o2(fo2), .o3(fo3), .o4(fo4), .whole(fw), .c0(fc0));
  child us (.din(st),   .o0(so0), .o1(so1), .o2(so2), .o3(so3), .o4(so4), .whole(sw), .c0());
  grand ug (.din(flat), .whole2(gw), .g1(gg1));
  child uc (.din({hi, lo16}), .o0(), .o1(cc1), .o2(), .o3(), .o4(), .whole(), .c0());
  child up (.din(flat[55:0]), .o0(), .o1(pp1), .o2(), .o3(), .o4(), .whole(), .c0());
  thru  ut (.pin(flat), .t1(tt1));
  initial begin #1;
    $display("FLAT f0=%h f1=%h f2f3f4=%b%b%b whole=%h c0=%h", fo0, fo1, fo2, fo3, fo4, fw, fc0);
    $display("STRU f0=%h f1=%h f2f3f4=%b%b%b whole=%h", so0, so1, so2, so3, so4, sw);
    $display("GRND whole=%h f1=%h CAT=%h PS=%h THRU=%h", gw, gg1, cc1, pp1, tt1);
  end
endmodule
"#);
    for expect in [
        "FLAT f0=deadbeef01 f1=1234 f2f3f4=101 whole=deadbeef0191a5 c0=deadbeef01",
        "STRU f0=deadbeef01 f1=1234 f2f3f4=101 whole=deadbeef0191a5",
        "GRND whole=deadbeef0191a5 f1=1234 CAT=1234 PS=1234 THRU=1234",
    ] {
        assert!(o.contains(expect), "expected `{expect}` in:\n{o}");
    }
}
