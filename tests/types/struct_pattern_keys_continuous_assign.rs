//! IEEE 1800-2023 §10.9.1 / §10.9.2: a continuous assignment (§10.3.2) of a
//! structure pattern gives an unpacked array member each element of its
//! nested pattern, whether the elements are set by position, by replication
//! ("Each replication shall represent an entire single dimension"), by index
//! key, by type key or by `default:`; and a structure-level `default:` or
//! type key reaches the members and elements of a nested array or structure
//! ("Use of the default keyword applies to members in nested structures or
//! elements in unpacked arrays in structures"). An unpacked struct is stored
//! one signal per leaf, so the elaborator binds the pattern to every leaf; a
//! member it could not bind was assigned whole, had no storage of its own,
//! and read 0 (variables) or x (nets of a user nettype), #1081. Expected
//! values are the reference simulator's except where a test says otherwise.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("Simulation"))
        .collect()
}

/// The reporter's testcase: `default:`, index keys and a replication.
#[test]
fn reporter_default_keys_and_replication() {
    let src = r#"
`timescale 1ns/1ps
typedef struct { int tag; int a[2]; } S;
module top;
  int x;
  S v_default, v_keys, v_rep;
  assign v_default = '{tag:x, a:'{default:x}};
  assign v_keys    = '{tag:x, a:'{0:x+1, 1:x+2}};
  assign v_rep     = '{tag:x, a:'{2{x+3}}};
  initial begin
    #1 x = 7;
    #1 $display("default %0d,%0d", v_default.a[0], v_default.a[1]);
       $display("keys %0d,%0d", v_keys.a[0], v_keys.a[1]);
       $display("replication %0d,%0d", v_rep.a[0], v_rep.a[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(lines(src), ["default 7,7", "keys 8,9", "replication 10,10"]);
}

/// Structure-level `default:` and type keys, two-dimensional members,
/// arrays of structures, descending ranges and type keys inside a member.
#[test]
fn nested_keys_reach_every_leaf() {
    let src = r#"
`timescale 1ns/1ps
typedef struct { int p; int q; } PQ;
typedef struct { int tag; int a[2]; } S;
typedef struct { int tag; int m[2][3]; } M;
typedef struct { int tag; PQ b[2]; } B;
typedef struct { int tag; int d[1:0]; } D;
typedef struct { int tag; int a[2]; byte c; } T;
module top;
  int x;
  S s_def_struct, s_tag_def, s_type;
  M m_def, m_rep, m_keys;
  B b_def, b_ord, b_keys;
  D d_keys, d_rep;
  T t_type;
  assign s_def_struct = '{default: x};
  assign s_tag_def = '{tag: x, default: x+1};
  assign m_def = '{tag: x, m: '{default: x}};
  assign m_rep = '{tag: x, m: '{2{'{3{x+1}}}}};
  assign m_keys = '{tag: x, m: '{0: '{default: x+2}, 1: '{x, x+1, x+2}}};
  assign b_def = '{tag: x, b: '{default: x}};
  assign b_ord = '{tag: x, b: '{'{x, x+1}, '{p: x+2, q: x+3}}};
  assign b_keys = '{tag: x, b: '{1: '{default: x+4}, default: '{p: x, q: 0}}};
  assign d_keys = '{tag: x, d: '{1: x+1, 0: x+2}};
  assign d_rep = '{tag: x, d: '{2{x+3}}};
  assign t_type = '{int: x, byte: 8'd5};
  assign s_type = '{tag: x, a: '{int: x+6}};
  initial begin
    #1 x = 7;
    #1;
    $display("s_def_struct %0d %0d %0d", s_def_struct.tag, s_def_struct.a[0], s_def_struct.a[1]);
    $display("s_tag_def %0d %0d %0d", s_tag_def.tag, s_tag_def.a[0], s_tag_def.a[1]);
    $display("m_def %0d %0d %0d %0d %0d %0d %0d", m_def.tag, m_def.m[0][0], m_def.m[0][1], m_def.m[0][2], m_def.m[1][0], m_def.m[1][1], m_def.m[1][2]);
    $display("m_rep %0d %0d %0d %0d %0d %0d %0d", m_rep.tag, m_rep.m[0][0], m_rep.m[0][1], m_rep.m[0][2], m_rep.m[1][0], m_rep.m[1][1], m_rep.m[1][2]);
    $display("m_keys %0d %0d %0d %0d %0d %0d %0d", m_keys.tag, m_keys.m[0][0], m_keys.m[0][1], m_keys.m[0][2], m_keys.m[1][0], m_keys.m[1][1], m_keys.m[1][2]);
    $display("b_def %0d %0d %0d %0d %0d", b_def.tag, b_def.b[0].p, b_def.b[0].q, b_def.b[1].p, b_def.b[1].q);
    $display("b_ord %0d %0d %0d %0d %0d", b_ord.tag, b_ord.b[0].p, b_ord.b[0].q, b_ord.b[1].p, b_ord.b[1].q);
    $display("b_keys %0d %0d %0d %0d %0d", b_keys.tag, b_keys.b[0].p, b_keys.b[0].q, b_keys.b[1].p, b_keys.b[1].q);
    $display("d_keys %0d %0d %0d", d_keys.tag, d_keys.d[1], d_keys.d[0]);
    $display("d_rep %0d %0d %0d", d_rep.tag, d_rep.d[1], d_rep.d[0]);
    $display("t_type %0d %0d %0d %0d", t_type.tag, t_type.a[0], t_type.a[1], t_type.c);
    $display("s_type %0d %0d %0d", s_type.tag, s_type.a[0], s_type.a[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "s_def_struct 7 7 7",
            "s_tag_def 7 8 8",
            "m_def 7 7 7 7 7 7 7",
            "m_rep 7 8 8 8 8 8 8",
            "m_keys 7 9 9 9 7 8 9",
            "b_def 7 7 7 7 7",
            "b_ord 7 7 8 9 10",
            "b_keys 7 7 0 11 11",
            "d_keys 7 8 9",
            "d_rep 7 10 10",
            "t_type 7 7 7 5",
            "s_type 7 13 13",
        ]
    );
}

