//! An element of a packed array whose element type is a NAMED signed type
//! is signed wherever the array is declared (IEEE 1800-2023 §7.4.1,
//! §11.8.1): a widening cast or assignment of `lanes[1]` sign-extends, and
//! the element takes part in signed arithmetic and comparisons. The
//! declaration contexts below all read the element as unsigned before
//! (#1312): non-ANSI and ANSI ports, subroutine and class-method formals and
//! locals, block variables, structure members reached through nested
//! structures, packed arrays of structures, class properties and instance
//! names.
//!
//! Expected values are the reference simulator's, except for the selects of
//! the ports themselves (`port`, `ops`, `ansi self td/vec`, `hier td/vec/o`):
//! the reference reads those elements unsigned (129185), while a port of a
//! data type is an object of that type like any other (§23.2.2), so §7.4.1
//! applies and the element is -1887. A part-select and a concatenation of
//! the element stay unsigned (§11.8.1).

use xezim::simulate;

const SRC: &str = r#"
typedef logic signed [16:0] lane_t;
typedef logic signed [23:0] acc_t;
typedef struct packed { lane_t [0:1] m; logic [3:0] pad; } st_t;
typedef lane_t [0:1] lane_arr_t;
typedef enum logic signed [4:0] { E_N = -5'sd5, E_P = 5'sd3 } e_t;
typedef struct packed { st_t i; logic [1:0] q; } st2_t;
package gp;
  typedef logic signed [16:0] lane_t;
  typedef logic signed [23:0] acc_t;
  typedef lane_t [0:1] arr_t;
  function automatic acc_t pf(input arr_t a); return acc_t'(a[1]); endfunction
  task automatic pt(input lane_t [0:1] v, output acc_t r); r = acc_t'(v[1]); endtask
endpackage
class box;
  lane_t [0:1] arr;
  st_t st;
  st_t [0:1] sa;
  function new(input lane_t [0:1] v); arr[0] = v[0]; arr[1] = v[1]; endfunction
  function automatic acc_t mf(input lane_t [0:1] v); return acc_t'(v[1]); endfunction
  function automatic acc_t md(input lane_t [1:0] v); return acc_t'(v[0]); endfunction
  task automatic mt(input lane_t [0:1] v, output acc_t r); r = acc_t'(v[1]); endtask
  static function automatic acc_t sf(input lane_t [0:1] v); return acc_t'(v[1]); endfunction
  function automatic acc_t stf(input st_t s); return acc_t'(s.m[1]); endfunction
  function automatic acc_t wl(); lane_t [0:1] lv; lv[0] = 14304; lv[1] = -1887; return acc_t'(lv[1]); endfunction
endclass
module nonansi (lanes_in, echo_out, st_out, p_td, p_a, p_b, p_en, p_2d, r);
  input  lane_t [0:1] lanes_in;
  output lane_t [0:1] echo_out;
  output st_t         st_out;
  input  lane_arr_t   p_td;
  input  lane_t [0:1] p_a, p_b;
  input  e_t [0:1]    p_en;
  input  lane_t [0:1][0:1] p_2d;
  output acc_t r;
  assign echo_out = lanes_in;
  assign st_out = '{m: lanes_in, pad: 4'h0};
  assign r = acc_t'(lanes_in[1]);
  wire acc_t w24 = lanes_in[1];
  initial begin
    #2;
    $display("CHECK port in %0d out %0d st %0d td %0d b %0d en %0d 2d %0d ca %0d w %0d",
             acc_t'(lanes_in[1]), acc_t'(echo_out[1]), acc_t'(st_out.m[1]), acc_t'(p_td[1]),
             acc_t'(p_b[1]), acc_t'(p_en[1]), acc_t'(p_2d[1][1]), r, w24);
    $display("CHECK ops %0d %0d %0d %0d %0d %0d %0d",
             lanes_in[1] * 24'sd34, lanes_in[1] >>> 6, lanes_in[1] / 2, lanes_in[1] - lanes_in[0],
             lanes_in[1] < 0, lanes_in[1] inside {[-1900:-1000]}, (lanes_in[1] < 0) ? lanes_in[1] : lanes_in[0]);
    $display("CHECK unsigned %0d %0d %0d %s", acc_t'(lanes_in[1][16:0]), acc_t'({lanes_in[1]}),
             lanes_in[1][16], $sformatf("%0d", lanes_in[1]));
  end
endmodule
module ansi (input lane_arr_t a_td, input st_t a_st, input st_t [0:1] a_sa, input lane_t [0:1] a_vec, output acc_t o);
  assign o = acc_t'(a_td[1]);
  initial begin
    #2 $display("CHECK ansi self td %0d st %0d sa %0d vec %0d", acc_t'(a_td[1]), acc_t'(a_st.m[1]),
                acc_t'(a_sa[1].m[1]), acc_t'(a_vec[1]));
  end
