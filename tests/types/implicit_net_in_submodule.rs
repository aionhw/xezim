//! §6.10: an undeclared identifier used as a primitive port, a module-instance
//! port actual or a continuous-assign target creates an implicit 1-bit net, and
//! reading it later is legal. Inside a sub-module the elaboration registers that
//! net under its instance path (`u.gate_net`), so the §6.20.2/§8.25 finder must
//! match the bare identifier as a trailing path component.
//!
//! Without this, a legal vendor ROM model was rejected at elaboration:
//!
//! ```text
//! buf sBIST (bBIST, BIST);              // bBIST: implicit net, legal
//! wire iCEB = (bBIST) ? bCEBM : bCEB;   // ERROR: initializer of 'iCEB'
//!                                       // refers to undeclared identifier 'bBIST'
//! ```
//!
//! Icarus 13.0 compiles and runs the reduced case (prints `q=111`), Verilator
//! 5.054 lints it with only its `IMPLICIT` warning.

use xezim::simulate;

/// Implicit net from a gate primitive, from an instance port actual, and from a
/// continuous assign — all inside a sub-module, all read from initializers.
/// The sub-module is instantiated twice: once directly (`u.gate_net`) and once
/// through a generate block, which pushes the elaborated path one level deeper
/// (`g_top[0].u.gate_net`).
const DESIGN: &str = r#"
module leaf (input i, output o);
  assign o = i;
endmodule

module sub (input a, output [2:0] q);
  buf g (gate_net, a);
  leaf m (.i(a), .o(inst_net));
  assign assign_net = a;

  wire q_gate   = gate_net;
  wire q_inst   = inst_net;
  wire q_assign = assign_net;

  assign q = {q_gate, q_inst, q_assign};
endmodule

module tb;
  wire [2:0] q_direct;
  wire [2:0] q_gen;
  genvar gi;
  sub u (.a(1'b1), .q(q_direct));
  generate
    for (gi = 0; gi < 1; gi++) begin : g_top
      sub u (.a(1'b1), .q(q_gen));
    end
  endgenerate
  initial begin
    #1 $display("q=%b%b", q_direct, q_gen);
    $finish;
  end
endmodule
"#;

#[test]
fn submodule_implicit_nets_are_declared() {
    let run = simulate(DESIGN, 1_000).unwrap_or_else(|e| {
        panic!("legal §6.10 implicit nets must elaborate, got: {e}");
    });
    let msgs: Vec<String> = run
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert!(msgs.iter().any(|m| m == "q=111111"), "got {msgs:?}");
}

#[test]
fn a_name_declared_nowhere_is_still_rejected() {
    // A class property initializer is not a net context, so §6.10 does not
    // rescue the name: the finder must still report it.
    let bad = r#"
class C;
  int m = no_such_name;
endclass

module tb;
  initial $finish;
endmodule
"#;
    let err = match simulate(bad, 1_000) {
        Ok(_) => panic!("an undeclared name must still be reported"),
        Err(e) => e,
    };
    assert!(
        err.contains("undeclared identifier 'no_such_name'"),
        "unexpected error: {err}"
    );
}
