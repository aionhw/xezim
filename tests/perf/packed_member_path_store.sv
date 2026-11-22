// §7.2, §7.4.2: member writes through constant element selects of packed
// structs (`st.e[1].dw[0].value <= d`), from generate loops, NBA and
// blocking, in a submodule and in the top module, with a descending and an
// ascending packed dimension. Each must compile (no interpreter fallback) and
// land where Verilator 5.046 puts it.
package pms_pkg;
  typedef struct packed { logic [7:0] value; } dw_t;
  typedef struct packed { dw_t [1:0] dw; } nest_t;
  typedef struct packed { nest_t [1:0] e; } nest2_t;
  typedef struct packed { logic [3:0] tag; dw_t [0:2] up; } asc_t;
endpackage

module pms_sub import pms_pkg::*; (input logic clk, input logic [7:0] d,
                                   output nest2_t st, output nest_t c_o, output asc_t a_o,
                                   output logic [7:0] rd, output nest2_t s2_o);
  // `s2` also names a different signal in the top module.
  nest2_t s2;
  always_ff @(posedge clk) begin
    s2 <= '0;
    s2.e[1].dw[1].value <= 8'h99;
  end
  assign s2_o = s2;
  // A read through constant element selects.
  assign rd = st.e[1].dw[0].value;
  for (genvar i = 0; i < 2; i++) begin : g_e
    for (genvar j = 0; j < 2; j++) begin : g_d
      always_ff @(posedge clk) st.e[i].dw[j].value <= d + 8'(2 * i + j);
    end
  end
  always_comb begin
    c_o.dw[0].value = d;
    c_o.dw[1].value = d + 1;
  end
  always_comb begin
    a_o.tag = 4'h9;
    a_o.up[0].value = 8'hA0;
    a_o.up[1].value = 8'hA1;
    a_o.up[2].value = 8'hA2;
  end
endmodule

module packed_member_path_store_check;
  import pms_pkg::*;
  logic clk = 0;
  logic [7:0] d = 8'h20;
  nest2_t st, top_st;
  nest_t c;
  asc_t a;
  logic [7:0] rd;
  nest2_t s2_o;
  logic [7:0] s2 = 8'h11;
  pms_sub u (.clk(clk), .d(d), .st(st), .c_o(c), .a_o(a), .rd(rd), .s2_o(s2_o));
  always_ff @(posedge clk) begin
    top_st.e[1].dw[0].value <= 8'h55;
    top_st.e[0].dw[1].value <= 8'h66;
    top_st.e[1].dw[1].value <= 8'h77;
    top_st.e[0].dw[0].value <= 8'h88;
  end
  initial begin
    #1 clk = 1;
    #1;
    $display("st=%h c=%h a=%h top_st=%h rd=%h s2_o=%h s2=%h", st, c, a, top_st, rd, s2_o, s2);
    if (st === 32'h23222120 && c === 16'h2120 && a === 28'h9a0a1a2 && top_st === 32'h77556688
        && rd === 8'h22 && s2_o === 32'h99000000 && s2 === 8'h11)
      $display("PACKED_MEMBER_PATH_OK");
    else
      $display("PACKED_MEMBER_PATH_FAIL");
    $finish;
  end
endmodule
