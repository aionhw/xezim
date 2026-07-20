//! §7.4.5 / §20.7: array query functions ($left/$right/$high/$low/$size) on a
//! MULTI-DIMENSIONAL PACKED vector must report per-dimension bounds, not the
//! flattened vector. `logic [3:0][7:0] pk` has dim1=[3:0] and dim2=[7:0], so
//! `$left(pk)`=3 and `$size(pk)`=4 — the old code collapsed to the 32-bit
//! flat vector and returned 31 / 32 for every dimension. Dimensions number
//! unpacked-first, then packed.

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn query_multidim_packed_vector() {
    const SRC: &str = r#"
module top;
  logic [3:0][7:0] pk;         // dim1 [3:0], dim2 [7:0]
  logic [3:0][7:0] arr [0:1];  // dim1 unpacked [0:1], dim2 [3:0], dim3 [7:0]
  initial begin
    $display("P1 %0d %0d %0d %0d", $left(pk), $right(pk), $high(pk), $size(pk));
    $display("P2 %0d %0d %0d", $left(pk,2), $right(pk,2), $size(pk,2));
    $display("A %0d %0d %0d", $size(arr,1), $size(arr,2), $size(arr,3));
    $finish;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out = output_of(&sim);
    assert!(out.contains("P1 3 0 3 4"), "dim1 of [3:0][7:0] is [3:0], size 4:\n{}", out);
    assert!(out.contains("P2 7 0 8"), "dim2 is [7:0], size 8:\n{}", out);
    assert!(out.contains("A 2 4 8"), "mixed unpacked+packed dim sizes:\n{}", out);
}
