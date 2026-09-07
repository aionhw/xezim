//! §6.10: an implicit net belongs to the scope that uses it, even when its
//! bare name is spoken for somewhere else in the design.
//!
//! Elaboration registers PLACEHOLDER SIGNALS for things that are not nets —
//! instance names and generate-block labels among them. Two implicit-net
//! passes then asked `signals.contains_key(<bare name>)` to decide whether a
//! scoped net was still needed, so any of those placeholders (or an unrelated
//! signal that merely shared the spelling) suppressed the real per-instance
//! net. The child port was left undriven and read x.
//!
//! A gate library hits this constantly: a vendor flop cell contains
//! `buf IC (clk, dCK);` — an implicit net named `clk` — and every design has
//! some other `clk`. The whole cell then read x, which looked like a CDC race
//! in an async FIFO rather than a naming bug.
//!
//! Both spellings are covered here because they are created by DIFFERENT
//! passes: the instance-port pass and `create_implicit_nets_for_pending`.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

const PRIMS: &str = r#"
primitive p_buf (out, in); output out; input in;
  table 1 : 1 ; 0 : 0 ; endtable
endprimitive
"#;

#[test]
fn an_implicit_net_survives_an_instance_of_the_same_name() {
    // The third instance is named `c`; the cell's `buf` makes an implicit net
    // `c`. Before the fix only the instance whose name matched worked: x x 1.
    let o = out(&format!(
        r#"{PRIMS}
module cellu (output Q, input A);
  buf B (c, A);
  p_buf P (Q, c);
endmodule
module tb;
  logic a = 0;
  wire q1, q2, q3;
  cellu a1 (.Q(q1), .A(a));
  cellu b  (.Q(q2), .A(a));
  cellu c  (.Q(q3), .A(a));      // instance name == the cell's implicit net
  initial begin #5 a = 1; #5; $display("Q=%b%b%b", q3, q2, q1); end
endmodule
"#
    ));
    assert!(o.contains("Q=111"), "instance-name collision broke the net:\n{o}");
}

#[test]
fn an_implicit_net_survives_a_generate_label_of_the_same_name() {
    // Same class, different creating pass: a generate-block label is also
    // registered as a placeholder signal. Before the fix: x0 instead of 11.
    let o = out(&format!(
        r#"{PRIMS}
module cellu (output Q, input A);
  buf B (gen, A);
  p_buf P (Q, gen);
endmodule
module tb;
  logic a = 0;
  wire q1, q2;
  genvar i;
  generate for (i = 0; i < 1; i = i + 1) begin : gen   // label `gen`
    wire dummy;
  end endgenerate
  cellu u1 (.Q(q1), .A(a));
  cellu u2 (.Q(q2), .A(a));
  initial begin #5 a = 1; #5; $display("G=%b%b", q2, q1); end
endmodule
"#
    ));
    assert!(o.contains("G=11"), "generate-label collision broke the net:\n{o}");
}

#[test]
fn an_implicit_net_survives_a_top_level_signal_of_the_same_name() {
    // The shape the original report reduced to: the cell's internal clock net
    // is named `clk`, and the testbench has its own `clk`.
    let o = out(&format!(
        r#"{PRIMS}
module cellu (output Q, input CK);
  buf IC (clk, CK);            // implicit net `clk` inside the cell
  p_buf P (Q, clk);
endmodule
module tb;
  logic clk = 0;               // and a top-level `clk`
  wire q1, q2;
  cellu u1 (.Q(q1), .CK(clk));
  cellu u2 (.Q(q2), .CK(clk));
  initial begin #5 clk = 1; #5; $display("C=%b%b", q2, q1); end
endmodule
"#
    ));
    assert!(o.contains("C=11"), "top-level signal collision broke the net:\n{o}");
}
