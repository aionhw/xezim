//! Stage 2 of Verilog-AMS support: `wreal` real nets (AMS §3.7).
//!
//! A `wreal` is the discrete real-number-modeling net — it carries a real
//! value rather than a bit vector, and its multiple drivers fold through a
//! built-in resolution. xezim lowers it onto the §6.6.7 user-defined-nettype
//! path rather than a parallel mechanism: the net is tagged with a synthetic
//! nettype whose resolver is a reserved marker, and the resolution expands to
//! an expression at elaboration. That reuses the machinery that already
//! unions nets joined across module ports, so a node driven from several
//! instances resolves exactly ONCE (resolving per port net and again on the
//! results is right for a sum and wrong for a min/max).
//!
//! The keywords are reserved only under `--ams`; see `gate_is_off_by_default`.

use crate::ams_mode::{with_ams, without_ams};
use crate::util::{r, u};
use xezim::simulate;

/// AMS §3.7: a plain `wreal` carries a real value. Without the implicit real
/// data type the net elaborates as a 1-bit implicit wire and every real driven
/// onto it truncates to its LSB.
#[test]
fn a_wreal_net_carries_a_real_value() {
    let sim = with_ams(|| {
        simulate(
            r#"
module tb;
  wreal n;
  real a;
  assign n = a;
  initial begin a = 1.25; #1; end
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "n"), 1.25);
}

/// AMS §3.7 driver resolution, all four resolved forms over the same three
/// drivers. `avg` divides by a REAL count — an integer division there would
/// silently floor the average.
#[test]
fn resolved_wreal_forms_fold_their_drivers() {
    let sim = with_ams(|| {
        simulate(
            r#"
module tb;
  wrealsum nsum;
  wrealavg navg;
  wrealmin nmin;
  wrealmax nmax;
  real a, b, c;
  assign nsum = a; assign nsum = b; assign nsum = c;
  assign navg = a; assign navg = b; assign navg = c;
  assign nmin = a; assign nmin = b; assign nmin = c;
  assign nmax = a; assign nmax = b; assign nmax = c;
  initial begin a = 1.0; b = 2.0; c = 6.0; #1; end
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "nsum"), 9.0);
    assert_eq!(r(&sim, "navg"), 3.0, "avg must divide by a real count");
    assert_eq!(r(&sim, "nmin"), 1.0);
    assert_eq!(r(&sim, "nmax"), 6.0);
}

/// A non-integral average: `(1.0 + 2.0) / 2.0` is 1.5, not 1.
#[test]
fn wrealavg_keeps_the_fractional_part() {
    let sim = with_ams(|| {
        simulate(
            r#"
module tb;
  wrealavg n;
  real a, b;
  assign n = a;
  assign n = b;
  initial begin a = 1.0; b = 2.0; #1; end
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "n"), 1.5);
}

/// §3.7 defines `wreal` only for a net "driven by a single driver" and says
/// nothing about more, so the multi-driver fold is tool-defined. xezim sums,
/// which is what makes a current-summing wrapper mean what it says — several
/// stages each drive a contribution onto a shared node and the node sees the
/// total. Summing is also an identity on ONE driver, so §3.7's defined case is
/// unaffected; `a_wreal_net_carries_a_real_value` pins that half.
///
/// (This branch briefly made two drivers an ERROR and required an explicit
/// `wrealsum`. Trunk's sum-by-default won on merge — see
/// `tests/sv_compliance/tests_advanced/50_wreal_nets.sv`.)
#[test]
fn a_plain_wreal_sums_its_drivers() {
    let sim = with_ams(|| {
        simulate(
            r#"
module tb;
  wreal n;
  real a, b;
  assign n = a;
  assign n = b;
  initial begin a = 1.0; b = 2.5; #1; end
endmodule
"#,
            10,
        )
        .expect("two drivers on a plain wreal must resolve, not fail")
    });
    assert_eq!(r(&sim, "n"), 3.5);
}

/// A `wreal` crossing an ANSI module port, driven from two INSTANCES. This is
/// the shape the union-find in `resolve_user_nettype_drivers` exists for: the
/// port nets are joined by identity assigns, and the node must be resolved
/// once over both instance drivers (2.5 + 2.5), not twice.
#[test]
fn wreal_resolves_across_ansi_module_ports() {
    let sim = with_ams(|| {
        simulate(
            r#"
module drv(output wreal o);
  real v;
  assign o = v;
  initial v = 2.5;
endmodule
module tb;
  wrealsum node;
  drv u1(.o(node));
  drv u2(.o(node));
  initial #1;
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "node"), 5.0);
}

/// The same through a NON-ANSI port declaration (`output wreal o;` in the
/// body), which takes a different parse path.
#[test]
fn wreal_resolves_across_non_ansi_module_ports() {
    let sim = with_ams(|| {
        simulate(
            r#"
module drv(o);
  output wreal o;
  real v;
  assign o = v;
  initial v = 1.25;
endmodule
module tb;
  wrealsum node;
  drv u1(.o(node));
  drv u2(.o(node));
  initial #1;
endmodule
"#,
            10,
        )
        .expect("simulate")
    });
    assert_eq!(r(&sim, "node"), 2.5);
}

/// The gate. `wreal` itself is NOT gated — trunk reserves it in the main
/// keyword table like `uwire`, and its compliance tests use it in plain `.sv`
/// files. The VENDOR spellings are: `wrealsum`/`wrealavg`/`wrealmin`/
/// `wrealmax` appear in neither Verilog-AMS 2.4.0 nor VAMS-2023 (§3.7 admits
/// `wreal` alone), so reserving them unconditionally would take four
/// plausible identifiers away from designs that compile today.
#[test]
fn only_the_vendor_spellings_are_gated() {
    let sim = without_ams(|| {
        simulate(
            r#"
module tb;
  integer wrealsum, wrealavg, wrealmin, wrealmax;
  initial begin
    wrealsum = 2; wrealavg = 3; wrealmin = 4; wrealmax = 5;
    #1;
  end
endmodule
"#,
            10,
        )
        .expect("the vendor spellings must be plain identifiers with the gate off")
    });
    assert_eq!(u(&sim, "wrealsum"), Some(2));
    assert_eq!(u(&sim, "wrealmax"), Some(5));
}

/// …and `wreal` works with no flag at all, which is what trunk's compliance
/// tests rely on.
#[test]
fn wreal_needs_no_flag() {
    let sim = without_ams(|| {
        simulate(
            r#"
module tb;
  wreal n;
  real a;
  assign n = a;
  initial begin a = 1.25; #1; end
endmodule
"#,
            10,
        )
        .expect("wreal is an ordinary reserved net type")
    });
    assert_eq!(r(&sim, "n"), 1.25);
}
