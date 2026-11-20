//! §6.20.3, §7.4.1: a variable declared inside a procedural block or a
//! function with a type parameter and packed dimensions (`T [1:0] tmp;`)
//! is a packed array of the bound type.
//!
//! An instance's type-parameter bindings are substituted into the block
//! declarations of its cloned statements, but only for a reference without
//! dimensions (`T tmp;`). `T [1:0] tmp;` kept the bare `T`, which no longer
//! names the binding once the instance is inlined, so the local held one
//! element of the wrong width and every element select read the wrong bits.
//! The binding's own packed dimensions stay innermost and the declared ones
//! go outside them.
//!
//! Expected values are Verilator's (5.046).

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

/// A struct and a vector bound to type parameters, used as the
/// element of block locals (read, member write) and of a function local.
#[test]
fn block_locals_of_a_type_parameter_with_packed_dimensions() {
    let got = lines(
        r#"
module child #(parameter type T = logic, parameter type V = logic [7:0])
  (input T [1:0] a, output logic [3:0] r, output logic [3:0] r2, output logic [71:0] w,
   output logic [7:0] vr, output logic [7:0] fr);
  function automatic logic [7:0] pick(input logic [15:0] x);
    V [1:0] y;
    y = x;
    return y[1];
  endfunction
  always_comb begin
    T [1:0] tmp;
    T [1:0] t2;
    V [1:0] v;
    tmp = a;
    r = tmp[1].id;
    w = tmp;
    t2 = a;
    t2[1].id = t2[1].id + 4'd1;
    r2 = t2[1].id;
    v = 16'hA55A;
    vr = v[1];
    fr = pick(16'h3CC3);
  end
endmodule
module top;
  typedef struct packed { logic [3:0] id; logic [31:0] data; } s_t;
  s_t [1:0] x;
  logic [3:0] r, r2; logic [71:0] w; logic [7:0] vr, fr;
  initial begin x[0] = 36'h1_0000_00AB; x[1] = 36'h2_0000_00CD;
    #1 $display("t1: r=%h r2=%h w=%h vr=%h fr=%h", r, r2, w, vr, fr); end
  child #(.T(s_t)) c (.a(x), .r(r), .r2(r2), .w(w), .vr(vr), .fr(fr));
endmodule
"#,
    );
    let want = "t1: r=2 r2=3 w=2000000cd1000000ab vr=a5 fr=3c";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

/// §6.18: a typedef declared in a block hides a type parameter of the same
/// name there, with or without packed dimensions on the declaration; in
/// another block the parameter is visible again.
#[test]
fn block_typedef_hides_the_type_parameter() {
    let got = lines(
        r#"
// a block-local typedef that hides a type parameter of the same name
module child #(parameter type T = logic [7:0]) (output int bx, output int by, output int bz);
  always_comb begin
    typedef logic [3:0] T;
    T [1:0] x;
    T y;
    x = '1;
    y = '1;
    bx = $bits(x);
    by = $bits(y);
  end
  always_comb begin
    T [1:0] z;
    z = '1;
    bz = $bits(z);
  end
endmodule
module top;
  int bx, by, bz;
  child #(.T(logic [5:0])) c (.bx(bx), .by(by), .bz(bz));
  initial #1 $display("t3: bx=%0d by=%0d bz=%0d", bx, by, bz);
endmodule
"#,
    );
    let want = "t3: bx=8 by=4 bz=12";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}
