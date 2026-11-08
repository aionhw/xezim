//! IEEE 1800-2023 §7.4.1: a function local or formal declared with an
//! ascending (or offset) outer packed dimension keeps its element order when
//! the call is compiled into a continuous assignment or `always_comb`.
//!
//! The bytecode compiler inlines such a call and keeps each packed local in a
//! register, slicing element `i` as bits `[i*w +: w]`. That is the layout of
//! `[N-1:0]` only: for `u8_t [0:3] r` (or `logic [1:4][7:0]`) the element
//! order came back reversed (`assign d = mk();` gave `43424140`), while the
//! same call from a procedural statement, which the interpreter runs, gave
//! `40414243`. Such a local or formal now keeps the call off the inlined path.
//!
//! Expected values are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
package P; typedef logic [7:0] u8_t; endpackage
module m4(o); import P::*; output u8_t [0:3] o;
  function automatic u8_t [0:3] mk();
    u8_t [0:3] r;
    for (int i = 0; i < 4; i++) r[i] = 8'h40 + i[7:0];
    return r;
  endfunction
  assign o = mk();
endmodule
module tb; import P::*;
  u8_t [0:3] d, e, h, k;
  logic [0:3][7:0] g;
  logic [1:4][7:0] n;
  function automatic u8_t [0:3] mk();
    u8_t [0:3] r;
    for (int i = 0; i < 4; i++) r[i] = 8'h40 + i[7:0];
    return r;
  endfunction
  function automatic logic [0:3][7:0] mk2();
    logic [0:3][7:0] r;
    for (int i = 0; i < 4; i++) r[i] = 8'h50 + i[7:0];
    return r;
  endfunction
  function automatic logic [1:4][7:0] mk3();
    logic [1:4][7:0] r;
    for (int i = 1; i <= 4; i++) r[i] = 8'h60 + i[7:0];
    return r;
  endfunction
  function automatic logic [7:0] pick(input u8_t [0:3] v, input int j);
    return v[j];
  endfunction
  assign d = mk();
  assign g = mk2();
  assign n = mk3();
  always_comb e = mk();
  assign h = {pick(32'h01020304, 0), pick(32'h01020304, 3), 16'h0};
  m4 u(.o(k));
  initial begin #2 $display("%h %h %h %h %h %h", d, g, n, e, h, k); end
endmodule
"#;

#[test]
fn ascending_packed_function_locals_keep_element_order() {
    let sim = simulate(SRC, 10).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    let want = "40414243 50515253 61626364 40414243 01040000 40414243";
    assert!(
        out.iter().any(|l| l.trim() == want),
        "missing {want:?} in {out:?}"
    );
}
