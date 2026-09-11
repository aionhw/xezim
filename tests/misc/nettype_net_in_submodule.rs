//! IEEE 1800-2017 §6.6.7(d), §7.2, §6.10: a user-defined nettype whose data
//! type is an unpacked struct, declared as a net inside a SUB-MODULE.
//!
//! An unpacked struct has no signal under its own name — elaboration registers
//! the member LEAVES (`base.i`, `base.v`, ...) and nothing at `base` — which is
//! also how a nettype net of that type is stored. The post-inlining implicit-net
//! pass looked for a signal at the base name, did not find one, and laid a 1-bit
//! implicit net over it (§6.10). That scalar then absorbed the continuous
//! assign: the member leaves were never written, and every read of the net came
//! back 0.0.
//!
//! It was silent and position-dependent. The same declaration in the TOP module
//! worked, because the top-level path never reaches that pass — so moving a
//! working module one level down, or merely adding a second uninstantiated
//! module (which changes which module auto-detection picks as the root), turned
//! a correct design into one that elaborated cleanly and simulated wrong.
//!
//! Reported from an analog/mixed-signal partition where the supply node of a
//! generated netlist top read 0 V instead of 3.3 V.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 4_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

/// The node package both tests use: a resolved nettype carrying a current that
/// sums across drivers and a voltage that exactly one driver establishes — the
/// §6.6.7(d) shape, an unpacked struct as a nettype data type.
const PKG: &str = r#"
package p;
  typedef struct { real i; real v; bit drives_v; } nd_t;
  function automatic nd_t res(input nd_t drivers[]);
    nd_t r;
    r.i = 0.0; r.v = 0.0; r.drives_v = 1'b0;
    foreach (drivers[k]) begin
      r.i += drivers[k].i;
      if (drivers[k].drives_v) begin r.v = drivers[k].v; r.drives_v = 1'b1; end
    end
    return r;
  endfunction
  nettype nd_t nd with res;
  function automatic nd_t dv(input real volts);
    dv = '{i: 0.0, v: volts, drives_v: 1'b1};
  endfunction
endpackage
"#;

#[test]
fn nettype_net_declared_in_submodule_keeps_its_declaration() {
    // `dd` is declared and continuously assigned one level below the root.
    // Before the fix this printed 0.0000 and emitted a §6.10 implicit-net
    // warning for `u_mid.dd`.
    let src = format!(
        "{PKG}
module mid import p::*; ();
  nd dd;
  assign dd = dv(3.3);
  initial begin #2; $display(\"SUB=%0.4f\", dd.v); end
endmodule

module tb;
  mid u_mid ();
  initial begin #4; $finish; end
endmodule
"
    );
    let out = msgs(&src);
    assert!(
        out.iter().any(|m| m.contains("SUB=3.3000")),
        "nettype net declared in a sub-module lost its value: {out:?}"
    );
    // The declaration is present, so no implicit net may be synthesized for it.
    assert!(
        !out.iter().any(|m| m.contains("implicit 1-bit net") && m.contains("dd")),
        "an implicit net was laid over the declared nettype net: {out:?}"
    );
}

#[test]
fn top_module_choice_does_not_change_the_result() {
    // The same module text, once as the sole root and once demoted to a
    // sub-module by the presence of a second, uninstantiated module. Root
    // auto-detection must not change a net's value.
    let body = "
  nd dd;
  assign dd = dv(3.3);
  initial begin #2; $display(\"V=%0.4f\", dd.v); end";

    let as_root = format!(
        "{PKG}
module tb import p::*; ();
{body}
  initial begin #4; $finish; end
endmodule
"
    );
    let demoted = format!(
        "{PKG}
module other ();
  initial begin #1; end
endmodule

module tb import p::*; ();
{body}
  initial begin #4; $finish; end
endmodule
"
    );

    let a = msgs(&as_root);
    let b = msgs(&demoted);
    let pick = |v: &Vec<String>| {
        v.iter()
            .find(|m| m.starts_with("V="))
            .cloned()
            .unwrap_or_else(|| format!("<no V= line> {v:?}"))
    };
    assert_eq!(
        pick(&a),
        pick(&b),
        "adding an uninstantiated module changed the nettype net's value"
    );
    assert!(pick(&a).contains("3.3000"), "expected 3.3000, got {}", pick(&a));
}

#[test]
fn submodule_nettype_net_still_resolves_multiple_drivers() {
    // The declaration surviving is not enough on its own: the net must still go
    // through its resolution function. Two drivers inside the sub-module — one
    // current, one voltage — must sum the current and take the single voltage,
    // not overwrite each other.
    let src = format!(
        "{PKG}
module mid import p::*; ();
  nd n;
  assign n = '{{i: 1.5, v: 0.0, drives_v: 1'b0}};
  assign n = '{{i: 2.5, v: 0.0, drives_v: 1'b0}};
  assign n = dv(1.25);
  initial begin #2; $display(\"I=%0.4f V=%0.4f\", n.i, n.v); end
endmodule

module tb;
  mid u_mid ();
  initial begin #4; $finish; end
endmodule
"
    );
    let out = msgs(&src);
    assert!(
        out.iter().any(|m| m.contains("I=4.0000")),
        "currents did not sum through the resolver in a sub-module: {out:?}"
    );
    assert!(
        out.iter().any(|m| m.contains("V=1.2500")),
        "the voltage owner did not establish the node voltage: {out:?}"
    );
}
