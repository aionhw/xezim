//! IEEE 1800-2023 §10.9.1 / §10.9.2: type keys in array assignment patterns
//! for resolved-nettype structure members, plain variables, continuous
//! assignments, multidimensional arrays (with outer type keys taking precedence
//! over nested `default:` patterns), typedefs, package-scoped types, and module
//! / class type parameters (issue #1290).
//!
//! The values match the reference simulator, except where a type key of an
//! outer level sits above a `default:` row or structure pattern (`CA2`, `V2`,
//! `SE`, `PARAM_U2`, `CLS_INIT`, `CLS_WHOLE`, `CLS_U2`). §10.9.1 descends into
//! each sub-array or structure with the type and default keys, so the type
//! key sets every element of its type; the reference simulator gives those
//! elements the `default:` pattern's value instead.

fn lines(src: &str) -> Vec<String> {
    let sim = xezim::simulate(src, 100_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("Simulation"))
        .collect()
}

#[test]
fn nettype_member_type_key_and_precedence() {
    let src = r#"
`timescale 1ns/1ps
package pkg1290;
  typedef integer word_t;
endpackage

module top;
  typedef integer my_int;
  typedef struct { integer a[1]; } S1;
  typedef struct { integer a[2]; } S2;
  typedef struct { integer m[2][2]; } SM;

  function automatic S1 resolve_s1(input S1 drivers[]);
    S1 result;
    result.a[0] = drivers[0].a[0];
    return result;
  endfunction

  function automatic S2 resolve_s2(input S2 drivers[]);
    S2 result;
    result.a[0] = drivers[0].a[0];
    result.a[1] = drivers[0].a[1];
    return result;
  endfunction

  function automatic SM resolve_sm(input SM drivers[]);
    $display("RSM=%0d,%0d,%0d,%0d",
      drivers[0].m[0][0], drivers[0].m[0][1],
      drivers[0].m[1][0], drivers[0].m[1][1]);
    return drivers[0];
  endfunction

  nettype S1 N1 with resolve_s1;
  nettype S2 N2 with resolve_s2;
  nettype SM NM with resolve_sm;

  N1 n_builtin, n_tdef, n_pkg;
  N2 n_idx_first, n_type_first, n_nonmatch, n_multi;
  NM nm;
  int x;

  assign n_builtin    = '{a:'{integer:x, default:99}};
  assign n_tdef       = '{a:'{my_int:x+1, default:99}};
  assign n_pkg        = '{a:'{pkg1290::word_t:x+2, default:99}};
  assign n_idx_first  = '{a:'{0:x, integer:8, default:99}};
  assign n_type_first = '{a:'{integer:8, 0:x, default:99}};
  assign n_nonmatch   = '{a:'{int:8, default:99}};
  assign n_multi      = '{a:'{integer:11, my_int:22, default:99}};
  assign nm           = '{m:'{integer:2, 0:'{integer:1, default:3}, default:'{default:2}}};

  initial begin
    x = 7;
    #1;
    $display("N1=%0d,%0d,%0d", n_builtin.a[0], n_tdef.a[0], n_pkg.a[0]);
    $display("N2=%0d,%0d|%0d,%0d|%0d,%0d|%0d,%0d",
      n_idx_first.a[0], n_idx_first.a[1],
      n_type_first.a[0], n_type_first.a[1],
      n_nonmatch.a[0], n_nonmatch.a[1],
      n_multi.a[0], n_multi.a[1]);
    $finish;
  end
endmodule
"#;
    let out = lines(src);
    assert!(out.contains(&"N1=7,8,9".to_string()), "{out:?}");
    assert!(
        out.contains(&"N2=7,8|7,8|99,99|22,22".to_string()),
        "{out:?}"
    );
    assert!(out.contains(&"RSM=1,1,2,2".to_string()), "{out:?}");
}

#[test]
fn multidim_array_and_struct_member_outer_type_keys() {
    let src = r#"
`timescale 1ns/1ps
package pkg1290;
  typedef integer word_t;
endpackage

module top;
  typedef integer row_t[2];
  typedef struct { integer m[2][2]; } SM;
  typedef struct { integer a; byte b; } E;
  typedef struct { E arr[2][2]; } SE;

  SM ca_sm1, ca_sm2, ca_sm3;
  SM v1, v2, v3, v_flat;
  SE se;
  integer plain2d[2][2];
  row_t rm[2];

  assign ca_sm1 = '{m:'{integer:2, 0:'{integer:1, default:3}, default:'{default:2}}};
  assign ca_sm2 = '{m:'{integer:2, default:'{integer:1, default:4}}};
  assign ca_sm3 = '{m:'{byte:2, default:'{integer:1, default:4}}};

  initial begin
    v1       = '{m:'{integer:2, 0:'{integer:1, default:3}, default:'{default:2}}};
    v2       = '{m:'{integer:2, default:'{integer:1, default:4}}};
    v3       = '{m:'{byte:2, default:'{integer:1, default:4}}};
    v_flat.m = '{integer:2, 0:'{integer:1, default:3}, default:'{default:2}};
    se       = '{integer:7, default:'{a:5, b:8'd6}};
    plain2d  = '{integer:2, 0:'{integer:1, default:3}, default:'{default:2}};
    rm       = '{row_t:'{5, 6}, default:'{0, 0}};
    #1;
    $display("CA1=%0d,%0d,%0d,%0d", ca_sm1.m[0][0], ca_sm1.m[0][1], ca_sm1.m[1][0], ca_sm1.m[1][1]);
    $display("CA2=%0d,%0d,%0d,%0d", ca_sm2.m[0][0], ca_sm2.m[0][1], ca_sm2.m[1][0], ca_sm2.m[1][1]);
    $display("CA3=%0d,%0d,%0d,%0d", ca_sm3.m[0][0], ca_sm3.m[0][1], ca_sm3.m[1][0], ca_sm3.m[1][1]);
    $display("V1=%0d,%0d,%0d,%0d", v1.m[0][0], v1.m[0][1], v1.m[1][0], v1.m[1][1]);
    $display("V2=%0d,%0d,%0d,%0d", v2.m[0][0], v2.m[0][1], v2.m[1][0], v2.m[1][1]);
    $display("V3=%0d,%0d,%0d,%0d", v3.m[0][0], v3.m[0][1], v3.m[1][0], v3.m[1][1]);
    $display("VF=%0d,%0d,%0d,%0d", v_flat.m[0][0], v_flat.m[0][1], v_flat.m[1][0], v_flat.m[1][1]);
    $display("SE=%0d,%0d|%0d,%0d", se.arr[0][0].a, se.arr[0][0].b, se.arr[1][1].a, se.arr[1][1].b);
    $display("P2=%0d,%0d,%0d,%0d", plain2d[0][0], plain2d[0][1], plain2d[1][0], plain2d[1][1]);
    $display("RM=%0d,%0d,%0d,%0d", rm[0][0], rm[0][1], rm[1][0], rm[1][1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "CA1=1,1,2,2",
            "CA2=2,2,2,2",
            "CA3=1,1,1,1",
            "V1=1,1,2,2",
            "V2=2,2,2,2",
            "V3=1,1,1,1",
            "VF=1,1,2,2",
            "SE=7,6|7,6",
            "P2=1,1,2,2",
            "RM=5,6,5,6",
        ]
    );
}

#[test]
fn module_and_class_type_parameter_keys() {
    let src = r#"
`timescale 1ns/1ps
module sub #(type T = integer) (
  output integer p0, output integer p1,
  output integer c0, output integer c1
);
  typedef integer local_t;
  typedef struct { integer a[2]; } S;
  S vp, vc, vl;
  integer y;
  assign vc = '{a:'{T:y, default:99}};
  initial begin
    y = 42;
    #1;
    vp = '{a:'{T:y, default:99}};
    vl = '{a:'{local_t:y+1, default:99}};
    p0 = vp.a[0]; p1 = vp.a[1];
    c0 = vc.a[0]; c1 = vc.a[1];
    $display("LOC=%0d,%0d", vl.a[0], vl.a[1]);
  end
endmodule

class cls #(type T = integer);
  typedef struct { integer a[2]; } S;
  function void run(output integer o0, output integer o1);
    S v;
    v = '{a:'{T:77, default:99}};
    o0 = v.a[0];
    o1 = v.a[1];
  endfunction
endclass

module top;
  integer ip0, ip1, ic0, ic1;
  integer bp0, bp1, bc0, bc1;
  integer co0, co1, cbo0, cbo1;
  sub #(integer) u_int(ip0, ip1, ic0, ic1);
  sub #(byte)    u_byte(bp0, bp1, bc0, bc1);
  cls #(integer) ci;
  cls #(byte)    cb;
  initial begin
    #2;
    ci = new(); ci.run(co0, co1);
    cb = new(); cb.run(cbo0, cbo1);
    $display("SUB_INT=%0d,%0d|%0d,%0d", ip0, ip1, ic0, ic1);
    $display("SUB_BYTE=%0d,%0d|%0d,%0d", bp0, bp1, bc0, bc1);
    $display("CLS=%0d,%0d|%0d,%0d", co0, co1, cbo0, cbo1);
    $finish;
  end
endmodule
"#;
    let out = lines(src);
    assert!(out.contains(&"SUB_INT=42,42|42,42".to_string()), "{out:?}");
    assert!(out.contains(&"SUB_BYTE=99,99|99,99".to_string()), "{out:?}");
    assert!(out.contains(&"CLS=77,77|99,99".to_string()), "{out:?}");
    assert!(
        out.iter().filter(|l| *l == "LOC=43,43").count() == 2,
        "{out:?}"
    );
}

#[test]
fn packed_typedef_enum_and_param_pattern_type_keys() {
    let src = r#"
package pk_pat;
  typedef bit signed [31:0] w_t;
  typedef bit [7:0] u8_t;
  typedef integer i_t;
  typedef enum logic [1:0] { E0 = 2'd0, E1 = 2'd1 } e_t;
  typedef struct { i_t a; byte b; } cs_t;
  typedef struct packed { integer a; logic [7:0] b; } ps_t;
  typedef struct { w_t [1:0][1:0] p2d; } sp_t;
  parameter u8_t [1:0] PK_B = '{pk_pat::u8_t: 8'hC3};
endpackage

module top;
  import pk_pat::*;

  w_t [1:0][1:0] p2d;
  w_t [1:0] p1d;
  e_t [1:0] pe;
  sp_t sp;

  localparam w_t [1:0] P_PK = '{w_t: 32'sd11, default: 32'sd22};
  localparam u8_t [1:0] P_U8 = '{u8_t: 8'hAA, default: 8'h55};
  localparam integer P_U1[2] = '{integer: 7, default: 9};
  localparam integer P_U2[2][2] = '{integer: 2, default: '{integer: 1, default: 2}};
  localparam cs_t P_CS = '{i_t: 10, byte: 8'sd3};
  localparam ps_t P_PS = '{integer: 12, default: 8'h34};
  localparam ps_t [1:0] P_PSA = '{ps_t: '{integer: 4, default: 8'h05}, default: '{0, 8'h00}};

  initial begin
    p2d = '{w_t: 32'sd2, default: 32'sd1};
    $display("P2D=%0d,%0d,%0d,%0d", p2d[1][1], p2d[1][0], p2d[0][1], p2d[0][0]);
    p2d[0] = '{w_t: 32'sd5, default: 32'sd6};
    $display("P2D_SL=%0d,%0d", p2d[0][1], p2d[0][0]);
    p1d = '{w_t: 32'sd7, default: 32'sd8};
    $display("P1D=%0d,%0d", p1d[1], p1d[0]);
    pe = '{e_t: E1, default: E0};
    $display("PE=%0d,%0d", pe[1], pe[0]);
    sp.p2d[0] = '{w_t: 32'sd33, default: 32'sd44};
    $display("MEM_SL=%0d,%0d", sp.p2d[0][1], sp.p2d[0][0]);

    $display("PK_B=%02h,%02h", PK_B[1], PK_B[0]);
    $display("PARAM_PK=%0d,%0d", P_PK[1], P_PK[0]);
    $display("PARAM_U8=%02h,%02h", P_U8[1], P_U8[0]);
    $display("PARAM_U1=%0d,%0d", P_U1[0], P_U1[1]);
    $display("PARAM_U2=%0d,%0d,%0d,%0d", P_U2[0][0], P_U2[0][1], P_U2[1][0], P_U2[1][1]);
    $display("PARAM_CS=%0d,%0d", P_CS.a, P_CS.b);
    $display("PARAM_PS=%0d,%02h", P_PS.a, P_PS.b);
    $display("PARAM_PSA=%0d,%02h|%0d,%02h", P_PSA[1].a, P_PSA[1].b, P_PSA[0].a, P_PSA[0].b);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "P2D=2,2,2,2",
            "P2D_SL=5,5",
            "P1D=7,7",
            "PE=1,1",
            "MEM_SL=33,33",
            "PK_B=c3,c3",
            "PARAM_PK=11,11",
            "PARAM_U8=aa,aa",
            "PARAM_U1=7,7",
            "PARAM_U2=2,2,2,2",
            "PARAM_CS=10,3",
            "PARAM_PS=12,34",
            "PARAM_PSA=4,05|4,05",
        ]
    );
}

#[test]
fn class_formal_and_nba_pattern_type_keys() {
    let src = r#"
`timescale 1ns/1ps
module top;
  typedef integer i_t;
  typedef logic [15:0] u16_t;
  typedef struct { i_t a; byte b; } cs_t;
  typedef struct { integer m[2]; } sm1_t;
  typedef struct { u16_t u; byte b; } s16_t;

  class C;
    cs_t arr[2] = '{i_t: 55, default: '{a: 1, b: 8'd4}};
    sm1_t sm_arr[2];
    integer uarr[2][2];
  endclass

  function automatic integer f_arr(input integer x[2]);
    return x[0] * 10 + x[1];
  endfunction

  s16_t s16;
  logic [7:0] x8, y8;

  initial begin
    C c;
    c = new();
    $display("CLS_INIT=%0d,%0d|%0d,%0d", c.arr[0].a, c.arr[0].b, c.arr[1].a, c.arr[1].b);
    c.arr[0] = '{i_t: 11, i_t: 22, byte: 8'sd3};
    $display("CLS_ELEM=%0d,%0d", c.arr[0].a, c.arr[0].b);
    c.arr = '{0: '{a: 10, b: 8'd2}, i_t: 88, default: '{a: 0, b: 8'd9}};
    $display("CLS_WHOLE=%0d,%0d|%0d,%0d", c.arr[0].a, c.arr[0].b, c.arr[1].a, c.arr[1].b);
    c.sm_arr[0] = '{integer: 7, default: 8};
    $display("CLS_ARR_MEM=%0d,%0d", c.sm_arr[0].m[0], c.sm_arr[0].m[1]);
    c.uarr = '{integer: 2, default: '{integer: 1, default: 2}};
    $display("CLS_U2=%0d,%0d,%0d,%0d", c.uarr[0][0], c.uarr[0][1], c.uarr[1][0], c.uarr[1][1]);
    $display("F_ARR=%0d,%0d", f_arr('{integer: 7, default: 9}), f_arr('{0: 3, integer: 4, default: 9}));

    x8 = 8'hFF;
    y8 = 8'h01;
    s16 <= #1 '{u16_t: x8 + y8, default: 8'd3};
    #2;
    $display("NBA_CARRY=%04h,%0d", s16.u, s16.b);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        lines(src),
        [
            "CLS_INIT=55,4|55,4",
            "CLS_ELEM=22,3",
            "CLS_WHOLE=10,2|88,9",
            "CLS_ARR_MEM=7,7",
            "CLS_U2=2,2,2,2",
            "F_ARR=77,34",
            "NBA_CARRY=0100,3",
        ]
    );
}
