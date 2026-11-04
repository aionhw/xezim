//! §29.3.2, A.5.2 (#868): a sequential UDP declares its output `reg`, either
//! as `output q; reg q;` or within the output declaration, `output reg q;`.
//! In the body of a non-ANSI header the second form was skipped whole, so
//! the UDP was taken for a combinational one and its output stayed x. The
//! fixture writes a level-sensitive, an edge-sensitive and an
//! `initial`-statement UDP in the three declaration forms with the same
//! tables; the reference simulator prints `ALL PASS` for it.

use xezim::simulate;

#[test]
fn output_reg_in_a_non_ansi_body_makes_the_udp_sequential() {
    let src = include_str!("udp_output_reg_body_868.sv");
    let sim = simulate(src, 10_000).expect("simulate failed");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert!(!lines.iter().any(|l| l.starts_with("FAIL")), "{lines:#?}");
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("PASS")).count(),
        9,
        "{lines:#?}"
    );
    assert_eq!(lines.last().copied(), Some("ALL PASS"), "{lines:#?}");
}

/// A.5.2: `output reg q = v` gives the start state, in the body of a
/// non-ANSI header and in an ANSI header, as `initial q = v;` does (§29.7).
#[test]
fn output_reg_with_a_value_gives_the_start_state() {
    let src = r#"
primitive hold_body (q, a);
  output reg q = 1'b1; input a;
  table
    ? : ? : -;
  endtable
endprimitive
primitive hold_ansi (output reg q = 1'b0, input a);
  table
    ? : ? : -;
  endtable
endprimitive
module t;
  reg a;
  wire hb, ha;
  hold_body u1 (hb, a);
  hold_ansi u2 (ha, a);
  initial begin
    #1 $display("init %b %b", hb, ha);
    a = 0; #1 a = 1; #1 $display("held %b %b", hb, ha);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    let lines: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    assert_eq!(lines, ["init 1 0", "held 1 0"]);
}
