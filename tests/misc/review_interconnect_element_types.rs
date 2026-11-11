//! The elements of an `interconnect` array take the types of the ports
//! bound to them (IEEE 1800-2023 §6.6.7, §6.6.8, §23.3.3.7.1), found
//! reviewing #1039 and #1040. Every expected line is the reference
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

/// §6.6.8: the standard's `iBus[0:1]` example with a logic and a real
/// element, an array of three real elements, a struct nettype through an
/// interconnect port, a resolved struct nettype with two drivers, and an
/// interconnect array whose elements are struct nettype nets.
#[test]
fn interconnect_elements_take_their_port_types() {
    let got = t_lines(
        r#"
`timescale 1ns/1ps
package NetsPkg;
  nettype real realNet;
  typedef struct { real V; real I; } vi_t;
  function automatic vi_t res(input vi_t d[]);
    res.V = 0; res.I = 0;
    foreach (d[i]) begin res.V += d[i].V; res.I += d[i].I; end
  endfunction
  nettype vi_t vinet;
  nettype vi_t vires with res;
endpackage
module lDriver(output wire logic out); assign out = 1'b1; endmodule
module lMod(input wire logic i); endmodule
module rDriver import NetsPkg::*; #(real V = 2.5) (output realNet out); assign out = V; endmodule
module rMod import NetsPkg::*; (input realNet i); endmodule
module sDrv import NetsPkg::*; #(real V = 1.5) (output vinet o); assign o = '{V, 0.125}; endmodule
module sRd import NetsPkg::*; (input vinet i); endmodule
module rDrv import NetsPkg::*; #(real V = 1.0) (output vires o); assign o = '{V, 0.5}; endmodule
module rRd import NetsPkg::*; (input vires i); endmodule
module rlMod(input interconnect iBus[0:1]);
  lMod l1(iBus[0]);
  rMod r1(iBus[1]);
endmodule
module arrMod(input interconnect b[0:2]);
  rMod r0(b[0]); rMod r1(b[1]); rMod r2(b[2]);
endmodule
module passMod(input interconnect x); sRd r(.i(x)); endmodule
module t;
  interconnect iBus[0:1];
  lDriver l1(iBus[0]);
  rDriver r1(iBus[1]);
  rlMod m1(iBus);
  interconnect b[0:2];
  rDriver #(1.25) d0(b[0]);
  rDriver #(3.75) d1(b[1]);
  rDriver #(-0.5) d2(b[2]);
  arrMod m(b);
  interconnect w2;
  sDrv #(7.5) d4(.o(w2));
  passMod pm(.x(w2));
  interconnect w3;
  rDrv #(1.0) e1(.o(w3)); rDrv #(2.0) e2(.o(w3)); rRd e3(.i(w3));
  interconnect sa[0:1];
  sDrv #(4.5) f0(.o(sa[0])); sDrv #(5.5) f1(.o(sa[1]));
  sRd g0(.i(sa[0])); sRd g1(.i(sa[1]));
  initial begin
    #1;
    $display("T|mixed l=%b r=%g", m1.l1.i, m1.r1.i);
    $display("T|reals %g %g %g", m.r0.i, m.r1.i, m.r2.i);
    $display("T|pass %g/%g", pm.r.i.V, pm.r.i.I);
    $display("T|resolved %g/%g", e3.i.V, e3.i.I);
    $display("T|structs %g/%g %g/%g", g0.i.V, g0.i.I, g1.i.V, g1.i.I);
    $finish;
  end
endmodule
"#,
    );
    assert_eq!(
        got,
        [
            "T|mixed l=1 r=2.5",
            "T|reals 1.25 3.75 -0.5",
            "T|pass 7.5/0.125",
            "T|resolved 3/1",
            "T|structs 4.5/0.125 5.5/0.125",
        ]
    );
}
