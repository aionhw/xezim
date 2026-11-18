//! §7.2.1, §7.4.1, §13.4: a member of a packed-struct formal that is itself a
//! multi-dimensional packed array (`logic [3:0][4:0] sh`) is read by element
//! (`c.sh[i]`, `c.m3[i][j]`), by a bit or part of an element, and through a
//! nested struct, inside an inlined function, with the values the reference
//! simulator prints: a constant or variable index, ascending and offset
//! dimensions, an index out of range or x (x, or 0 for a 2-state struct).
//! A nested callee that reads its module's `localparam` (§6.20) is inlined
//! with it; one that reads a module variable sees the variable's current
//! value (#1245).

use xezim::simulate;

fn trace(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn packed_array_members_of_struct_formals() {
    let src = r#"
typedef struct packed {
  logic [3:0][4:0] sh;        // packed array member
  logic signed [7:0] s;       // signed member
  logic [1:0][2:0][1:0] m3;   // 3-D packed member
} cfg_t;
typedef struct packed {
  cfg_t inner;
  logic [2:0][7:0] b;
} outer_t;

module sub #(parameter int W = 6) (input logic clk, input cfg_t c, input outer_t o,
                                   input logic [1:0] idx, input logic rst, output logic [63:0] acc);
  localparam int PW = W;
  localparam logic [7:0] MASK = 8'h3C;
  function automatic logic [4:0] f_var(input logic [1:0] i, input cfg_t cc);
    return cc.sh[i];
  endfunction
  function automatic logic [4:0] f_const(input cfg_t cc);
    return cc.sh[2];
  endfunction
  function automatic logic signed [15:0] f_signed(input cfg_t cc);
    return cc.s;                       // sign-extends
  endfunction
  function automatic logic [1:0] f_3d(input logic [1:0] i, input logic [1:0] j, input cfg_t cc);
    return cc.m3[i[0]][j];
  endfunction
  function automatic logic [2:0] f_bits(input logic [1:0] i, input cfg_t cc);
    return cc.sh[i][3:1];              // part select of an element
  endfunction
  function automatic logic f_bit(input logic [1:0] i, input cfg_t cc);
    return cc.sh[i][i];                // bit of an element
  endfunction
  function automatic logic [7:0] f_nested(input logic [1:0] i, input outer_t oo);
    return {oo.inner.sh[i], 3'b0} ^ oo.b[i];
  endfunction
  function automatic logic [PW-1:0] clip(input logic [15:0] v);
    if ((v >> PW) != '0) return {PW{1'b1}};
    return v[PW-1:0];
  endfunction
  function automatic logic [7:0] use_mask(input logic [7:0] v);
    return v & MASK;
  endfunction
  function automatic logic [PW-1:0] outer_fn(input logic [15:0] v);
    return clip(v + 1) ^ PW'(use_mask(v[7:0]));
  endfunction
  always_ff @(posedge clk) begin
    if (rst) acc <= 0;
    else begin
      acc[4:0]   <= f_var(idx, c);
      acc[9:5]   <= f_const(c);
      acc[25:10] <= f_signed(c);
      acc[27:26] <= f_3d(idx, ~idx, c);
      acc[30:28] <= f_bits(idx, c);
      acc[31]    <= f_bit(idx, c);
      acc[39:32] <= f_nested(idx, o);
      acc[44:40] <= outer_fn({8'h0, c.s});
      acc[63:45] <= '0;
    end
  end
endmodule

module top;
  logic clk = 0;
  cfg_t c;
  outer_t o;
  logic [1:0] idx;
  logic [63:0] acc;
  logic rst = 1;
  sub #(.W(5)) u (.clk(clk), .c(c), .o(o), .idx(idx), .rst(rst), .acc(acc));
  logic [63:0] r = 64'h1234_5678_9abc_def1;
  function automatic logic [63:0] step(input logic [63:0] x);
    return {x[62:0], x[63] ^ x[60] ^ x[1] ^ x[0]} ^ (x >> 7);
  endfunction
  initial begin
    #1 clk = 1; #1 clk = 0; rst = 0;
    for (int n = 0; n < 40; n++) begin
      r = step(r); c = r[$bits(cfg_t)-1:0];
      r = step(r); o = r;
      idx = n[1:0];
      if (n == 7) c.sh[1] = 5'bx01z0;
      #1 clk = 1; #1 clk = 0;
      $display("T|n=%0d acc=%h", n, acc);
    end
    $finish;
  end
endmodule
"#;
    assert_eq!(
        trace(src),
        [
            "T|n=0 acc=000013be2Xff32e4",
            "T|n=1 acc=00001f9800010200",
            "T|n=2 acc=000001a04c000318",
            "T|n=3 acc=00001fxxcbfe0089",
            "T|n=4 acc=00001b6d6X01162c",
            "T|n=5 acc=000001069c003062",
            "T|n=6 acc=0000033008000800",
            "T|n=7 acc=000004xx04002fa0",
            "T|n=8 acc=00000fa98Xffca61",
            "T|n=9 acc=00000fa207ffc210",
            "T|n=10 acc=000017b81bff2463",
            "T|n=11 acc=000003xxcc005809",
            "T|n=12 acc=00000fb07X01425e",
            "T|n=13 acc=0000132eb801b077",
            "T|n=14 acc=000017d907fe2421",
            "T|n=15 acc=000003xx0000f440",
            "T|n=16 acc=00000f0d4Xff43b8",
            "T|n=17 acc=00000f0493fe4393",
            "T|n=18 acc=000017011bfe2652",
            "T|n=19 acc=000004xxf4000cbe",
            "T|n=20 acc=000003e74X017298",
            "T|n=21 acc=00000fe12800cf14",
            "T|n=22 acc=00001b881c011042",
            "T|n=23 acc=00000bxxe000d1cc",
            "T|n=24 acc=0000036c2X00fe94",
            "T|n=25 acc=000003b607fef481",
            "T|n=26 acc=00000bb607fed421",
            "T|n=27 acc=00001fxx1fff0812",
            "T|n=28 acc=000003bfbX003af7",
            "T|n=29 acc=0000175c6401259d",
            "T|n=30 acc=000017b30bffae10",
            "T|n=31 acc=000013xx37febda6",
            "T|n=32 acc=00001b9d3X0195e6",
            "T|n=33 acc=00001718d401adea",
            "T|n=34 acc=000007bee80161ad",
            "T|n=35 acc=000003xx3ffef657",
            "T|n=36 acc=000003da8Xff7421",
            "T|n=37 acc=00001324d401b24b",
            "T|n=38 acc=00000b8157fe535a",
            "T|n=39 acc=00000bxxcbffdd49",
        ]
    );
}

#[test]
fn member_dimensions_ranges_and_two_state_structs() {
    let src = r#"
typedef struct packed {
  logic [0:3][4:0] up;        // ascending outer dimension
  logic [4:1][3:0] off;       // offset outer dimension
  logic [7:0] tail;
} asc_t;
typedef struct packed {
  bit [3:0][5:0] b;
  bit [3:0] t;
} two_t;

module sub2 (input logic clk, input asc_t a, input two_t w, input logic [2:0] i,
             input logic [7:0] mv_in, output logic [63:0] r);
  logic [7:0] mv;
  assign mv = mv_in;
  function automatic logic [4:0] up_el(input logic [2:0] k, input asc_t s);
    return s.up[k];
  endfunction
  function automatic logic [3:0] off_el(input logic [2:0] k, input asc_t s);
    return s.off[k];
  endfunction
  function automatic logic [5:0] two_el(input logic [2:0] k, input two_t s);
    return s.b[k];
  endfunction
  function automatic logic [3:0] up_bits(input logic [2:0] k, input asc_t s);
    return s.up[k][4:1];
  endfunction
  function automatic logic [7:0] rd_mv(input logic [7:0] v);
    return v ^ mv;              // module variable read
  endfunction
  function automatic logic [7:0] outer_mv(input logic [7:0] v);
    return rd_mv(v) + 8'd1;
  endfunction
  always_ff @(posedge clk) begin
    r[4:0]   <= up_el(i, a);
    r[8:5]   <= off_el(i, a);
    r[14:9]  <= two_el(i, w);
    r[18:15] <= up_bits(i, a);
    r[26:19] <= outer_mv(a.tail);
    r[63:27] <= '0;
  end
endmodule

module top;
  logic clk = 0;
  asc_t a;
  two_t w;
  logic [2:0] i;
  logic [7:0] mv_in;
  logic [63:0] r;
  sub2 u (.clk(clk), .a(a), .w(w), .i(i), .mv_in(mv_in), .r(r));
  initial begin
    for (int n = 0; n < 12; n++) begin
      a = {32'h9abc_def0 ^ (n * 32'h1357_9bdf), 16'h5a5a ^ 16'(n)};
      w = 28'h1234567 ^ (n * 28'h0f0f0f1);
      i = n[2:0];
      if (n == 9) i = 3'bx10;
      mv_in = 8'(n * 37);
      #1 clk = 1; #1 clk = 0;
      $display("T|n=%0d r=%h", n, r);
    end
    $finish;
  end
endmodule
"#;
    assert_eq!(
        trace(src),
        [
            "T|n=0 r=0000000002dd2XX5",
            "T|n=1 r=0000000003fedb5a",
            "T|n=2 r=00000000009f84bf",
            "T|n=3 r=0000000001bb1fad",
            "T|n=4 r=00000000065XX1Xx",
            "T|n=5 r=00000000073XXXxx",
            "T|n=6 r=00000000041XXXxx",
            "T|n=7 r=0000000002fXXXxx",
            "T|n=8 r=0000000003d85XX0",
            "T|n=9 r=0000000000fXXXxx",
            "T|n=10 r=00000000011994a6",
            "T|n=11 r=00000000063b5aac",
        ]
    );
}
