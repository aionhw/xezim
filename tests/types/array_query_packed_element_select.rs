//! #984 (H): §20.7, §7.4.5: an array query of a select that names elements
//! of a multi-dimensional packed array describes the dimensions the select
//! leaves: `$size(a3[1])` of `logic [1:0][3:0][7:0] a3` is 4 and
//! `$dimensions(a3[1])` 2. They answered for the flattened `a3[1]` (32,
//! `$left` 31, one dimension), and a select leaving one non-normalized
//! dimension (`a5[0][2]` of `[15:8]` elements) reported `[7:0]`. Expected
//! lines are the reference simulator's.

use xezim::simulate;

#[test]
fn queries_of_a_select_take_the_remaining_dimensions() {
    let src = r#"
module top;
  logic [3:0][7:0] a2;
  logic [1:0][3:0][7:0] a3;
  logic [1:0][3:0][7:0][2:0] a4;
  logic [1:0][0:3][15:8] a5;
  typedef struct packed { logic [1:0][3:0][7:0] f0; } s3_t;
  s3_t s3;
  logic [1:0][3:0][7:0] u3 [2];
  initial begin
    $display("H a3[1]: size=%0d left=%0d right=%0d high=%0d low=%0d inc=%0d dims=%0d",
             $size(a3[1]), $left(a3[1]), $right(a3[1]), $high(a3[1]), $low(a3[1]), $increment(a3[1]), $dimensions(a3[1]));
    $display("H a4[1][1]: size=%0d left=%0d dims=%0d size2=%0d", $size(a4[1][1]), $left(a4[1][1]), $dimensions(a4[1][1]), $size(a4[1][1], 2));
    $display("H a4[1]: size=%0d dims=%0d size3=%0d", $size(a4[1]), $dimensions(a4[1]), $size(a4[1], 3));
    $display("H a5[1]: size=%0d left=%0d right=%0d inc=%0d size2=%0d left2=%0d", $size(a5[1]), $left(a5[1]), $right(a5[1]), $increment(a5[1]), $size(a5[1], 2), $left(a5[1], 2));
    $display("H a5[0][2]: size=%0d left=%0d", $size(a5[0][2]), $left(a5[0][2]));
    $display("H s3.f0[1]: size=%0d left=%0d dims=%0d", $size(s3.f0[1]), $left(s3.f0[1]), $dimensions(s3.f0[1]));
    $display("H u3[1]: size=%0d dims=%0d u3[1][0]: size=%0d dims=%0d", $size(u3[1]), $dimensions(u3[1]), $size(u3[1][0]), $dimensions(u3[1][0]));
    $display("H ctl: %0d %0d %0d %0d %0d %0d %0d", $size(a2[1]), $size(a3[1][1]), $size(a3,1), $size(a3,2), $size(a3,3), $dimensions(a3), $size(a3));
    $display("H bits: %0d %0d %0d", $bits(a3[1]), $bits(a4[1][1]), $bits(s3.f0[1]));
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "H a3[1]: size=4 left=3 right=0 high=3 low=0 inc=1 dims=2",
            "H a4[1][1]: size=8 left=7 dims=2 size2=3",
            "H a4[1]: size=4 dims=3 size3=3",
            "H a5[1]: size=4 left=0 right=3 inc=-1 size2=8 left2=15",
            "H a5[0][2]: size=8 left=15",
            "H s3.f0[1]: size=4 left=3 dims=2",
            "H u3[1]: size=2 dims=3 u3[1][0]: size=4 dims=2",
            "H ctl: 8 8 2 4 8 3 2",
            "H bits: 32 24 32",
        ]
    );
}
