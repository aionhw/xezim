//! Two defects found while debugging a bidirectional DDR bus testbench.
//!
//! 1. §28.8 `tran` / `tranif0` / `tranif1` were modelled as a ONE-directional
//!    `assign terminal0 = terminal1`, so the second terminal was never driven,
//!    a disabled switch's `z` erased the net's own driver, and contention
//!    between two drivers resolved to whichever wrote last rather than to `x`.
//!    Each switch now bridges its terminals' OWN drivers with the wired-net
//!    resolution of Table 28-1.
//!
//! 2. §6.10 implicit-net creation descended into a `MemberAccess` base, so a
//!    cross-module reference (`testbench.chip.dqs`) had its ROOT — the instance
//!    name — declared as a stray 1-bit net under the current prefix. That net
//!    then drove the real one to X.

use xezim::simulate;

/// A tri-state driver on one net must appear on the other side of the `tran`,
/// and high-impedance must stay high-impedance.
const TRAN: &str = r#"
module tb;
  wire [3:0] a, b;
  logic en;
  logic [3:0] d;
  assign a = en ? d : 4'bzzzz;
  tran t (a, b);

  logic [3:0] b_off, a_on, b_on;
  logic off_is_z;
  initial begin
    en = 0; d = 4'h0;
    #1 b_off = b;
    off_is_z = (b === 4'bzzzz);
    en = 1; d = 4'hA;
    #1 a_on = a;
       b_on = b;
  end
endmodule
"#;

/// A hierarchical reference in a sub-module must not declare its root as a net.
const XMR: &str = r#"
module leaf ();
  wire [3:0] v;
  assign v = 4'hA;
endmodule
module probe ();
  wire [3:0] seen;
  assign seen = top.l_inst.v;   // cross-module READ
endmodule
module top;
  leaf  l_inst();
  probe p_inst();
  logic [3:0] observed;
  initial #1 observed = p_inst.seen;
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or(0)
        & 0xF
}

#[test]
fn tran_propagates_in_both_directions() {
    let sim = simulate(TRAN, 100).expect("simulate failed");
    // The driven side and the far side agree.
    assert_eq!(u(&sim, "a_on"), 0xA);
    assert_eq!(u(&sim, "b_on"), 0xA, "tran never drove its second terminal");
}

#[test]
fn tran_keeps_high_impedance_when_undriven() {
    let sim = simulate(TRAN, 100).expect("simulate failed");
    assert_eq!(u(&sim, "off_is_z"), 1, "an undriven tran net must stay z");
}

#[test]
fn a_cross_module_reference_does_not_declare_its_root_as_a_net() {
    let sim = simulate(XMR, 100).expect("simulate failed");
    assert_eq!(u(&sim, "observed"), 0xA, "the cross-module read was clobbered");
}

/// §28.8 compliance: the resolution table, the conditional switches, and the
/// unknown-control case.
const SWITCHES: &str = r#"
module tb;
  logic ctrl;
  wire net_a, net_b, net_c, net_d, net_e, net_f;
  logic val_a, val_b, val_c, val_d, val_e, val_f;

  assign net_a = val_a;
  assign net_b = val_b;
  assign net_c = val_c;
  assign net_d = val_d;
  assign net_e = val_e;
  assign net_f = val_f;

  tran    u_tran    (net_a, net_b);
  tranif1 u_tranif1 (net_c, net_d, ctrl);
  tranif0 u_tranif0 (net_e, net_f, ctrl);

  int fails;
  initial begin
    fails = 0;
    #1;
    // tran: z yields to a driven value, in both directions.
    val_a = 1'b1; val_b = 1'bz; #10;
    if (net_a !== 1'b1 || net_b !== 1'b1) fails++;
    val_a = 1'bz; val_b = 1'b0; #10;
    if (net_a !== 1'b0 || net_b !== 1'b0) fails++;
    // tran: contention gives x on both nets.
    val_a = 1'b1; val_b = 1'b0; #10;
    if (net_a !== 1'bx || net_b !== 1'bx) fails++;

    // tranif1 disabled: each net keeps its own driver.
    ctrl = 1'b0; val_c = 1'b1; val_d = 1'b0; #10;
    if (net_c !== 1'b1 || net_d !== 1'b0) fails++;
    // enabled: contention -> x
    ctrl = 1'b1; #10;
    if (net_c !== 1'bx || net_d !== 1'bx) fails++;
    // enabled: z passes through
    val_c = 1'bz; val_d = 1'b1; #10;
    if (net_c !== 1'b1 || net_d !== 1'b1) fails++;

    // tranif0 has the opposite polarity.
    ctrl = 1'b1; val_e = 1'b0; val_f = 1'b1; #10;
    if (net_e !== 1'b0 || net_f !== 1'b1) fails++;
    ctrl = 1'b0; #10;
    if (net_e !== 1'bx || net_f !== 1'bx) fails++;

    // An unknown control makes differing bits unknown.
    ctrl = 1'bx; val_c = 1'b1; val_d = 1'b0; #10;
    if (net_c !== 1'bx || net_d !== 1'bx) fails++;
  end
