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

/// §7.4.1: a type parameter bound to an integer atom (`int`, `byte`,
/// `integer`, `int unsigned`) with packed dimensions in a block local is a
/// packed array of the atom: its elements keep the atom's signedness, and
/// the whole array is unsigned, as for the same declaration in the module.
/// Expected values are the reference simulator's.
#[test]
fn integer_atom_binding_with_packed_dimensions() {
    let got = lines(
        r#"
module child #(parameter type T = logic);
  T [1:0] m;
  initial begin
    T [1:0] a;
    a[0] = -1; a[1] = 5; m[0] = -1; m[1] = 5;
    #1 $display("%m blk bits=%0d a0=%0d a1=%0d whole_neg=%0d", $bits(a), a[0], a[1], a < 0);
    $display("%m mod bits=%0d m0=%0d m1=%0d whole_neg=%0d", $bits(m), m[0], m[1], m < 0);
  end
endmodule
module top;
  child #(.T(int)) c();
  child #(.T(byte)) c2();
  child #(.T(integer)) c3();
  child #(.T(int unsigned)) c4();
endmodule
"#,
    );
    for want in [
        "top.c blk bits=64 a0=-1 a1=5 whole_neg=0",
        "top.c mod bits=64 m0=-1 m1=5 whole_neg=0",
        "top.c2 blk bits=16 a0=-1 a1=5 whole_neg=0",
        "top.c3 blk bits=64 a0=-1 a1=5 whole_neg=0",
        "top.c4 blk bits=64 a0=4294967295 a1=5 whole_neg=0",
    ] {
        assert!(
            got.iter().any(|l| l == want),
            "missing `{want}` in {got:#?}"
        );
    }
}

/// §6.18, §7.2: a block typedef built on a type parameter, a struct written
/// in the declaration, a queue of the packed array, a function's return and
/// argument types and a `for` loop variable, with the type bound to a
/// typedef of the top and of a module below the top (§6.20.3). Expected
/// values are the reference simulator's.
#[test]
fn block_typedefs_structs_and_subroutines_of_a_type_parameter() {
    let got = lines(
        r#"
module child #(parameter type T = logic);
  function automatic T [1:0] f(T [1:0] x);
    T [1:0] y;
    y = x;
    y[0] = '1;
    return y;
  endfunction
  initial begin
    #1;
    begin
      typedef T [1:0] pair_t;
      pair_t p;
      struct packed { T a; T b; } s;
      T [1:0] q [$];
      p = 16'h1234;
      s = 16'h5678;
      q.push_back(16'h9abc);
      $display("pair bits=%0d p[1]=%h", $bits(p), p[1]);
      $display("struct bits=%0d s.a=%h", $bits(s), s.a);
      $display("queue elem bits=%0d q[0][0]=%h", $bits(q[0]), q[0][0]);
      $display("f=%h bits=%0d", f(16'h1200), $bits(f(16'h1200)));
    end
    for (T [1:0] i = 0; i < 2; i++) $display("for bits=%0d i=%0d", $bits(i), i);
  end
endmodule
module child2 #(parameter type T = logic);
  initial begin
    T [1:0] a;
    a[1] = 4'h9; a[0] = 4'h3;
    #1 $display("nontop bits=%0d a=%h", $bits(a), a);
  end
endmodule
module mid;
  typedef logic [3:0] nib_t;
  child2 #(.T(nib_t)) c2();
endmodule
module top;
  typedef logic [7:0] byte_t;
  child #(.T(byte_t)) u();
  mid m();
endmodule
"#,
    );
    for want in [
        "pair bits=16 p[1]=12",
        "struct bits=16 s.a=56",
        "queue elem bits=16 q[0][0]=bc",
        "f=12ff bits=16",
        "for bits=16 i=0",
        "for bits=16 i=1",
        "nontop bits=8 a=93",
    ] {
        assert!(
            got.iter().any(|l| l == want),
            "missing `{want}` in {got:#?}"
        );
    }
}

/// §7.4.1: a type parameter bound to an enum with packed dimensions, in a
/// task, a block and the module: an element written through a cast of the
/// type reads back. Expected values are the reference simulator's.
#[test]
fn enum_binding_with_packed_dimensions() {
    let got = lines(
        r#"
module child #(parameter type E = logic);
  E [2:0] me;
  task automatic tk(output int o);
    E [2:0] e;
    e[1] = E'(2);
    o = $bits(e) * 100 + e[1];
  endtask
  initial begin
    int o;
    me[1] = E'(2);
    #1 tk(o);
    $display("tk=%0d me1=%0d", o, me[1]);
    begin
      E [2:0] b;
      E s;
      b[1] = E'(2);
      s = E'(2);
      $display("blk b1=%0d s=%0d bits=%0d", b[1], s, $bits(b));
    end
  end
endmodule
module top;
  typedef enum logic [1:0] {X0, X1, X2} e_t;
  child #(.E(e_t)) u();
endmodule
"#,
    );
    for want in ["tk=602 me1=2", "blk b1=2 s=2 bits=6"] {
        assert!(
            got.iter().any(|l| l == want),
            "missing `{want}` in {got:#?}"
        );
    }
}

#[test]
fn local_alias_of_a_dimensioned_type_parameter() {
    let got = lines(r#"
module child #(parameter type T = logic) (output int width);
  function automatic int size_of_pair();
    typedef T [1:0] pair_t;
    pair_t pair;
    return $bits(pair);
  endfunction
  initial width = size_of_pair();
endmodule
module top;
  int a, b;
  child #(.T(logic [5:0])) one(a);
  child #(.T(logic [12:0])) two(b);
  initial #1 $display("aliases %0d %0d", a, b);
endmodule
"#);
    assert!(got.iter().any(|s| s == "aliases 12 26"), "{got:?}");
}
