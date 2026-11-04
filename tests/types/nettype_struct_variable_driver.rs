//! §6.6.7 + §10.3: a user-defined nettype net driven from a struct VARIABLE.
//!
//! A nettype whose data type is an unpacked struct is stored as one signal
//! per member, so a whole-struct value has to be expanded member-wise. The
//! WRITE side did that already. The driver side did not: drivers were packed
//! into the resolution-function call as written.
//!
//! A driver that is already an assignment pattern survives, because it is
//! member-wise as authored:
//!
//!     assign n = '{i: 3.0e-3, v: 0.0};        // resolved correctly
//!
//! A driver naming a struct variable has no whole-struct value to read, so
//! the resolver received uninitialised members -- nan, or zero once a later
//! pass defaulted them:
//!
//!     nd_t t;  always_comb t = ...;
//!     assign n = t;                           // members arrived empty
//!
//! Both are legal. §10.3 gives `net_assignment ::= net_lvalue = expression`
//! and an expression may name a variable; the only restriction the clause
//! puts on a user-defined nettype is on the LEFT-hand side ("shall not
//! contain any indexing or select"). So the driver is expanded member-wise,
//! not rejected.
//!
//! WHY IT MATTERS beyond the clause. A SPICE node is bidirectional: current
//! is injected into it and a voltage is present on it, both properties of
//! one net. Carrying that needs an aggregate nettype -- IEEE 1800-2017
//! 6.6.7(d), and the LRM's own example in that clause is exactly a
//! `{real; bit;}` struct. Without it, a real-number model must split every
//! analog node into two ports, and cannot be instantiated against the
//! netlist's own hierarchy.
//!
//! The values are asserted, not just the fact that resolution ran. The
//! failing mode produced a resolver that WAS called, with the right driver
//! count, and members that were quietly empty -- a test that checked only
//! "did it resolve" would have passed against the bug.

use xezim::simulate;

fn notes(src: &str) -> Vec<String> {
    let sim = simulate(src, 10_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("NOTE:"))
        .collect()
}

const PKG: &str = r#"
package p;
  typedef struct { real i; real v; bit drives_v; } nd_t;
  function automatic nd_t res(input nd_t drivers[]);
    nd_t r;
    r.i = 0.0;
    r.v = 0.0;
    r.drives_v = 1'b0;
    foreach (drivers[k]) begin
      r.i += drivers[k].i;
      if (drivers[k].drives_v) begin
        r.v = drivers[k].v;
        r.drives_v = 1'b1;
      end
    end
    return r;
  endfunction
  nettype nd_t nd with res;
endpackage
"#;

/// Two drivers, each staged through a struct VARIABLE. Currents must sum and
/// the voltage must come from the one driver claiming to own it.
#[test]
fn struct_nettype_driven_from_a_variable_resolves() {
    let src = format!(
        "{PKG}
module drv import p::*; #(parameter real I = 0.0, parameter real V = 0.0,
                          parameter bit OWNS = 1'b0) (inout nd o);
  nd_t t;
  always_comb begin
    t.i = I;
    t.v = V;
    t.drives_v = OWNS;
  end
  assign o = t;
endmodule

module top;
  p::nd n;
  drv #(.I(3.0e-3), .V(0.0),    .OWNS(1'b0)) a (n);
  drv #(.I(-1.0e-3), .V(1.8086), .OWNS(1'b1)) b (n);
  initial begin
    #1;
    $display(\"NOTE: i=%0.4f v=%0.4f owner=%0b\", n.i * 1e3, n.v, n.drives_v);
    $finish;
  end
endmodule
"
    );
    // 3.0 mA and -1.0 mA sum to 2.0 mA; the voltage is b's, not a's zero,
    // and not an average of the two.
    assert_eq!(notes(&src), vec!["NOTE: i=2.0000 v=1.8086 owner=1"]);
}

/// The assignment-pattern form was already correct and must stay so -- it is
/// the form that worked, and the fix must not disturb it.
#[test]
fn struct_nettype_driven_from_a_pattern_still_resolves() {
    let src = format!(
        "{PKG}
module drv import p::*; #(parameter real I = 0.0, parameter real V = 0.0,
                          parameter bit OWNS = 1'b0) (inout nd o);
  assign o = '{{i: I, v: V, drives_v: OWNS}};
endmodule

module top;
  p::nd n;
  drv #(.I(3.0e-3), .V(0.0),    .OWNS(1'b0)) a (n);
  drv #(.I(-1.0e-3), .V(1.8086), .OWNS(1'b1)) b (n);
  initial begin
    #1;
    $display(\"NOTE: i=%0.4f v=%0.4f owner=%0b\", n.i * 1e3, n.v, n.drives_v);
    $finish;
  end
endmodule
"
    );
    assert_eq!(notes(&src), vec!["NOTE: i=2.0000 v=1.8086 owner=1"]);
}

/// The shape this exists for: one element OWNS the node voltage while
/// another reads that voltage and drives a current back onto the same net.
/// That is a SPICE node, and it is what a single port per node requires.
#[test]
fn a_node_can_be_read_and_driven_through_one_port() {
    let src = format!(
        "{PKG}
module pump import p::*; #(parameter real G = 2.0e-2) (inout nd dra);
  nd_t t;
  always_comb begin
    t.i = G * (3.3 - dra.v);
    t.v = 0.0;
    t.drives_v = 1'b0;
  end
  assign dra = t;
endmodule

module filt import p::*; #(parameter real C = 1.0e-9) (inout nd dra);
  real state = 0.0;
  always #1 state = state + (dra.i * 1.0e-9 / C);
  assign dra = '{{i: 0.0, v: state, drives_v: 1'b1}};
endmodule

module top;
  p::nd dra;
  pump u_p (dra);
  filt u_f (dra);
  initial begin
    #600;
    $display(\"NOTE: settled v=%0.2f i_small=%0b\", dra.v, dra.i < 1.0e-3);
    $finish;
  end
endmodule
"
    );
    // The pump's current falls as the node it drives against rises, so the
    // node charges to the supply and the current goes to nothing: it moves
    // G * (3.3 - v) per step, a time constant of 50 steps, so it has settled
    // by #600. Staged through a variable on purpose: that is the form that
    // used to fail.
    assert_eq!(notes(&src), vec!["NOTE: settled v=3.30 i_small=1"]);
}
