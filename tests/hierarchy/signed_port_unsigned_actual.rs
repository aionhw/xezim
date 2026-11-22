//! §23.3.3.7, §11.8.1: an input port's own declaration decides whether the
//! module body reads it as signed; the connection converts the actual to the
//! port's type. Elaboration renames an input port to a simple actual (a net,
//! a constant select, a concatenation), and the body then read the actual's
//! signedness: `input logic signed [12:0] e_i` connected to `{c[11], c}`
//! read 8165 instead of -27, so `a_i - e_i` and `e_i < 0` were unsigned, and
//! an unsigned port connected to a signed net read as signed. lowRISC
//! Muntjac's FPU passes such a concatenation as an exponent
//! (`muntjac_fpu_mul_add.sv`), and `fadd.s pi + 1e-8` returned 1e-8.
//!
//! The expected values are the reference simulator's; Icarus Verilog 13
//! agrees.

#[test]
fn input_port_reads_with_its_declared_signedness() {
    let src = r#"
module ks (input logic signed [12:0] e_i, input logic signed [12:0] a_i);
  wire signed [13:0] diff = a_i - e_i;
  initial #1 $display("%m e_i=%0d diff=%0d lt=%b", e_i, diff, e_i < 0);
endmodule
module kn (e_i, a_i);
  input signed [12:0] e_i;
  input signed [12:0] a_i;
  wire signed [13:0] diff = a_i - e_i;
  initial #1 $display("%m e_i=%0d diff=%0d lt=%b", e_i, diff, e_i < 0);
endmodule
module ku (input logic [12:0] e_i);
  initial #1 $display("%m e_i=%0d lt=%b gt=%b", e_i, e_i < 0, e_i > 13'd100);
endmodule
module top;
  logic signed [11:0] c = -12'sd27;
  logic signed [12:0] a = 13'sd1;
  logic signed [12:0] sv = -13'sd27;
  logic [12:0] uv = 13'h1fe5;
  ks s_concat (.e_i({c[11], c}), .a_i(a));
  ks s_repl (.e_i({{1{c[11]}}, c}), .a_i(a));
  ks s_unsigned (.e_i(uv), .a_i(a));
  ks s_part (.e_i(sv[12:0]), .a_i(a));
  ks s_signed (.e_i(sv), .a_i(a));
  kn n_concat (.e_i({c[11], c}), .a_i(a));
  ku u_signed (.e_i(sv));
  ku u_concat (.e_i({c[11], c}));
endmodule
"#;
    let sim = xezim::simulate(src, 100).expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_eq!(
        lines,
        [
            "top.s_concat e_i=-27 diff=28 lt=1",
            "top.s_repl e_i=-27 diff=28 lt=1",
            "top.s_unsigned e_i=-27 diff=28 lt=1",
            "top.s_part e_i=-27 diff=28 lt=1",
            "top.s_signed e_i=-27 diff=28 lt=1",
            "top.n_concat e_i=-27 diff=28 lt=1",
            "top.u_signed e_i=8165 lt=0 gt=1",
            "top.u_concat e_i=8165 lt=0 gt=1",
        ],
        "{lines:#?}"
    );
}

/// The same holds for a net driven by a whole-net continuous assignment:
/// `wire signed [12:0] ws; assign ws = uv;` reads signed. Sharing one
/// storage slot between such a net and its driver gave it the driver's
/// signedness.
#[test]
fn identity_assignment_keeps_the_net_signedness() {
    let src = r#"
module top;
  logic [12:0] uv = 13'h1fe5;
  logic signed [12:0] sv = -13'sd27;
  wire signed [12:0] ws;
  wire [12:0] wu;
  assign ws = uv;
  assign wu = sv;
  initial #1 $display("ws=%0d lt=%b wu=%0d lt=%b", ws, ws < 0, wu, wu < 0);
endmodule
"#;
    let sim = xezim::simulate(src, 100).expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_eq!(lines, ["ws=-27 lt=1 wu=8165 lt=0"], "{lines:#?}");
}

/// An element of an unpacked array and a struct member reach the port with
/// the port's signedness too: an unsigned port connected to a signed one
/// reads 8165, and a signed port connected to an unsigned one reads -27.
#[test]
fn element_and_member_actuals_read_with_the_port_signedness() {
    let src = r#"
module ks (input logic signed [12:0] e_i);
  initial #1 $display("%m e_i=%0d lt=%b", e_i, e_i < 0);
endmodule
module ku (input logic [12:0] e_i);
  initial #1 $display("%m e_i=%0d lt=%b gt=%b", e_i, e_i < 0, e_i > 13'd100);
endmodule
module top;
  typedef struct packed { logic signed [12:0] f; logic [2:0] g; } st_t;
  logic signed [12:0] sa [2] = '{-13'sd27, -13'sd27};
  logic [12:0] ua [2] = '{13'h1fe5, 13'h1fe5};
  st_t st = '{f: -13'sd27, g: 3'd0};
  ku u_elem (.e_i(sa[0]));
  ku u_member (.e_i(st.f));
  ks s_elem (.e_i(ua[1]));
endmodule
"#;
    let sim = xezim::simulate(src, 100).expect("simulate");
    let lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_eq!(
        lines,
        [
            "top.u_elem e_i=8165 lt=0 gt=1",
            "top.u_member e_i=8165 lt=0 gt=1",
            "top.s_elem e_i=-27 lt=1",
        ],
        "{lines:#?}"
    );
}