endmodule
"#;

#[test]
fn bidirectional_switches_follow_the_resolution_table() {
    let sim = simulate(SWITCHES, 500).expect("simulate failed");
    let fails = sim
        .get_signal("fails")
        .or_else(|| sim.get_signal("tb.fails"))
        .expect("fails")
        .to_u64()
        .unwrap_or(99);
    assert_eq!(fails, 0, "{} of the 9 IEEE 1800-2017 §28.8 checks failed", fails);
}

/// §6.6.1: a net with several continuous drivers resolves ALL of them, rather
/// than taking whichever assign ran last.
const MULTI_DRIVER: &str = r#"
module tb;
  wire [1:0] w;
  logic en_a, en_b;
  assign w = en_a ? 2'b01 : 2'bzz;
  assign w = en_b ? 2'b10 : 2'bzz;

  logic none_z, only_a, only_b, contention_x;
  initial begin
    en_a = 0; en_b = 0; #1 none_z      = (w === 2'bzz);
    en_a = 1; en_b = 0; #1 only_a      = (w === 2'b01);
    en_a = 0; en_b = 1; #1 only_b      = (w === 2'b10);
    en_a = 1; en_b = 1; #1 contention_x = (w === 2'bxx);
  end
endmodule
"#;

#[test]
fn a_net_resolves_all_of_its_continuous_drivers() {
    let sim = simulate(MULTI_DRIVER, 100).expect("simulate failed");
    assert_eq!(u(&sim, "none_z"), 1, "both drivers z -> z");
    assert_eq!(u(&sim, "only_a"), 1, "one driver, one z -> the driven value");
    assert_eq!(u(&sim, "only_b"), 1);
    assert_eq!(u(&sim, "contention_x"), 1, "two conflicting drivers must give x");
}

/// A `tran` declared in a SUB-MODULE, bridging two nets referenced through the
/// top module. Gates in an inlined module are lowered before the switch pass
/// ran, so the switch was dropped; and a hierarchical lvalue had its root
/// prefixed with the instance path, so the assign wrote a name that resolved to
/// nothing.
const SUBMODULE_TRAN: &str = r#"
module leaf ();
  wire [3:0] p;
  logic en;
  logic [3:0] d;
  assign p = en ? d : 4'bzzzz;
endmodule

module bridge ();
  tran t (top.l0.p, top.l1.p);
endmodule

module wr_lhs ();
  assign top.probe = top.l0.p;   // cross-module lvalue in a sub-module
endmodule

module top;
  wire [3:0] probe;
  leaf l0(); leaf l1();
  bridge b(); wr_lhs w();

  logic [3:0] seen_l1, seen_probe, seen_back;
  initial begin
    l0.en = 1; l0.d = 4'hA;
    l1.en = 0; l1.d = 4'h0;
    #1 seen_l1 = l1.p;        // driven across the bridge
       seen_probe = probe;    // cross-module lvalue
    l0.en = 0;
    l1.en = 1; l1.d = 4'h5;
    #1 seen_back = l0.p;      // and back the other way
  end
endmodule
"#;

#[test]
fn a_tran_in_a_submodule_bridges_hierarchical_terminals() {
    let sim = simulate(SUBMODULE_TRAN, 100).expect("simulate failed");
    assert_eq!(u(&sim, "seen_l1"), 0xA, "a tran inside a sub-module was dropped");
    assert_eq!(u(&sim, "seen_back"), 0x5, "the bridge must work in both directions");
}

#[test]
fn a_continuous_assign_to_a_cross_module_lvalue_reaches_the_net() {
    let sim = simulate(SUBMODULE_TRAN, 100).expect("simulate failed");
    assert_eq!(u(&sim, "seen_probe"), 0xA, "the cross-module lvalue wrote a stray signal");
}