/// Nets of a user nettype (§6.6.7): without a resolver the pattern is an
/// ordinary assignment; with one, each element of the resolved value
/// reaches its leaf, also for ordered patterns and two drivers.
#[test]
fn nettype_array_members() {
    let src = r#"
`timescale 1ns/1ps
typedef struct { logic [7:0] tag; logic [7:0] a[2]; } S;
function automatic S f(input S d[]);
  f = '{default: 0};
  foreach (d[i]) begin
    f.tag += d[i].tag;
    f.a[0] += d[i].a[0];
    f.a[1] += d[i].a[1];
  end
endfunction
nettype S N;
nettype S R with f;
module top;
  logic [7:0] x;
  N n_def, n_keys, n_rep, n_ord;
  R r_ord, r_def, r_two;
  assign n_def = '{tag: x, a: '{default: x}};
  assign n_keys = '{tag: x, a: '{0: x+1, 1: x+2}};
  assign n_rep = '{tag: x, a: '{2{x+3}}};
  assign n_ord = '{tag: x, a: '{x+4, x+5}};
  assign r_ord = '{tag: x, a: '{x+4, x+5}};
  assign r_def = '{tag: x, a: '{default: x}};
  assign r_two = '{tag: x, a: '{x, x+1}};
  assign r_two = '{tag: 1, a: '{2, 3}};
  initial begin
    #1 x = 7;
    #1;
    $display("n_def %0d %0d %0d", n_def.tag, n_def.a[0], n_def.a[1]);
    $display("n_keys %0d %0d %0d", n_keys.tag, n_keys.a[0], n_keys.a[1]);
    $display("n_rep %0d %0d %0d", n_rep.tag, n_rep.a[0], n_rep.a[1]);
    $display("n_ord %0d %0d %0d", n_ord.tag, n_ord.a[0], n_ord.a[1]);
    $display("r_ord %0d %0d %0d", r_ord.tag, r_ord.a[0], r_ord.a[1]);
    $display("r_def %0d %0d %0d", r_def.tag, r_def.a[0], r_def.a[1]);
    $display("r_two %0d %0d %0d", r_two.tag, r_two.a[0], r_two.a[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "n_def 7 7 7",
            "n_keys 7 8 9",
            "n_rep 7 10 10",
            "n_ord 7 11 12",
            "r_ord 7 11 12",
            "r_def 7 7 7",
            "r_two 8 9 11",
        ]
    );
}

/// Packed members a `default:` reaches: a packed structure gets the value
/// per member and a multidimensional packed array per innermost element
/// (§10.9.2: neither is a simple bit vector type, §6.11.1). The reference
/// simulator assigns the whole `logic [1:0][3:0]` member from a
/// structure-level `default:` (`0a`); this follows the LRM (`aa`), as the
/// member-target assignment `v.m` does on both.
#[test]
fn default_reaches_packed_members() {
    let src = r#"
`timescale 1ns/1ps
typedef struct packed { logic [1:0] a; logic [1:0] b; } ps_t;
typedef struct { int t; ps_t p; logic [1:0][3:0] m; } S;
typedef struct { int t; ps_t pa[2]; } SA;
module top;
  logic [3:0] x;
  S v, w;
  SA sa;
  assign v.p = '{default: x[1:0]};
  assign v.m = '{default: x};
  assign v.t = 5;
  assign w = '{default: x};
  assign sa = '{t: 1, pa: '{default: x[1:0]}};
  initial begin
    #1 x = 4'b1010;
    #1 $display("v %0d %b %h", v.t, v.p, v.m);
    $display("w %0d %b %h", w.t, w.p, w.m);
    $display("sa %0d %b %b", sa.t, sa.pa[0], sa.pa[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        ["v 5 1010 aa", "w 10 1010 aa", "sa 1 1010 1010"]
    );
}

/// A child module's output port, a parameter of the child as an index key,
/// and an array variable as the member's item.
#[test]
fn child_ports_parameter_keys_and_array_items() {
    let src = r#"
`timescale 1ns/1ps
typedef struct { int tag; int a[2]; } S;
typedef struct { int tag; int a[0:2]; } S3;
module child #(parameter int K = 1) (input int i, output S o, output S3 p);
  assign o = '{tag: i, a: '{default: i + K}};
  assign p = '{tag: i, a: '{K: i + 10, default: i}};
endmodule
module top;
  int x;
  int arr[2];
  S c_o, s_arr;
  S3 c_p;
  child #(.K(2)) u(.i(x), .o(c_o), .p(c_p));
  assign s_arr = '{tag: x, a: arr};
  initial begin
    arr[0] = 4; arr[1] = 5;
    #1 x = 7;
    #1;
    $display("c_o %0d %0d %0d", c_o.tag, c_o.a[0], c_o.a[1]);
    $display("c_p %0d %0d %0d %0d", c_p.tag, c_p.a[0], c_p.a[1], c_p.a[2]);
    $display("s_arr %0d %0d %0d", s_arr.tag, s_arr.a[0], s_arr.a[1]);
    arr[1] = 9;
    #1 $display("s_arr %0d %0d %0d", s_arr.tag, s_arr.a[0], s_arr.a[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        ["c_o 7 9 9", "c_p 7 7 7 17", "s_arr 7 4 5", "s_arr 7 4 9"]
    );
}
