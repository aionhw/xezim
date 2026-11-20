// IEEE 1800-2023 §6.12.1, §10.3.2, §10.7, §13.4.1, §13.5: a real assigned to
// a 64-bit integral variable or net is rounded (ties away from zero), in every
// compiled form. Real storage is 64 bits wide too, so fitting the value to
// the width alone kept the real's IEEE 754 bits.
module sink(input logic [63:0] v, output logic [63:0] o);
  assign o = v;
endmodule
module top;
  typedef struct packed { logic [63:0] a; } ps_t;
  real r = 2.5;
  shortreal sr = 2.5;
  realtime rt = 2.5;
  wire [63:0] w_decl = r;
  wire [63:0] w_time = 9200s;
  wire [63:0] w_cont;
  assign w_cont = r;
  wire [63:0] w_sr = sr;
  wire [63:0] w_rt = rt;
  wire signed [63:0] w_signed = r;
  wire [63:0] w_port;
  sink u_port(.v(r), .o(w_port));
  wire [63:0] w_sum = r * 2;
  logic [63:0] v_comb, v_blk, v_nba, v_mem [2], v_cond, v_plus, v_arg, v_ret, v_loc, v_init;
  time t_blk;
  ps_t ps;
  function automatic longint f_arg(longint a); return a; endfunction
  function automatic longint f_ret(real x); return x; endfunction
  function automatic logic [63:0] f_loc(real x); logic [63:0] t; t = x; return t; endfunction
  function automatic longint f_init(real x); longint t = x; return t; endfunction
  always_comb v_comb = r;
  always @(r) begin
    v_blk = r;
    v_nba <= r;
    v_mem[1] = r;
    v_cond = r > 0 ? r : -r;
    v_plus = 0;
    v_plus += r;
    t_blk = r;
    ps.a = r;
    v_arg = f_arg(r);
    v_ret = f_ret(r);
    v_loc = f_loc(r);
    v_init = f_init(r);
  end
  initial begin
    #1 r = 3.5; sr = 3.5; rt = 3.5;
    #1 $display("T|nets %0d %0d %0d %0d %0d %0d %0d %0d", w_decl, w_time, w_cont, w_sr, w_rt, w_signed, w_port, w_sum);
    $display("T|vars %0d %0d %0d %0d %0d %0d %0d %0d", v_comb, v_blk, v_nba, v_mem[1], v_cond, v_plus, t_blk, ps.a);
    $display("T|calls %0d %0d %0d %0d", v_arg, v_ret, v_loc, v_init);
    r = -2.5; sr = -2.5; rt = -2.5;
    #1 $display("T|nets %0d %0d %0d %0d %0d %0d", $signed(w_decl), $signed(w_cont), $signed(w_sr), $signed(w_rt), w_signed, $signed(w_port));
    $display("T|vars %0d %0d %0d %0d %0d %0d %0d", $signed(v_comb), $signed(v_blk), $signed(v_nba), $signed(v_mem[1]), $signed(v_cond), $signed(v_plus), $signed(ps.a));
    $display("T|calls %0d %0d %0d %0d", $signed(v_arg), $signed(v_ret), $signed(v_loc), $signed(v_init));
    $finish;
  end
endmodule
