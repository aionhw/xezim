//! Register constant propagation in the bytecode compiler
//! (`fold_const_regs`): an unrolled `i = 0; bus[i] = v; i = i + k; …`
//! sequence compiles to static bit writes, and the folded values must match
//! the interpreter's own arithmetic (signed 32-bit integer steps, a step
//! past the bus width dropped, an X index dropped).

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn unrolled_index_chain_folds_to_static_bit_writes() {
    let out = msgs(
        r#"
module tb;
  logic [199:0] bus;
  logic a, b, c, d;
  integer i;
  always @* begin
    bus = '0;
    i = 0;
    bus[i] = a; i = i + 1;
    bus[i] = b; i = i + 5;
    bus[i] = c; i = i + 64;
    bus[i] = d; i = i + 130;
    bus[i] = 1'b1;          // i = 200: past the width, dropped
  end
  initial begin
    a = 1; b = 1; c = 0; d = 1; #1;
    $display("A=%b %b %b %b %b", bus[0], bus[1], bus[6], bus[70], bus[199]);
    c = 1; d = 0; #1;
    $display("B=%b %b", bus[6], bus[70]);
    $finish;
  end
endmodule
"#,
    );
    assert!(out.iter().any(|m| m == "A=1 1 0 1 0"), "{out:?}");
    assert!(out.iter().any(|m| m == "B=1 0"), "{out:?}");
}

#[test]
fn folded_compare_and_xor_match_interpreter() {
    let out = msgs(
        r#"
module tb;
  logic [7:0] r;
  integer k;
  // IEEE 1800-2023 §9.2.2.2.2: evaluate once without external stimulus.
  always_comb begin
    k = 3;
    k = k + 4;                 // 7
    r[0] = (k == 7);
    r[1] = (k == 8);
    k = k ^ 32'h5;             // 2
    r[7:2] = k[5:0];
  end
  initial begin
    #1 $display("C=%b", r);
    $finish;
  end
endmodule
"#,
    );
    assert!(out.iter().any(|m| m == "C=00001001"), "{out:?}");
}
