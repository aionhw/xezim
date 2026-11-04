//! §6.24.1/§13.4.3: a cast inside a constant function folds at elaboration,
//! so a package or module localparam computed by that function has the
//! function's value in every scope that reads it. Expected values from the
//! reference simulator.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn package_localparam_from_function_with_casts() {
    let msgs = messages(
        "package p;
  typedef enum logic [2:0] {North, East, South, West, Eject} dir_e;
  function automatic bit has_n(int unsigned r, dir_e d);
    case (d) North: return r < 2; East: return r % 2 == 0; default: return 1'b0; endcase
  endfunction
  function automatic int unsigned cnt_cast();
    int unsigned n = 0;
    for (int unsigned r = 0; r < 4; r++) for (int unsigned d = 0; d < 4; d++)
      if (has_n(r, dir_e'(d))) n++;
    return n;
  endfunction
  function automatic int unsigned c_intcast(int unsigned x); return int'(x) + 1; endfunction
  function automatic int unsigned c_enumcast(int unsigned x); return 32'(dir_e'(x)); endfunction
  localparam int unsigned A = cnt_cast();
  localparam int unsigned C = c_intcast(4);
  localparam int unsigned D = c_enumcast(2);
endpackage
module child; import p::*;
  initial $display(\"child A=%0d C=%0d D=%0d\", A, C, D);
endmodule
module tb; import p::*;
  child u();
  initial #1 $display(\"top A=%0d C=%0d D=%0d\", A, C, D);
endmodule",
    );
    assert_line(&msgs, "child A=4 C=5 D=2");
    assert_line(&msgs, "top A=4 C=5 D=2");
}

#[test]
fn cast_width_sign_and_typedef_chain() {
    let msgs = messages(
        "package q;
  typedef enum logic [2:0] {N0, N1, N2, N3, N4} d_e;
  typedef d_e d2_t;
  typedef logic [5:0] u6_t;
  typedef enum {X0, X1, X2} ie_t;
  function automatic int f_byte(int x); return byte'(x); endfunction
  function automatic int f_enum_wrap(int x); return 32'(d_e'(x)); endfunction
  function automatic int f_signed(int unsigned x); return (int'(x) < 0) ? 1 : 0; endfunction
  function automatic int f_chain(int x); return 32'(d2_t'(x)); endfunction
  function automatic int f_u6(int x); return u6_t'(x) + 1; endfunction
  function automatic int f_ie(int x); return ie_t'(x); endfunction
  function automatic int f_shortint(int x); return shortint'(x) >>> 4; endfunction
  localparam int A1 = f_byte(300);
  localparam int A2 = f_enum_wrap(9);
  localparam int A3 = f_signed(32'hffff_ffff);
  localparam int A4 = f_chain(10);
  localparam int A5 = f_u6(70);
  localparam int A6 = f_ie(2);
  localparam int A7 = f_shortint(32'h0000_8000);
endpackage
module child;
  function automatic int f_scoped(int x); return 32'(q::d_e'(x)); endfunction
  localparam int B = f_scoped(13);
  initial $display(\"child B=%0d\", B);
endmodule
module tb; import q::*;
  child u();
  initial $display(\"pkg A1=%0d A2=%0d A3=%0d A4=%0d A5=%0d A6=%0d A7=%0d\", A1, A2, A3, A4, A5, A6, A7);
endmodule",
    );
    assert_line(&msgs, "pkg A1=44 A2=1 A3=1 A4=2 A5=7 A6=2 A7=-2048");
    assert_line(&msgs, "child B=5");
}

#[test]
fn package_function_called_from_instance_uses_package_typedefs() {
    let msgs = messages(
        "package a; typedef enum logic [2:0] {N0, N1, N2, N3, N4} d_e; endpackage
package x; typedef logic [1:0] d_e; endpackage
package b;
  import a::*;
  typedef logic [4:0] w_t;
  function automatic bit has_n(int unsigned r, d_e d);
    case (d) N0: return r < 2; N1: return r % 2 == 0; default: return 1'b0; endcase
  endfunction
  function automatic int first_dir(int unsigned r);
    for (int d = 0; d < 5; d++) if (has_n(r, d_e'(d))) return d;
    return -1;
  endfunction
  function automatic int f_w(int x); return w_t'(x); endfunction
  function automatic int f_imp(int x); return 32'(d_e'(x)); endfunction
endpackage
module child #(parameter int W = 9);
  typedef logic [7:0] w_t;
  localparam int C2 = b::first_dir(W - 7);
  localparam int C3 = b::f_w(37);
  localparam int C4 = b::f_imp(13);
  initial $display(\"child C2=%0d C3=%0d C4=%0d\", C2, C3, C4);
endmodule
module tb; child u(); endmodule",
    );
    assert_line(&msgs, "child C2=1 C3=5 C4=5");
}

#[test]
fn early_return_formal_conversion_and_unit_typedef() {
    let msgs = messages(
        "typedef enum logic [1:0] {U0, U1, U2} u_e;
package q;
  typedef enum bit signed [3:0] {NEG = -3, POS = 5} s_e;
  typedef logic [0:5] asc_t;
  function automatic int narrow3(logic [2:0] x); return x; endfunction
  function automatic int f_formal(int x); return narrow3(int'(x)); endfunction
  function automatic logic [3:0] ret4(int x); return int'(x); endfunction
  function automatic int sform(shortint s); return s; endfunction
  function automatic int f_sform(int x); return sform(shortint'(x)); endfunction
  function automatic int f_asc(int x); return asc_t'(x); endfunction
  function automatic int f_senum(int x); return int'(s_e'(x)); endfunction
  function automatic int f_unit(int x); return 32'(u_e'(x)); endfunction
  function automatic int f_nested(int x); return x > 3 ? narrow3(3'(byte'(x))) : int'(narrow3(x)); endfunction
  localparam int P2 = f_formal(13);
  localparam int P3 = ret4(37);
  localparam int P4 = f_sform(32'h18000);
  localparam int P6 = f_asc(70);
  localparam int P7 = f_senum(13);
  localparam int P8 = f_unit(6);
  localparam int P9 = f_nested(300);
endpackage
module tb; import q::*;
  initial $display(\"P2=%0d P3=%0d P4=%0d P6=%0d P7=%0d P8=%0d P9=%0d\", P2, P3, P4, P6, P7, P8, P9);
endmodule",
    );
    assert_line(&msgs, "P2=5 P3=5 P4=-32768 P6=6 P7=-3 P8=2 P9=4");
}

#[test]
fn instance_typedefs_type_params_and_imports() {
    let msgs = messages(
        "package pk;
  typedef logic [3:0] n_t;
  typedef enum logic [2:0] {X0, X1, X2} k_e;
endpackage
typedef logic [7:0] w_t;
module grand #(parameter int G = 3) ();
  typedef logic [G-1:0] g_t;
  function automatic int fg(int x); return g_t'(x); endfunction
  localparam int CG = fg(13);
  initial #1 $display(\"%m CG=%0d\", CG);
endmodule
module child #(parameter int W = 5, parameter type T = logic [3:0]) ();
  import pk::*;
  typedef logic [W-1:0] w_t;
  typedef w_t v_t;
  typedef pk::n_t a_t;
  typedef enum logic [1:0] {A, B, C, D} e_t;
  typedef logic signed [3:0] s_t;
  function automatic int fw(int x); return w_t'(x); endfunction
  function automatic int fv(int x); return v_t'(x); endfunction
  function automatic int fe(int x); return 32'(e_t'(x)); endfunction
  function automatic int fs(int x); return int'(s_t'(x) < 0); endfunction
  function automatic int ft(int x); return T'(x); endfunction
  function automatic int fi(int x); return n_t'(x); endfunction
  function automatic int fa(int x); return a_t'(x); endfunction
  function automatic int fk(int x); return 32'(k_e'(x)); endfunction
  function automatic int floop(); int n = 0;
    for (int i = 0; i < 8; i++) if (e_t'(i) == B) n++;
    return n; endfunction
  localparam int CW = fw(37);
  localparam int CV = fv(37);
  localparam int CE = fe(6);
  localparam int CS = fs(12);
  localparam int CT = ft(45);
  localparam int CI = fi(37);
  localparam int CA = fa(38);
  localparam int CK = fk(13);
  localparam int CL = floop();
  logic [CW-1:0] sized;
  grand #(.G(W - 2)) gr ();
  initial #1 $display(\"%m CW=%0d CV=%0d CE=%0d CS=%0d CT=%0d CI=%0d CA=%0d CK=%0d CL=%0d bits=%0d\",
                      CW, CV, CE, CS, CT, CI, CA, CK, CL, $bits(sized));
endmodule
module tb;
  child c5 ();
  child #(.W(6), .T(logic [2:0])) c6 ();
endmodule",
    );
    assert_line(&msgs, "tb.c5 CW=5 CV=5 CE=2 CS=1 CT=13 CI=5 CA=6 CK=5 CL=2 bits=5");
    assert_line(&msgs, "tb.c6 CW=37 CV=37 CE=2 CS=1 CT=5 CI=5 CA=6 CK=5 CL=2 bits=37");
    assert_line(&msgs, "tb.c5.gr CG=5");
    assert_line(&msgs, "tb.c6.gr CG=13");
}
