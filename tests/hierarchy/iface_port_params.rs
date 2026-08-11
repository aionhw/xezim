//! J2c/J2e const-eval closures — reference-validated.
//!
//! J2e (§25.5): a parameter of a BOUND interface instance read through the
//! child's interface-port formal in a CONSTANT context (`localparam D =
//! c.DEPTH;`, `logic [c.DEPTH-1:0]`) silently evaluated 0 — the classic
//! silent-zero shape. The child's param map is now seeded with
//! `<formal>.<param>` from the already-elaborated instance, and both const
//! evaluators accept the MemberAccess spelling (guarded on key existence so
//! every other member shape keeps its prior path).
//! J2b/J2c ($bits of a type parameter) is pinned here as already correct.

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

/// Reference: D=6 mask=111111.
#[test]
fn interface_port_parameter_in_constant_context() {
    let src = r#"
interface cfg_if #(parameter int DEPTH = 4) ();
endinterface
module user_m(cfg_if c);
  localparam int D = c.DEPTH;
  logic [c.DEPTH-1:0] mask;
  initial begin
    mask = '1;
    $display("T|D=%0d mask=%b", D, mask);
  end
endmodule
module tb;
  cfg_if #(.DEPTH(6)) ci ();
  user_m u (.c(ci));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(line(&sim, "T|D="), "T|D=6 mask=111111");
}

/// Reference: W=16 then W=8 (declaration order).
#[test]
fn bits_of_type_parameter_is_constant() {
    let src = r#"
module inner #(parameter type T = logic [7:0]) ();
  localparam int W = $bits(T);
  initial $display("T|W=%0d", W);
endmodule
module tb;
  inner #(.T(logic [15:0])) i16 ();
  inner i8 ();
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let ws: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|W="))
        .collect();
    assert!(ws.contains(&"T|W=16".to_string()), "override applies: {ws:?}");
    assert!(ws.contains(&"T|W=8".to_string()), "default applies: {ws:?}");
}
