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

#[test]
fn multi_driver_nets_tranif_packages_and_compiled_cast_parity() {
    let src = r#"
package m8_pkg_base;
  typedef logic [15:0] u16_t;
  localparam int BASE_C = 11;
endpackage

package m8_pkg_ext;
  import m8_pkg_base::*;
  class box_base #(int W = 8);
    int tag;
    function new(int t = 0);
      tag = t;
    endfunction
    virtual function int compute(int x);
      int a, b;
      a = x + W;
      b = a * 2;
      return b + tag;
    endfunction
  endclass

  class box_sub #(int W = 8) extends box_base #(W);
    int extra;
    function new(int t = 0, int e = 0);
      super.new(t);
      extra = e;
    endfunction
    function int try_cast_sum(box_base #(W) h);
      box_sub #(W) s;
      if ($cast(s, h)) return s.extra + compute(3);
      return -1;
    endfunction
  endclass

  class cast_checker;
    static function int check_spec_casts(box_base #(8) h);
      box_sub #(8) s8;
      box_sub #(16) s16;
      int ok1, ok2;
      ok1 = $cast(s8, h);
      ok2 = $cast(s16, h);
      if (ok1 && !ok2) return s8.try_cast_sum(h);
      return -2;
    endfunction
  endclass

  function automatic int pkg_helper(u16_t x);
    return int'(x) + BASE_C;
  endfunction
endpackage

module m8_leaf #(parameter int W = 8, parameter logic [7:0] K = 8'h5A) (
  input  logic [W-1:0] d_in,
  output logic [W-1:0] d_out
);
  bit [1:0][3:0] pair_v;
  initial pair_v = '{4'hA, 4'h5};
  if (W > 8) begin : g_wide
    assign d_out = d_in ^ {{(W-8){1'b0}}, K};
  end else begin : g_narrow
    assign d_out = d_in ^ K[W-1:0];
  end
  function automatic int read_pair();
    return {pair_v[1], pair_v[0]};
  endfunction
endmodule

module top;
  import m8_pkg_base::*;
  import m8_pkg_ext::*;

  logic [7:0]  in0, out0, out1;
  logic [15:0] in2, out2, out3;
  m8_leaf #(.W(8),  .K(8'h5A)) u0(.d_in(in0), .d_out(out0));
  m8_leaf #(.W(8),  .K(8'h5A)) u1(.d_in(in0), .d_out(out1));
  m8_leaf #(.W(16), .K(8'hA5)) u2(.d_in(in2), .d_out(out2));
  m8_leaf #(.W(16), .K(8'hA5)) u3(.d_in(in2), .d_out(out3));

  wire [159:0] w160;
  assign w160 = {{32{1'bz}}, 32'h1234_5678, {96{1'bz}}};
  assign w160 = {32'h8765_4321, {96{1'bz}}, 32'hCAFE_BABE};
  assign w160 = {{96{1'bz}}, 32'hDEAD_BEEF, {32{1'bz}}};
  assign w160 = {{152{1'bz}}, 8'h2F};

  wand [127:0] wd128;
  assign wd128 = {{32{1'bz}}, 32'hFFFF_0000, 32'hAAAA_5555, 32'h0000_FFFF};
  assign wd128 = {{32{1'bz}}, 32'hFFFF_FFFF, 32'h0000_0000, {32{1'bz}}};
  assign wd128 = {32'h1234_5678, {32{1'bx}}, {32{1'bz}}, {32{1'b1}}};
  assign wd128 = {32'hFFFF_FFFF, {96{1'bz}}};

  wor [127:0] wo128;
  assign wo128 = {{32{1'bz}}, 32'hFFFF_0000, 32'hAAAA_5555, 32'h0000_0000};
  assign wo128 = {{32{1'bz}}, 32'h0000_0000, 32'hFFFF_FFFF, {32{1'bz}}};
  assign wo128 = {32'h1234_5678, {32{1'bx}}, {32{1'bz}}, {32{1'b0}}};
  assign wo128 = {32'h0000_FFFF, {96{1'bz}}};

  tri0 [79:0] t0_80;
  assign t0_80 = {16'hA5A5, {64{1'bz}}};
  tri1 [79:0] t1_80;
  assign t1_80 = {{64{1'bz}}, 16'h5A5A};

  logic ta_drv, tb_drv, t_ctl;
  wire ta0, ta1, ta2, ta3;
  wire tb0, tb1, tb2, tb3;
  assign ta3 = ta_drv ? 1'b1 : 1'bz;
  assign ta2 = ta_drv ? 1'b0 : 1'bz;
  assign ta1 = ta_drv ? 1'bx : 1'bz;
  assign ta0 = ta_drv ? 1'bz : 1'bz;
  assign tb3 = tb_drv ? 1'b1 : 1'bz;
  assign tb2 = tb_drv ? 1'b1 : 1'bz;
  assign tb1 = tb_drv ? 1'b0 : 1'bz;
  assign tb0 = tb_drv ? 1'bz : 1'bz;
  tranif1 sw0(ta0, tb0, t_ctl);
  tranif1 sw1(ta1, tb1, t_ctl);
  tranif1 sw2(ta2, tb2, t_ctl);
  tranif1 sw3(ta3, tb3, t_ctl);

  initial begin
    box_sub #(8) sub_h;
    box_base #(8) base_h;
    int acc, cast_res;

    in0 = 8'hFF;
    in2 = 16'h0F0F;
    ta_drv = 1'b1; tb_drv = 1'b1; t_ctl = 1'b1;
    #1;
    $display("W160:%040h", w160);
    $display("WD128:%032h", wd128);
    $display("WO128:%032h", wo128);
    $display("TRI:%020h:%020h", t0_80, t1_80);
    $display("INST:%02h:%02h:%04h:%04h:%02h:%0d", out0, out1, out2, out3, u0.read_pair(), pkg_helper(16'd5));
    $display("TRAN_ON:%b%b%b%b:%b%b%b%b", ta3, ta2, ta1, ta0, tb3, tb2, tb1, tb0);

    t_ctl = 1'b0;
    #1;
    $display("TRAN_OFF:%b%b%b%b:%b%b%b%b", ta3, ta2, ta1, ta0, tb3, tb2, tb1, tb0);

    t_ctl = 1'bx;
    #1;
    $display("TRAN_X:%b%b%b%b:%b%b%b%b", ta3, ta2, ta1, ta0, tb3, tb2, tb1, tb0);

    sub_h = new(5, 42);
    base_h = sub_h;
    acc = 0;
    for (int k = 0; k < 5; k++) begin
      acc += sub_h.compute(k);
    end
    cast_res = cast_checker::check_spec_casts(base_h);
    $display("CLS:%0d:%0d", acc, cast_res);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "W160:8765432112345678zzzzzzzzdeadbeefcafebaXX",
            "WD128:12345678xxxx0000000000000000ffff",
            "WO128:1234ffffffffxxxxffffffff00000000",
            "TRI:a5a50000000000000000:ffffffffffffffff5a5a",
            "INST:a5:a5:0faa:0faa:a5:16",
            "TRAN_ON:1xxz:1xxz",
            "TRAN_OFF:10xz:110z",
            "TRAN_X:1xxz:1xxz",
            "CLS:125:69",
        ]
    );
}

#[test]
fn bytes_string_inline_signal_dsu_tran_and_two_state_fast_paths() {
    let src = r#"
module top;
  wire n0, n1, n2, n3, n4, n5, n6, n7, n8, n9, n10, n11, n12, n13, n14, n15;
  logic drv0;
  assign n0 = drv0;
  tran t00(n0, n1);   tran t01(n1, n2);   tran t02(n2, n3);   tran t03(n3, n4);
  tran t04(n4, n5);   tran t05(n5, n6);   tran t06(n6, n7);   tran t07(n7, n8);
  tran t08(n8, n9);   tran t09(n9, n10);  tran t10(n10, n11); tran t11(n11, n12);
  tran t12(n12, n13); tran t13(n13, n14); tran t14(n14, n15);

  wire u0, u1, u2, u3;
  logic udrv0, udrv3;
  assign u0 = udrv0;
  assign u3 = udrv3;
  tran tu0(u0, u1);
  tran tu1(u1, u2);
  tran tu2(u2, u3);

  wire ca, cb;
  logic cdrv_a, cdrv_b;
  assign ca = cdrv_a;
  assign cb = cdrv_b;
  tran t_conf(ca, cb);

  wire ta, tb;
  logic tdrv_a, tdrv_b, t_en;
  assign ta = tdrv_a;
  assign tb = tdrv_b;
  tranif1 sw_c(ta, tb, t_en);

  logic [31:0] dc_src32;
  bit   [31:0] dc_dst32;
  assign dc_dst32 = dc_src32;

  logic [63:0] dc_src64;
  bit   [63:0] dc_dst64;
  assign dc_dst64 = dc_src64;

  real   r_arr[4];
  string s_arr[4];

  function automatic bit [31:0] conv_to_bit32(
    input logic [31:0] lv,
    input logic signed [7:0] sv8,
    input logic [7:0] uv8,
    input logic [127:0] w128,
    input real rv,
    input int mode
  );
    bit [31:0] res;
    case (mode)
      0: res = lv;
      1: res = sv8;
      2: res = uv8;
      3: res = w128;
      4: res = rv;
      default: res = '1;
    endcase
    return res;
  endfunction

  function automatic string str_loop_op(input string a, input string b, input int idx, input byte ch);
    string s;
    s = {a, ":", b};
    s = s.toupper();
    s.putc(idx, ch);
    return s;
  endfunction

  typedef struct packed {
    logic signed [15:0] x;
  } s16_t;
  typedef logic signed [15:0] l16_t;

  initial begin
    string long_a, long_b, cat_s, rep_s, low_s, sub_s, nul_s;
    string str_q[$];
    int cmp1, cmp2;
    longint fsum;
    bit [31:0] b0, b1, b2, b3, b4, b5;
    real r_neg;
    logic [7:0] latin;
    s16_t casted;
    l16_t lcast;

    long_a = "AlphaBetaGammaDeltaEpsilonZetaEtaThetaIotaKappaLambdaMuNuXiOmicronPiRho";
    long_b = "0123456789_SigmaTauUpsilonPhiChiPsiOmega";
    cat_s = {long_a, "-", long_b, 8'h21};
    rep_s = {2{"Segment_0123456789_"}};
    low_s = cat_s.tolower();
    sub_s = cat_s.substr(5, 19);
    cmp1 = cat_s.compare(low_s);
    cmp2 = cat_s.icompare(low_s);
    str_q.push_back(cat_s);
    str_q.push_back(low_s);
    str_q[0].putc(0, "Z");
    fsum = 0;
    foreach (cat_s[i]) begin
      fsum += (cat_s.getc(i) ^ i);
    end
    nul_s = string'({8'h41, 8'h00, 8'h42, "LongTail_0123456789_0123456789_0123456789_0123456789"});
    $display("STR len=%0d cat=%s sub=%s cmp=%0d,%0d q0=%c getc5=%c fsum=%0d nul=%s",
             cat_s.len(), str_loop_op(long_a, long_b, 5, "_"), sub_s,
             (cmp1 < 0) ? -1 : ((cmp1 > 0) ? 1 : 0), cmp2,
             str_q[0].getc(0), cat_s.getc(5), fsum, nul_s);

    latin = 8'hE9;
    cat_s = {long_b, latin};
    cat_s = cat_s.toupper();
    r_neg = $bitstoreal(64'h8000_0000_0000_0000);
    casted = s16_t'(16'hFFF5);
    lcast = l16_t'(32'hFFFF_FFF4);
    $display("MISC latin_last=%02x s_def_len=%0d r_def=%016x r_neg=%016x cast=%0d,%0d",
             cat_s.getc(cat_s.len()-1), s_arr[2].len(),
             $realtobits(r_arr[1]), $realtobits(r_neg), casted.x, lcast);

    drv0 = 1'b1; udrv0 = 1'b1; udrv3 = 1'b0; #1;
    $display("TRAN_1 n15=%b n7=%b u1=%b u2=%b", n15, n7, u1, u2);
    drv0 = 1'b0; udrv3 = 1'bz; #1;
    $display("TRAN_0 n15=%b n7=%b u1=%b u2=%b", n15, n7, u1, u2);
    drv0 = 1'bz; #1;
    $display("TRAN_Z n15=%b n7=%b", n15, n7);

    cdrv_a = 1'b1; cdrv_b = 1'b0; #1;
    $display("CONF_X ca=%b cb=%b", ca, cb);
    cdrv_b = 1'bz; #1;
    $display("CONF_1 ca=%b cb=%b", ca, cb);

    tdrv_a = 1'b1; tdrv_b = 1'bz; t_en = 1'b0; #1;
    $display("TIF_OFF ta=%b tb=%b", ta, tb);
    t_en = 1'b1; #1;
    $display("TIF_ON  ta=%b tb=%b", ta, tb);
    tdrv_b = 1'b0; t_en = 1'bx; #1;
    $display("TIF_UNK ta=%b tb=%b", ta, tb);

    dc_src32 = 32'hA5x0_zF12;
    dc_src64 = 64'hF00D_xz12_CAFE_zx99;
    #1;
    $display("DC dc32=%08x dc64=%016x", dc_dst32, dc_dst64);

    b0 = conv_to_bit32(32'hA5x0_zF12, -8'sd5, 8'hAB, 128'h1111_2222_3333_4444_5555_6666_DEAD_BEEF, 3.75, 0);
    b1 = conv_to_bit32(0, -8'sd5, 8'hAB, 0, 3.75, 1);
    b2 = conv_to_bit32(0, -8'sd5, 8'hAB, 0, 3.75, 2);
    b3 = conv_to_bit32(0, -8'sd5, 8'hAB, 128'h1111_2222_3333_4444_5555_6666_DEAD_BEEF, 3.75, 3);
    b4 = conv_to_bit32(0, -8'sd5, 8'hAB, 0, 3.75, 4);
    b5 = conv_to_bit32(0, -8'sd5, 8'hAB, 0, 3.75, 5);
    $display("BA b0=%08x b1=%08x b2=%08x b3=%08x b4=%08x b5=%08x", b0, b1, b2, b3, b4, b5);
  end
endmodule
"#;
    assert_eq!(
        sim_lines(src),
        vec![
            "STR len=113 cat=ALPHA_ETAGAMMADELTAEPSILONZETAETATHETAIOTAKAPPALAMBDAMUNUXIOMICRONPIRHO:0123456789_SIGMATAUUPSILONPHICHIPSIOMEGA sub=BetaGammaDeltaE cmp=-1,0 q0=Z getc5=B fsum=8588 nul=ABLongTail_0123456789_0123456789_0123456789_0123456789",
            "MISC latin_last=e9 s_def_len=0 r_def=0000000000000000 r_neg=8000000000000000 cast=-11,-12",
            "TRAN_1 n15=1 n7=1 u1=x u2=x",
            "TRAN_0 n15=0 n7=0 u1=1 u2=1",
            "TRAN_Z n15=z n7=z",
            "CONF_X ca=x cb=x",
            "CONF_1 ca=1 cb=1",
            "TIF_OFF ta=1 tb=z",
            "TIF_ON  ta=1 tb=1",
            "TIF_UNK ta=x tb=x",
            "DC dc32=a5000f12 dc64=f00d0012cafe0099",
            "BA b0=a5000f12 b1=fffffffb b2=000000ab b3=deadbeef b4=00000004 b5=ffffffff",
        ]
    );
}
