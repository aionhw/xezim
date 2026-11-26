//! Part 2 performance & memory-architecture semantics integration tests:
//! - Single-buffer Wide2State / Wide4State (>64b, >128b, >256b) operations
//!   across both AST evaluation and compiled bytecode execution
//! - Wide 4-state `~Z == X`, `>>>` (0/1/X/Z MSB + overshift), `casez`/`casex`/`===`,
//!   `real'()` wide conversion, `<` unequal signed comparison, and 2-state X/Z drop
//! - Word-parallel `Value::concat_refs` and `reverse_slices` streaming / concat / repl
//! - Multi-instance struct-member collection typedef sharing, 2D/ND 2-state and
//!   `tri0`/`tri1`/wire default init, overlapping select net drivers, and strengths
//!
//! Verified byte-for-byte against reference simulators.

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 500).unwrap_or_else(|e| panic!("simulation failed: {e}\n{src}"));
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn wide_ops_streaming_and_elaboration_parity() {
    let src = r#"
typedef bit [7:0] byte_q_t [$];

module sub_cell;
  typedef struct {
    byte_q_t q;
    int id;
  } cell_s_t;
  cell_s_t st;
  initial begin
    st.q.push_back(8'hA5);
    st.q.push_back(8'h5A);
    st.id = 7;
  end
endmodule

module top;
  sub_cell u0();
  sub_cell u1();

  bit [15:0] arr2d_2s [2][2];
  logic [7:0] arr2d_4s [2][2];
  tri0 [7:0] t0_2d [2][2];
  tri1 [7:0] t1_2d [2][2];
  wire [7:0] w_3d [2][2][2];

  wire [15:0] ov_w;
  assign ov_w[7:0]  = 8'hF0;
  assign ov_w[11:4] = 8'h00;

  wire s_win;
  assign (strong0, pull1) s_win = 1'b0;
  assign (weak0, weak1)   s_win = 1'b1;

  wand wd_tie;
  assign (weak0, weak1) wd_tie = 1'b1;
  assign (weak0, weak1) wd_tie = 1'b0;

  wor wo_tie;
  assign (weak0, weak1) wo_tie = 1'b1;
  assign (weak0, weak1) wo_tie = 1'b0;

  function automatic logic [149:0] wide_step(
    input logic [149:0] x,
    input logic [149:0] y
  );
    logic [149:0] nx;
    nx = ~y;
    return (x + nx) ^ {3{50'h1_2345_6789_ABCD}};
  endfunction

  function automatic int check_casex_wide(
    input logic signed [7:0] a,
    input logic signed [149:0] b
  );
    casex (b)
      a: return 1;
      default: return 0;
    endcase
  endfunction

  initial begin
    logic [299:0] w4, w4_not;
    bit [299:0] w2;
    logic signed [149:0] sp, sn, sx, sz;
    logic signed [149:0] sh0, sh1, shx, shz, sho;
    real r_pos, r_neg;
    logic [149:0] loop_acc;
    bit [7:0] d_arr [3:0];
    bit [7:0] dyn [];
    byte q [$];
    bit [7:0] hdr;
    bit [31:0] pk32;
    bit [15:0] pk16;
    bit [11:0] s5;
    logic [159:0] w_src, w_rev;
    int cx_eq, cz_eq, ce_eq;
    bit signed [7:0] s8;
    logic signed [199:0] l200;

    #1;
    $display("SUB:%02h:%02h:%0d:%0d", u0.st.q[0], u1.st.q[1], u0.st.id, u1.st.id);
    $display("INIT:%04h:%02h:%02h:%02h:%02h", arr2d_2s[1][1], arr2d_4s[1][1], t0_2d[1][0], t1_2d[0][1], w_3d[1][1][1]);
    $display("NETS:%04h:%b:%b:%b", ov_w, s_win, wd_tie, wo_tie);

    w4 = 300'h0;
    w4[130] = 1'bz;
    w4[5] = 1'b1;
    w4_not = ~w4;
    w2 = w4;
    $display("WIDE_NOT_2S:%b:%b:%b:%b", (w4_not[130] === 1'bx), (w4_not[5] === 1'b0), (w2[130] === 1'b0), (w2[5] === 1'b1));

    sp = 150'sh0000_0000_0000_0000_0000_0000_0000_0000_0123;
    sn = -150'sd256;
    sx = {1'bx, 149'b1010};
    sz = {1'bz, 149'b1010};
    sh0 = sp >>> 4;
    sh1 = sn >>> 4;
    shx = sx >>> 4;
    shz = sz >>> 4;
    sho = sz >>> 200;
    $display("ASHR:%0h:%0d:%b:%b:%b", sh0, sh1, (shx[149:146] === 4'bxxxx), (shz[149:146] === 4'bzzzz), (sho === {150{1'bz}}));

    w_src = (160'd1 << 100) + (160'd1 << 60);
    r_pos = real'(w_src);
    r_neg = real'(-150'sd1000000000000000000);
    $display("REAL:%016h:%0d", $realtobits(r_pos), (r_neg == -1.0e18));

    s8 = -8'sd1;
    l200 = {200{1'b1}};
    $display("CMP:%0d:%0d:%0d", (s8 < 200'd5), ($unsigned(s8) < l200), (s8 === l200));

    loop_acc = 150'h1234_5678;
    for (int k = 0; k < 8; k++) begin
      loop_acc = wide_step(loop_acc, {150{k[0]}});
    end
    $display("LOOP:%038h", loop_acc);

    cx_eq = check_casex_wide(8'h8x, {{142{1'bx}}, 8'h81});
    cz_eq = ({1'bz, 149'h15} === {1'bx, 149'h15});
    ce_eq = 0;
    casez ({1'bz, 149'h15})
      {1'b0, 149'h15}: ce_eq = 1;
      default: ce_eq = 0;
    endcase
    $display("CASE:%0d:%0d:%0d", cx_eq, cz_eq, ce_eq);

    d_arr = '{8'h11, 8'h22, 8'h33, 8'h44};
    pk32 = {>>8{d_arr}};
    $display("D_PK:%08h", pk32);
    {>>8{d_arr}} = 32'hAABBCCDD;
    $display("D_UNPK:%02h:%02h:%02h:%02h", d_arr[3], d_arr[2], d_arr[1], d_arr[0]);
    {>>8{d_arr with [2:1]}} = 16'hCAFE;
    $display("D_WITH:%02h:%02h:%02h:%02h", d_arr[3], d_arr[2], d_arr[1], d_arr[0]);

    q = {8'h01, 8'h02, 8'h03, 8'h04};
    {>>8{hdr, q}} = 40'hFF_10203040;
    $display("Q_UNPK:%02h:%0d:%02h:%02h", hdr, q.size(), q[0], q[3]);
    pk32 = {<<8{32'h12345678}};
    s5 = {<<5{12'hABC}};
    w_rev = {<<8{160'h0102030405060708090A0B0C0D0E0F1011121314}};
    $display("STR:%08h:%03h:%040h", pk32, s5, w_rev);
    dyn = new[3];
    dyn[0] = 8'h55; dyn[1] = 8'h66; dyn[2] = 8'h77;
    pk16 = {>>8{dyn with [0 +: 2]}};
    $display("DYN_PK:%04h:%02h", pk16, {<<1{8'hB4}});
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "SUB:a5:5a:7:7",
            "INIT:0000:xx:00:ff:zz",
            "NETS:z0x0:0:0:1",
            "WIDE_NOT_2S:1:1:1:1",
            "ASHR:12:-16:1:1:1",
            "REAL:4630000000001000:1",
            "CMP:0:1:1",
            "LOOP:00000000000000000000000000000012345674",
            "CASE:1:0:1",
            "D_PK:11223344",
            "D_UNPK:aa:bb:cc:dd",
            "D_WITH:aa:ca:fe:dd",
            "Q_UNPK:ff:4:10:40",
            "STR:78563412:e56:14131211100f0e0d0c0b0a090807060504030201",
            "DYN_PK:5566:2d",
        ]
    );
}

#[test]
fn wide_concat_repl_stream_and_drop_xz_in_compiled_loop() {
    let src = r#"
module top;
  logic [199:0] bus4;
  bit   [199:0] bus2;
  logic [159:0] comb_out;
  byte q_in [$];
  byte q_out [$];

  always_comb begin
    comb_out = {<<8{bus4[159:0]}};
  end

  initial begin
    bit [199:0] acc;
    acc = 200'h0;
    q_in = {8'h11, 8'h22, 8'h33, 8'h44, 8'h55};
    for (int i = 0; i < 6; i++) begin
      bus4 = {{99{1'b1}}, 1'bz, {100{1'b0}}} ^ i;
      bus2 = bus4;
      #1;
      acc = acc ^ bus2 ^ {5{40'h0102030405}};
    end
    {<<8{q_out}} = q_in;
    $display("BYTECODE:%b:%050h:%02h:%02h:%02h",
             (bus2[99] === 1'b0), acc, comb_out[7:0], q_out[0], q_out[4]);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec!["BYTECODE:1:00000000000000000000000000000000000000000000000001:ff:55:11"]
    );
}
