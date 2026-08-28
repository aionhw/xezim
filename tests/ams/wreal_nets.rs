//! Stage 2 of Verilog-AMS support: `wreal` real nets (AMS §3.8).
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

/// AMS §3.8: a plain `wreal` carries a real value. Without the implicit real
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

/// AMS §3.8 driver resolution, all four resolved forms over the same three
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

/// AMS §3.8: a plain `wreal` permits ONE driver. Two must be a clean error —
/// silently picking one would produce a plausible waveform from a design the
/// standard does not define.
#[test]
fn a_plain_wreal_rejects_multiple_drivers() {
    let err = with_ams(|| {
        simulate(
            r#"
module tb;
  wreal n;
  real a, b;
  assign n = a;
  assign n = b;
  initial begin a = 1.0; b = 2.0; #1; end
endmodule
"#,
            10,
        )
        .err()
        .expect("two drivers on a plain wreal must be rejected")
    });
    assert!(err.contains("wreal"), "{}", err);
    assert!(
        err.contains("wrealsum"),
        "the error must name the resolved forms: {}",
        err
    );
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

/// The gate itself. `wreal`, `wrealsum`, … are NOT IEEE 1800 keywords, and
/// reserving them unconditionally would reject designs that compile today.
/// With AMS off they must lex as ordinary identifiers.
#[test]
fn gate_is_off_by_default() {
    let sim = without_ams(|| {
        simulate(
            r#"
module tb;
  integer wreal, wrealsum, wrealavg, wrealmin, wrealmax;
  initial begin
    wreal = 1; wrealsum = 2; wrealavg = 3; wrealmin = 4; wrealmax = 5;
    #1;
  end
endmodule
"#,
            10,
        )
        .expect("AMS keywords must be plain identifiers with the gate off")
    });
    assert_eq!(u(&sim, "wreal"), Some(1));
    assert_eq!(u(&sim, "wrealmax"), Some(5));
}
