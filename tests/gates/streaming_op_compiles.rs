//! §11.4.14: `{>>{…}}` / `{<<N{…}}` compile instead of falling back.
//!
//! A streaming concatenation is a FIXED bit permutation once the slice size
//! and the total width are known: `{>>{…}}` is exactly the concatenation, and
//! `{<<N{…}}` reverses the order of N-bit slices, leaving any leftover high
//! bits at the LSB end. Both lower to constant range selects plus one concat.
//! Previously the whole expression went to the AST interpreter — a byte swap
//! written `{<<8{x}}` ran ~32% slower than the same swap written out by hand.
//!
//! The width gate is the delicate part and is deliberately narrow. Placing the
//! slices needs each operand's width to be exactly right, and the general
//! width oracles are not trustworthy enough: BOTH `lrm_self_width` and
//! `expr_max_width` report 1 for an element of a packed-struct typedef array
//! in a submodule, so cross-checking them does not catch it — a total of 1
//! silently produced a 1-bit result. Only whole signals and constant-bound
//! part-selects are accepted; everything else keeps the AST path, which is
//! correct.
//!
//! Every expected value here is the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn streaming_forms_match_the_reference() {
    let o = out(r#"
module tb;
  logic [31:0] v32; logic [11:0] v12; logic [6:0] v7;
  initial begin
    v32 = 32'hDEAD_BEEF; v12 = 12'hA5C; v7 = 7'h5A;
    $display("A=%08x", {<<8{v32}});     // byte reverse
    $display("B=%08x", {<<{v32}});      // bit reverse (default slice 1)
    $display("C=%08x", {>>{v32}});      // no reorder
    $display("D=%08x", {<<16{v32}});    // halfword swap
    $display("E=%03x", {<<4{v12}});     // exact multiple
    $display("F=%03x", {<<5{v12}});     // 12/5 -> remainder 2
    $display("G=%02x", {<<3{v7}});      // 7/3  -> remainder 1
    $display("H=%011x", {<<8{v32, v12}});   // multi-operand
    $display("I=%08x", {<<8{v32[31:0]}});   // constant part-select operand
  end
endmodule
"#);
    // Reference simulator:
    for expect in [
        "A=efbeadde",
        "B=f77db57b",
        "C=deadbeef",
        "D=beefdead",
        "E=c5a",
        "F=e4a",
        "G=27",
        "H=5cfaeedbead",
        "I=efbeadde",
    ] {
        assert!(o.contains(expect), "expected {expect} in:\n{o}");
    }
}

/// The shape that made a naive width gate produce garbage: an element of a
/// packed-struct typedef array inside a submodule, where both width oracles
/// report 1. It must stay on the AST path and keep giving the reference's
/// answer rather than a 1-bit result.
#[test]
fn streaming_a_struct_array_element_stays_correct() {
    let o = out(r#"
package p;
  typedef struct packed {
    logic [63:0] pd; logic [7:0] tag; logic v; logic e;
  } line_t;                                   // 74 bits
endpackage
import p::*;
module churn (input line_t [1:0] w);
  line_t [1:0] gen;
  assign gen[0] = {<<{w[0]}};
  logic [63:0] lo0, hi0;
  always @(*) begin
    lo0 = gen[0][63:0];
    hi0 = {54'b0, gen[0][73:64]};
  end
endmodule
module tb;
  line_t [1:0] arr;
  churn u_c (.w(arr));
  initial begin
    arr[0] = {64'hA5A5_A5A5_B4B4_B4B4, 8'h11, 1'b1, 1'b0};
    arr[1] = {64'h5A5A_5A5A_C3C3_C3C3, 8'h22, 1'b0, 1'b1};
    #2;
    $display("LO0=%016x HI0=%016x", u_c.lo0, u_c.hi0);
  end
endmodule
"#);
    // Reference simulator: LO0=2d2d2d2da5a5a5a5 HI0=0000000000000188
    assert!(
        o.contains("LO0=2d2d2d2da5a5a5a5 HI0=0000000000000188"),
        "struct-array streaming lost bits:\n{o}"
    );
}