endmodule
module top;
  lane_t [0:1] a; lane_t [1:0] d; st_t s; st_t [0:1] sa; st2_t nst; acc_t r, r2;
  wire st_t [0:1] w_sa; assign w_sa = sa;
  e_t [0:1] en; lane_t [0:1][0:1] a2;
  wire lane_t [0:1] echo; wire st_t sto; wire acc_t pr, ao;
  box o;
  nonansi u_n (a, echo, sto, a, a, a, en, a2, pr);
  ansi u_a (a, s, sa, a, ao);
  function automatic acc_t widen(input lane_t [0:1] v); return acc_t'(v[1]); endfunction
  function automatic acc_t fwd(input lane_t [0:1] v); return widen(v); endfunction
  function automatic acc_t fstruct(input st_t x); return acc_t'(x.m[1]); endfunction
  function automatic acc_t f2d(input lane_t [0:1][0:1] v); return acc_t'(v[1][1]); endfunction
  function automatic acc_t flocal(); lane_t [0:1] lv; lv = a; return acc_t'(lv[1]); endfunction
  function automatic acc_t fstatic(); static lane_t [0:1] sv; sv = a; return acc_t'(sv[1]); endfunction
  function automatic acc_t fdesc(input lane_t [1:0] v); return acc_t'(v[0]); endfunction
  task automatic tk(input lane_t [0:1] v, output acc_t x); x = acc_t'(v[1]); endtask
  logic clk = 0; acc_t ffr;
  always_ff @(posedge clk) begin
    lane_t [0:1] fv; fv = a; ffr <= acc_t'(fv[1]);
  end
  initial begin
    a[0] = 14304; a[1] = -1887; d[1] = 14304; d[0] = -1887;
    en[0] = E_P; en[1] = E_N; a2[1] = a; a2[0] = a;
    s.m = a; s.pad = 0; sa[0] = s; sa[1] = s; nst.i = s; nst.q = 0;
    o = new(a); o.st = s; o.sa[1] = s;
    #1 clk = 1;
    #2;
    $display("CHECK hier td %0d st %0d sa %0d vec %0d o %0d", acc_t'(u_a.a_td[1]), acc_t'(u_a.a_st.m[1]),
             acc_t'(u_a.a_sa[1].m[1]), acc_t'(u_a.a_vec[1]), ao);
    $display("CHECK var st %0d sa %0d net %0d nst %0d desc %0d", acc_t'(s.m[1]), acc_t'(sa[1].m[1]),
             acc_t'(w_sa[1].m[1]), acc_t'(nst.i.m[1]), acc_t'(d[0]));
    $display("CHECK fn %0d %0d %0d %0d %0d %0d %0d", widen(a), fwd(a), fstruct(s), f2d(a2), flocal(), fstatic(), fdesc(d));
    tk(a, r); gp::pt(a, r2);
    $display("CHECK task %0d pkg %0d %0d", r, gp::pf(a), r2);
    $display("CHECK cls prop %0d st %0d sa %0d", acc_t'(o.arr[1]), acc_t'(o.st.m[1]), acc_t'(o.sa[1].m[1]));
    o.mt(a, r);
    $display("CHECK cls mf %0d md %0d mt %0d sf %0d stf %0d wl %0d", o.mf(a), o.md(d), r, box::sf(a), o.stf(s), o.wl());
    begin
      lane_t [0:1] bv; bv = a; r = acc_t'(bv[1]); $display("CHECK blk %0d", r);
    end
    begin : named
      lane_t [0:1] nv; nv = a; r = acc_t'(nv[1]); $display("CHECK named %0d", r);
    end
    for (int i = 0; i < 1; i++) begin
      lane_t [0:1] lv; lv = a; r = acc_t'(lv[1]); $display("CHECK for %0d", r);
    end
    fork begin
      lane_t [0:1] kv; kv = a; r = acc_t'(kv[1]); $display("CHECK fork %0d", r);
    end join
    case (1) 1: begin lane_t [0:1] cv; cv = a; r = acc_t'(cv[1]); $display("CHECK case %0d", r); end endcase
    $display("CHECK ff %0d", ffr);
    $finish;
  end
endmodule
"#;

#[test]
fn signed_named_elements_keep_their_sign_in_every_declaration_context() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    let actual: Vec<_> = sim
        .output
        .iter()
        .filter_map(|line| line.message.trim().strip_prefix("CHECK "))
        .collect();
    let expected = [
        "port in -1887 out -1887 st -1887 td -1887 b -1887 en -5 2d -1887 ca -1887 w -1887",
        "ops -64158 -30 -943 -16191 1 1 -1887",
        "unsigned 129185 129185 1 -1887",
        "ansi self td -1887 st -1887 sa -1887 vec -1887",
        "hier td -1887 st -1887 sa -1887 vec -1887 o -1887",
        "var st -1887 sa -1887 net -1887 nst -1887 desc -1887",
        "fn -1887 -1887 -1887 -1887 -1887 -1887 -1887",
        "task -1887 pkg -1887 -1887",
        "cls prop -1887 st -1887 sa -1887",
        "cls mf -1887 md -1887 mt -1887 sf -1887 stf -1887 wl -1887",
        "blk -1887",
        "named -1887",
        "for -1887",
        "fork -1887",
        "case -1887",
        "ff -1887",
    ];
    assert_eq!(actual, expected);
}

#[test]
fn selected_port_lane_keeps_its_type_in_cast_and_formatting() {
    let sim = simulate(
        r#"
typedef logic signed [16:0] lane_t;
typedef struct packed { lane_t [0:1] m; } s_t;
module child(lanes, data, extended);
  input lane_t [0:1] lanes;
  input s_t [0:1] data;
  output wire signed [31:0] extended;
  assign extended = int'(data[1].m[1]);
  initial #1 $display("lane=%0p type=%s", lanes[1], $typename(lanes));
endmodule
module top;
  lane_t [0:1] a;
  s_t [0:1] data;
  wire signed [31:0] ext;
  child u(a,data,ext);
  initial begin a='0; data='0; a[1]=-1887; data[1].m[1]=-1887;
    #2 $display("extended=%0d", ext);
  end
endmodule"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert!(
        lines.contains(&"lane=-1887 type=reg signed[16:0][0:1]"),
        "{lines:?}"
    );
    assert!(lines.contains(&"extended=-1887"), "{lines:?}");
}
