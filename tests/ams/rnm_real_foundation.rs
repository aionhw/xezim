//! Stage 1 of Verilog-AMS support: the real / real-number-modeling (RNM) /
//! user-defined-nettype foundation every later analog stage stands on.
//!
//! Nothing here is new behavior — it is the substrate `wreal` (Stage 2) lowers
//! onto and the analog stages extend. It had no group of its own, so a change
//! to real-value plumbing or to the §6.6.7 resolver dispatch could regress the
//! AMS substrate while every existing group stayed green. These tests pin it.
//!
//! Citations are IEEE 1800-2023 (`§`); Verilog-AMS 2.4.0 is cited as `AMS §`.

use crate::util::{r, u};
use xezim::simulate;

/// §6.12 `real` variables, and the `wire real` NET form that RNM designs use
/// for a continuously-driven analog quantity. Both must carry an f64, not the
/// raw bit image.
#[test]
fn real_variables_and_real_nets_carry_f64() {
    let src = r#"
module tb;
  real rv;
  wire real rn;
  assign rn = rv * 2.0;
  initial begin
    rv = 1.25;
    #1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert_eq!(r(&sim, "rv"), 1.25);
    assert_eq!(r(&sim, "rn"), 2.5, "`wire real` must resolve in the real domain");
}

/// §23.3.3 a `real` crossing a module port keeps full double precision — the
/// single most common RNM shape (a behavioral block feeding a real value down
/// the hierarchy).
#[test]
fn real_crosses_a_module_port_without_truncation() {
    let src = r#"
module leaf(input real din, output real dout);
  assign dout = din * 3.0;
endmodule
module tb;
  real a, c;
  leaf u(.din(a), .dout(c));
  initial begin
    a = 1.5;
    #1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert_eq!(r(&sim, "c"), 4.5);
}

/// §6.6.7 a user-defined nettype over `real` with a resolution function is
/// THE mechanism Stage 2's `wreal` resolution lowers onto: N drivers on one
/// net, folded by a user function. Two drivers, summed.
#[test]
fn user_defined_nettype_resolves_multiple_real_drivers() {
    let src = r#"
function automatic real Rsum(input real drv[]);
  real s = 0.0;
  foreach (drv[i]) s += drv[i];
  return s;
endfunction
nettype real rwire_t with Rsum;
module tb;
  rwire_t n;
  real a, b;
  assign n = a;
  assign n = b;
  initial begin
    a = 1.5; b = 2.25;
    #1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert_eq!(r(&sim, "n"), 3.75, "Rsum over both drivers");
}

/// The resolver is an arbitrary function, not just a sum — `wrealmax`-style
/// selection is what an analog "strongest driver wins" node needs, and it is
/// the shape Stage 2 reuses for `wrealmax`/`wrealmin`.
#[test]
fn a_selecting_resolver_picks_one_driver() {
    let src = r#"
function automatic real Rmax(input real d[]);
  real m = d[0];
  foreach (d[i]) if (d[i] > m) m = d[i];
  return m;
endfunction
nettype real rmax_t with Rmax;
module tb;
  rmax_t n;
  real a, b;
  assign n = a;
  assign n = b;
  initial begin
    a = 1.5; b = -2.75;
    #1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert_eq!(r(&sim, "n"), 1.5, "Rmax must select, not combine");
}

/// §20.8 real math and §20.5 conversions stay in the real domain. An RNM model
/// is mostly these calls, and a silent integer collapse in any of them turns a
/// plausible waveform into a wrong one.
#[test]
fn real_math_and_conversion_system_functions() {
    let src = r#"
module tb;
  real sq, pw, fl, it, bt;
  int  ri;
  initial begin
    sq = $sqrt(4.0);
    pw = $pow(2.0, 3.0);
    fl = $floor(2.7);
    ri = $rtoi(3.9);
    it = $itor(7);
    bt = $bitstoreal($realtobits(2.5));
    #1;
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    assert_eq!(r(&sim, "sq"), 2.0);
    assert_eq!(r(&sim, "pw"), 8.0);
    assert_eq!(r(&sim, "fl"), 2.0);
    assert_eq!(r(&sim, "it"), 7.0);
    assert_eq!(r(&sim, "bt"), 2.5, "$realtobits/$bitstoreal must round-trip");
    assert_eq!(u(&sim, "ri"), Some(3), "$rtoi truncates toward zero");
}
