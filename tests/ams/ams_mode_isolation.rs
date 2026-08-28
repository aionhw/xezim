//! Verilog-AMS regression: turning the AMS gate ON must not change how
//! ordinary SystemVerilog behaves.
//!
//! The gate exists so AMS keywords stay unreserved by default. The converse
//! matters just as much and is easier to break: an AMS-mode run of a design
//! with no AMS in it has to produce exactly what the default run produces.
//! A stray keyword arm or a changed default net type would show up here and
//! nowhere else in this group.

use crate::ams_mode::{with_ams, without_ams};
use crate::util::{r, u};
use xezim::simulate;

const PLAIN_SV: &str = r#"
package p;
  typedef enum logic [1:0] { A, B, C } e_t;
endpackage
module sub #(parameter int W = 8) (input logic clk, input logic [W-1:0] d, output logic [W-1:0] q);
  always_ff @(posedge clk) q <= d;
endmodule
module tb;
  import p::*;
  logic clk = 0;
  logic [7:0] d, q;
  wire  [7:0] w = d ^ 8'hA5;
  real  rv;
  e_t   e;
  int   count;
  sub #(.W(8)) u(.clk(clk), .d(w), .q(q));
  always #5 clk = ~clk;
  initial begin
    d = 8'h0F; e = B; rv = 2.5;
    repeat (4) @(posedge clk) count++;
    #1;
  end
endmodule
"#;

/// The same non-AMS design, run with the gate off and on. Every observable
/// must match.
#[test]
fn ams_mode_does_not_disturb_plain_systemverilog() {
    let off = without_ams(|| simulate(PLAIN_SV, 1000).expect("simulate with AMS off"));
    let on = with_ams(|| simulate(PLAIN_SV, 1000).expect("simulate with AMS on"));

    for name in ["d", "q", "w", "e", "count"] {
        assert_eq!(
            u(&off, name),
            u(&on, name),
            "{} differs between AMS off and AMS on",
            name
        );
    }
    assert_eq!(r(&off, "rv"), r(&on, "rv"));
    // The flop actually ran — otherwise every signal is x and the comparison
    // above is vacuously true.
    assert_eq!(u(&off, "count"), Some(4), "the design must have run");
    assert_eq!(u(&off, "q"), u(&off, "w"), "sub must have latched w");
}

/// §6.6.7 user-defined nettypes are plain SystemVerilog and must keep working
/// with AMS on — `wreal` lowers onto that same path, so a mistake in the
/// lowering would most likely surface as the user path breaking.
#[test]
fn user_defined_nettypes_still_work_with_ams_on() {
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
  initial begin a = 1.5; b = 2.25; #1; end
endmodule
"#;
    let on = with_ams(|| simulate(src, 10).expect("UDN must still elaborate with AMS on"));
    assert_eq!(r(&on, "n"), 3.75);
}
