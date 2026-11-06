//! IEEE 1800-2023 §10.8, §11.6.1: a parameter with a declared type is
//! assigned its initializer, so the operands are evaluated at the declared
//! width — `localparam int A = (8'd128 + 8'd128) >> 8` is 1, not the 8-bit
//! wrap 0 — wherever the parameter is read: in a packed dimension, a
//! part-select, a replication count and an unpacked size, not only in its
//! own value (#894). An untyped parameter keeps the self-determined width.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn out(src: &str) -> Vec<String> {
    let sim = simulate(src, 10).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn typed_localparam_sizes_declarations_at_its_declared_width() {
    let o = out(r#"
module child;
  localparam int A = (8'd128 + 8'd128) >> 8;
  logic [A:0] v;
  initial $display("T|child A=%0d bits=%0d", A, $bits(v));
endmodule
module top;
  localparam int A = (8'd128 + 8'd128) >> 8;
  localparam logic [15:0] B = 8'd200 + 8'd100;
  parameter int C = (8'd128 + 8'd128) >> 8;
  localparam A2 = (8'd128 + 8'd128) >> 8;
  logic [A:0] v;
  logic [B:0] vb;
  logic [C:0] vc;
  logic [A2:0] va2;
  logic [7:0] x = 8'hA5;
  logic [A:0] ps;
  logic [A+1:0] rep;
  int ua [B];
  child u();
  initial begin
    ps = x[A:0];
    rep = {(A+1){1'b1}};
    $display("T|A=%0d bits=%0d B=%0d bitsb=%0d C=%0d bitsc=%0d A2=%0d bitsa2=%0d ps=%b rep=%b sizeua=%0d",
      A, $bits(v), B, $bits(vb), C, $bits(vc), A2, $bits(va2), ps, rep, $size(ua));
  end
endmodule
"#);
    assert_eq!(
        o,
        [
            "T|child A=1 bits=2",
            "T|A=1 bits=2 B=300 bitsb=301 C=1 bitsc=2 A2=0 bitsa2=1 ps=01 rep=011 sizeua=300",
        ]
    );
}
