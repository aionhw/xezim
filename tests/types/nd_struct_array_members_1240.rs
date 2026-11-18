//! IEEE 1800-2023 §7.4.2, §7.2, §10.3.2, §13.5.1: a member of an element of
//! an unpacked array of unpacked structures with two or more unpacked
//! dimensions is an operand like a member of a one-dimensional one: a
//! continuous assignment or `always_comb` reading it wakes when it changes,
//! and the array passed by value carries its members in and out. The
//! members of such an element had no signal, so `assign q2 = a2[0][0].v`
//! never woke and an `input S x[1][1]` formal read 0.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn nd_struct_array_member_wakes_a_continuous_assignment() {
    let msgs = run(r##"
`timescale 1ns/1ps
typedef struct {int v;} S;
module top;
  S a1[2];
  S a2[1][1];
  int q1, q2;
  assign q1 = a1[0].v;
  assign q2 = a2[0][0].v;
  initial begin
    #1 a1[0].v = 7; a2[0][0].v = 7;
    #1 $display("q1=%0d q2=%0d direct=%0d", q1, q2, a2[0][0].v);
  end
endmodule
"##);
    assert_eq!(msgs, ["q1=7 q2=7 direct=7"]);
}

/// §13.5.1: an input formal of the array's shape copies every member in.
#[test]
fn nd_struct_array_passes_its_members_by_value() {
    let msgs = run(r##"
typedef struct {int v;} S;
module top;
  S a[1][1];
  int q;
  function automatic int f(input S x[1][1]); return x[0][0].v; endfunction
  initial begin
    a[0][0].v = 7;
    q = f(a);
    $display("q=%0d", q);
  end
endmodule
"##);
    assert_eq!(msgs, ["q=7"]);
}

/// Two and three dimensions, a nested struct member, `always_comb`, a
/// 4-state member, an element copy, a whole-array copy and comparison,
/// `%p`, input/output/ref formals, a nonblocking element write, a pattern
/// and `foreach`.
#[test]
fn nd_struct_array_members_in_every_use() {
    let msgs = run(r##"
`timescale 1ns/1ps
typedef struct {int v; logic [3:0] w;} S;
typedef struct {S in; int k;} N;
module top;
  S a2[2][2];
  S a3[1][2][2];
  N n2[1][2];
  S b2[2][2];
  int q2, q3, qn, qc, qw;
  S e;
  assign q2 = a2[1][0].v;
  assign q3 = a3[0][1][1].v;
  assign qn = n2[0][1].in.v;
  always_comb qc = a2[0][1].v + a2[1][1].v;
  assign qw = a2[1][1].w;
  function automatic int f(input S x[2][2]); return x[1][0].v + x[1][1].w; endfunction
  function automatic void g(output S x[2][2]); x[0][0].v = 11; x[1][1].w = 4'h3; endfunction
  task automatic h(ref S x[2][2]); x[0][1].v = 21; endtask
  function automatic int f3(input S x[1][2][2]); return x[0][1][1].v; endfunction
  initial begin
    #1 a2[1][0].v = 7; a3[0][1][1].v = 8; n2[0][1].in.v = 9; a2[0][1].v = 2; a2[1][1].v = 3; a2[1][1].w = 4'h5;
    #1 $display("q2=%0d q3=%0d qn=%0d qc=%0d qw=%0d", q2, q3, qn, qc, qw);
    $display("f=%0d f3=%0d", f(a2), f3(a3));
    e = a2[1][1]; $display("e=%p", e);
    b2 = a2; $display("b2[1][0].v=%0d b2[1][1].w=%0d", b2[1][0].v, b2[1][1].w);
    $display("a2=%p", a2);
    g(b2); $display("g: %0d %0d %0d", b2[0][0].v, b2[1][1].w, b2[1][0].v);
    h(a2); $display("h: %0d", a2[0][1].v);
    a2[0][0] <= '{v: 31, w: 4'h1};
    #1 $display("nba: %0d %0d qc=%0d", a2[0][0].v, a2[0][0].w, qc);
    a2 = '{'{'{1, 4'h1}, '{2, 4'h2}}, '{'{3, 4'h3}, '{4, 4'h4}}};
    #1 $display("pat: q2=%0d qc=%0d qw=%0d", q2, qc, qw);
    foreach (a2[i, j]) a2[i][j].v = i * 10 + j;
    #1 $display("foreach: q2=%0d qc=%0d %p", q2, qc, a2[1]);
    $display("eq=%0d", a2 == b2);
  end
endmodule
"##);
    assert_eq!(
        msgs,
        [
            "q2=7 q3=8 qn=9 qc=5 qw=5",
            "f=12 f3=8",
            "e='{v:3, w:'h5}",
            "b2[1][0].v=7 b2[1][1].w=5",
            "a2='{'{'{v:0, w:'hX}, '{v:2, w:'hX}}, '{'{v:7, w:'hX}, '{v:3, w:'h5}}}",
            "g: 11 3 0",
            "h: 21",
            "nba: 31 1 qc=24",
            "pat: q2=3 qc=6 qw=4",
            "foreach: q2=10 qc=12 '{'{v:10, w:'h3}, '{v:11, w:'h4}}",
            "eq=0",
        ]
    );
}

/// §23.3, §25, §27: the arrays of an instance, an interface instance and a
/// generate block, and a three-dimensional one read by `always_comb`.
#[test]
fn nd_struct_array_members_in_instances_and_generate_blocks() {
    let msgs = run(r##"
`timescale 1ns/1ps
typedef struct {int v; real r;} S;
interface ifc;
  S m[2][1];
  int q;
  assign q = m[1][0].v;
endinterface
module leaf;
  S m[1][2];
  int q;
  real rq;
  assign q = m[0][1].v;
  assign rq = m[0][1].r;
endmodule
module top;
  leaf u();
  ifc i();
  for (genvar g = 0; g < 2; g++) begin : gen
    S m[2][2];
    int q;
    assign q = m[g][1].v + g;
  end
  S big[3][2][2];
  int qb;
  always_comb qb = big[2][1][0].v;
  initial begin
    #1 u.m[0][1].v = 5; u.m[0][1].r = 2.5; i.m[1][0].v = 6; gen[0].m[0][1].v = 7; gen[1].m[1][1].v = 8;
    big[2][1][0].v = 9;
    #1 $display("u.q=%0d u.rq=%0.1f i.q=%0d g0=%0d g1=%0d qb=%0d", u.q, u.rq, i.q, gen[0].q, gen[1].q, qb);
  end
endmodule
"##);
    assert_eq!(msgs, ["u.q=5 u.rq=2.5 i.q=6 g0=7 g1=9 qb=9"]);
}
