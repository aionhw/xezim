//! A typedef that names `real`, `shortreal` or `realtime` (directly, through
//! another alias, a package, a class or a type parameter) declares real
//! objects (IEEE 1800-2023 §6.18, §6.12): variables, class properties,
//! subroutine formals and return values, parameters and cast targets keep
//! the fraction of what is assigned to them. Expected values are the
//! reference simulator's.

use xezim::simulate;

fn check_output(src: &str, expected: &[&str]) {
    let sim = simulate(src, 20).expect("simulate failed");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.trim().strip_prefix("CHECK "))
        .collect();
    assert_eq!(actual, expected);
}

/// §6.18, §6.21: block and module variables of an alias, an alias of an
/// alias, a package alias, `realtime` and `shortreal` aliases, arrays and
/// structure members.
#[test]
fn variables_of_a_real_alias_keep_fractions() {
    let src = r#"
package p; typedef real PR; endpackage
module top;
  typedef real R;
  typedef R R2;
  typedef realtime RT;
  typedef shortreal SR;
  typedef R A2[2];
  typedef struct { R a; R m[2]; } S;
  R mv = 1.25; R2 mv2; p::PR mpv; RT mrt; SR msr;
  R marr[2]; A2 ma2; R mq[$]; R md[]; R maa[int];
  initial begin
    R a; R b = 2.7; R2 c; RT t; SR s; R la[2]; R lq[$]; S st;
    a = 1.5; c = -2.25; t = 0.5; s = 3.75;
    la[0] = 1.5; la[1] = 2.5; lq.push_back(1.5);
    st.a = 1.5; st.m[1] = 2.5;
    $display("CHECK local %0.2f %0.2f %0.2f %0.2f %0.2f", a, b, c, t, s);
    $display("CHECK larr %0.2f %0.2f %0.2f", la[0], la[1], lq[0]);
    $display("CHECK st %0.2f %0.2f", st.a, st.m[1]);
    mv2 = 1.5; mpv = 2.5; mrt = 3.5; msr = 4.5;
    marr[0] = 1.5; ma2[1] = 2.5; mq.push_back(3.5);
    md = new[2]; md[1] = 4.5; maa[3] = 5.5;
    $display("CHECK mod %0.2f %0.2f %0.2f %0.2f %0.2f", mv, mv2, mpv, mrt, msr);
    $display("CHECK marr %0.2f %0.2f %0.2f %0.2f %0.2f",
             marr[0], ma2[1], mq[0], md[1], maa[3]);
  end
endmodule
"#;
    check_output(
        src,
        &[
            "local 1.50 2.70 -2.25 0.50 3.75",
            "larr 1.50 2.50 1.50",
            "st 1.50 2.50",
            "mod 1.25 1.50 2.50 3.50 4.50",
            "marr 1.50 2.50 3.50 4.50 5.50",
        ],
    );
}

