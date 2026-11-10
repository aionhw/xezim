//! #973 review: §6.11, §7.2.1: a bit or part select of an integral-atom
//! property (`int i; i[31]`, `byte sb; sb[7]`) and of a packed struct or
//! union property, through a typedef or written in place, addresses the
//! property's bits, from a method (`this.`) and through a handle. A local
//! array of the same name (fixed, dynamic or a queue) shadows the property
//! (§8.10). Expected values come from the reference simulator.

use xezim::simulate;

#[test]
fn atom_and_packed_struct_property_selects_write_their_bits() {
    let src = r#"
package pkg;
  typedef struct packed { bit [3:0] hi; bit [3:0] lo; } ps_t;
  typedef union packed { bit [7:0] b; bit [1:0][3:0] n; } pu_t;
endpackage
import pkg::*;
class holder;
  ps_t s;
  pu_t u;
  int i;
  byte sb;
  shortint si;
  struct packed { bit [1:0] a; bit [5:0] b; } anon;
  function void via_this();
    this.s = 0; this.s[6] = 1;
    this.i = 0; this.i[31] = 1;
    this.u = 0; this.u[0] = 1;
    this.anon = 0; this.anon[7] = 1;
  endfunction
endclass
module top;
  holder h;
  initial begin
    h = new; h.via_this();
    $display("this: s=%h i=%h u=%h anon=%h", h.s, h.i, h.u, h.anon);
    h.s = 0; h.s[1] = 1; h.sb = 0; h.sb[7] = 1; h.si = 0; h.si[15:8] = 8'h81;
    h.i = 32'hA5; h.i[3:0] = 4'h0;
    $display("handle: s=%h sb=%0d si=%h i=%h s7=%b i7=%b", h.s, h.sb, h.si, h.i, h.s[1], h.i[7]);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "this: s=40 i=80000000 u=01 anon=80",
            "handle: s=02 sb=-128 si=8100 i=000000a0 s7=1 i7=1",
        ]
    );
}

#[test]
fn a_local_array_shadows_an_atom_property_of_the_same_name() {
    let src = r#"
class C;
  int d;
  int q;
  function void dyna();
    int d[];
    d = new[2];
    d[0] = 11; d[1] = 22;
    $display("local d0=%0d d1=%0d", d[0], d[1]);
  endfunction
  function void fixed();
    int d[2];
    int q[$];
    d[0] = 33; d[1] = 44;
    q.push_back(5); q[0] = 6;
    $display("local fixed d0=%0d d1=%0d q0=%0d", d[0], d[1], q[0]);
  endfunction
endclass
module top;
  C c;
  initial begin
    c = new; c.d = 0; c.q = 0;
    c.dyna();
    c.fixed();
    $display("prop d=%0d q=%0d", c.d, c.q);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "local d0=11 d1=22",
            "local fixed d0=33 d1=44 q0=6",
            "prop d=0 q=0",
        ]
    );
}
