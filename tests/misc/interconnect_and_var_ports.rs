//! §6.6.8 `interconnect` (issue #125 gaps 1–2) and §23.2.2.3 `var` port
//! continuation.
//!
//! * A bare `interconnect w;` used to be parse-accepted and DISCARDED — the
//!   name looked undeclared, §6.10 built a 1-bit implicit wire, and the
//!   diagnostic blamed a missing declaration. It now registers a real net:
//!   dimensions survive (`interconnect [3:0] a` is 4 bits) and a nettype
//!   port connection shapes the typeless net (width/realness adoption; full
//!   §6.6.7 cross-hierarchy resolution is tracked separately).
//! * `module m (interconnect p);` was a hard parse error (three cascading
//!   diagnostics) — the ANSI port-list opener now accepts the keyword.
//! * `output var a, b` — `b` continues the SAME port declaration and is a
//!   variable (§23.2.2.3), but `var` was not carried across the comma the way
//!   the data type is, so `b` registered as an untyped default net (found
//!   reviewing the lint in PR #134; procedural assigns to `b` misbehaved).

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn interconnect_declaration_registers_with_dims() {
    let out = msgs(
        r#"
module top;
  interconnect [3:0] a;
  interconnect b, c;
  initial $display("W_%0d_%0d_%0d", $bits(a), $bits(b), $bits(c));
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "W_4_1_1"),
        "interconnect declaration dropped or dims lost: {:?}",
        out
    );
}

#[test]
fn interconnect_port_form_parses() {
    let out = msgs(
        r#"
module m (interconnect p);
endmodule
module top;
  m u();
  initial $display("PARSED");
endmodule
"#,
    );
    assert!(out.iter().any(|m| m == "PARSED"), "port form: {:?}", out);
}

#[test]
fn interconnect_adopts_connected_port_type() {
    // The rnet formal is 64-bit real; the typeless interconnect net adopts
    // that shape at binding, so $bits reports 64 and no width-mismatch
    // truncation occurs. (Driver RESOLUTION across the hierarchy is §6.6.7
    // machinery tracked separately — asserted here is the adoption only.)
    let out = msgs(
        r#"
function automatic real rsum (input real d []);
  rsum = 0.0; foreach (d[i]) rsum += d[i];
endfunction
nettype real rnet with rsum;
module src (inout rnet p);
endmodule
module top;
  interconnect w;
  src u1 (.p(w));
  initial begin #1; $display("B_%0d", $bits(w)); end
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "B_64"),
        "interconnect did not adopt the nettype port's shape: {:?}",
        out
    );
}

#[test]
fn var_kw_continues_across_port_comma() {
    let out = msgs(
        r#"
module m (output var a, b);
  initial begin a = 1'b1; b = 1'b1; end
endmodule
module top;
  wire x, y;
  m u(.a(x), .b(y));
  initial begin #1; $display("VK_%b_%b", x, y); end
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "VK_1_1"),
        "`output var a, b` — b lost var-ness: {:?}",
        out
    );
}
