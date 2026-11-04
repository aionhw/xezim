//! IEEE 1800-2023 §10.9.1: in a nested array parameter whose element is a
//! packed struct, a `default:` holding the struct's literal fills every
//! leaf. The row/leaf split took any pattern as a row, so `A[0][0]` was
//! packed from the two scalars `7` and `2'd1` as 34 bits (xezim-core #91
//! follow-up).

use xezim::simulate;

#[test]
fn struct_literal_default_fills_every_leaf() {
    let src = r#"
typedef struct packed { bit [31:0] s; bit [1:0] x; } p_t;
module top;
  localparam p_t A [2][2] = '{default: '{7, 2'd1}};
  localparam p_t R [2][2] = '{default: '{'{3, 2'd2}, '{4, 2'd3}}};
  localparam int B [2][3] = '{default: 9};
  initial $display("A=%0d,%0d,%0d,%0d R=%0d,%0d,%0d,%0d B=%0d,%0d",
    A[0][0].s, A[0][0].x, A[1][1].s, A[1][1].x,
    R[0][0].s, R[0][0].x, R[1][1].s, R[1][1].x, B[0][0], B[1][2]);
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"A=7,1,7,1 R=3,2,4,3 B=9,9"), "{msgs:?}");
}