/// §8.5, §8.25: class properties of an alias declared outside or inside the
/// class, a static property, and a property, formal and local of a type
/// parameter bound to `real` (and to `int`, which still rounds).
#[test]
fn class_properties_and_methods_of_a_real_alias() {
    let src = r#"
package p; typedef real PR; endpackage
module top;
  typedef real R;
  class Box #(type T = int);
    R val; R q = 0.5; static R sv = 0.75; p::PR pv; R arr[2]; R dq[$];
    T tv;
    typedef real CR;
    CR cv;
    function void put(R v); val = v; endfunction
    function R get(); return val; endfunction
    function R impl(input real x); impl = x; impl = impl + 0.25; endfunction
    function CR cget(CR x); CR l; l = x / 2; cv = l; return l; endfunction
    function T tget(input T x);
      T l;
      l = 3.7;
      $display("CHECK tparam l=%0.2f x/2=%0.2f", l, x / 2);
      return l;
    endfunction
    task tk(input R i, output R o, inout R q2);
      i = i + 0.5; o = 1.5; q2 = q2 + 0.25;
      $display("CHECK ctask %0.2f %0.2f %0.2f", i, o, q2);
    endtask
  endclass
  initial begin
    Box #(real) bx; Box #(int) bi; real x, y;
    bx = new; bi = new;
    bx.put(3.14159); bx.pv = 1.5; bx.arr[1] = 2.5; bx.dq.push_back(3.5); bx.tv = 4.5;
    $display("CHECK cls %0.5f %0.2f %0.2f %0.2f %0.2f %0.2f %0.2f",
             bx.get(), bx.q, Box#(real)::sv, bx.pv, bx.arr[1], bx.dq[0], bx.tv);
    bx.val = 3.9;
    $display("CHECK direct %0.2f impl=%0.2f cget=%0.2f cv=%0.2f",
             bx.val, bx.impl(1.5), bx.cget(3), bx.cv);
    void'(bx.tget(3)); void'(bi.tget(3));
    x = 0; y = 0; bx.tk(1, x, y);
    $display("CHECK ctask out %0.2f %0.2f", x, y);
  end
endmodule
"#;
    check_output(
        src,
        &[
            "cls 3.14159 0.50 0.75 1.50 2.50 3.50 4.50",
            "direct 3.90 impl=1.75 cget=1.50 cv=1.50",
            "tparam l=3.70 x/2=1.50",
            "tparam l=4.00 x/2=1.00",
            "ctask 1.50 1.50 0.25",
            "ctask out 1.50 0.25",
        ],
    );
}

/// §13.4.1, §13.5: the implicit return variable, the locals and the formals
/// of a function or task declared with an alias are reals; an integral
/// actual converts on the way in (§6.12.1).
#[test]
fn subroutine_formals_and_returns_of_a_real_alias() {
    let src = r#"
module top;
  typedef real R;
  function automatic R fimpl(input real x); fimpl = x; fimpl = fimpl + 0.25; endfunction
  function R fstat(input R x); R l; l = x * 0.5; return l; endfunction
  task automatic tk(input R i, output R o, inout R q);
    i = i + 0.5; o = 1.5; q = q + 0.25;
    $display("CHECK task %0.2f %0.2f %0.2f", i, o, q);
  endtask
  initial begin
    real x, y;
    $display("CHECK fimpl %0.2f fstat %0.2f", fimpl(1.5), fstat(3));
    x = 0; y = 0; tk(1, x, y);
    $display("CHECK task out %0.2f %0.2f", x, y);
  end
endmodule
"#;
    check_output(
        src,
        &[
            "fimpl 1.75 fstat 1.50",
            "task 1.50 1.50 0.25",
            "task out 1.50 0.25",
        ],
    );
}

/// §6.20.2, §6.24.1: parameters and localparams of an alias, a module type
/// parameter bound to one, and a cast to an alias as a real operand in a
/// procedure and in a continuous assignment.
#[test]
fn parameters_and_casts_of_a_real_alias() {
    let src = r#"
package p; typedef real PR; endpackage
module sub #(parameter type T = int, parameter p::PR MP = 1.5) ();
  T v;
  localparam p::PR ML = MP + 1.0;
  initial begin
    v = 2.5;
    #2 $display("CHECK sub v=%0.2f MP=%0.2f ML=%0.2f", v, MP, ML);
  end
endmodule
module top;
  typedef real R;
  typedef R R2;
  parameter R P = 1.5;
  localparam R L = 2.5;
  localparam p::PR LP = 0.25;
  sub #(.T(real)) u_s();
  sub #(.T(R), .MP(2.25)) u_s2();
  R wr; R drv = 0.5;
  assign wr = drv * 1.5;
  int ii = 3;
  real ca, cb;
  assign ca = R'(ii) / 2;
  assign cb = R2'(ii) / 2 + p::PR'(ii) / 4;
  initial begin
    $display("CHECK param %0.2f %0.2f %0.2f", P, L, LP);
    $display("CHECK cast %0.2f %0.2f %0.2f", R'(ii) / 2, R'(1) / 2, R2'(ii) / 2);
    #1 $display("CHECK wr=%0.2f ca=%0.2f cb=%0.2f", wr, ca, cb);
  end
endmodule
"#;
    check_output(
        src,
        &[
            "param 1.50 2.50 0.25",
            "cast 1.50 0.50 1.50",
            "wr=0.75 ca=1.50 cb=2.25",
            "sub v=2.50 MP=1.50 ML=2.50",
            "sub v=2.50 MP=2.25 ML=3.25",
        ],
    );
}
