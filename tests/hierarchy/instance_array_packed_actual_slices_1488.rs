//! §7.4, §23.3.3.5 (#1488): an array of instances bit-distributes a packed
//! actual of the full width over its instances, the rightmost instance index
//! taking the least significant bits. The slices were built as part-selects
//! numbered `[W-1:0]`, but a part-select of a multidimensional packed array
//! selects ELEMENTS (instance k of 16 read 15 elements, 225 bits), and an
//! ascending or offset range does not start at bit 0. A part-select actual
//! was sized by its element count, and a parameterized formal by the
//! instantiating scope's parameters instead of the instance's.
//!
//! Every expectation was cross-checked against the reference simulator.

use xezim::simulate;

fn expect(src: &str, wants: &[&str]) {
    let sim = simulate(src, 100).expect("simulate failed");
    let o: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    for want in wants {
        assert!(o.iter().any(|l| l == want), "missing {want:?} in {o:?}");
    }
}

#[test]
fn packed_actuals_distribute_bits_over_instances() {
    expect(
        r#"
module tap (input logic [14:0] p);
  logic [14:0] cap;
  always @(*) cap = p;
endmodule
module tapw #(parameter int W = 4) (input logic [W-1:0] p);
  logic [W-1:0] cap;
  always @(*) cap = p;
endmodule
module drv (output logic [14:0] o);
  parameter int K = 0;
endmodule
module t;
  logic [3:0][14:0] p2;       // 2-D
  logic [1:0][1:0][14:0] p3;  // 3-D
  logic [0:3][14:0] pa;       // ascending element range
  logic [1:0][29:0] pw;       // element 30 != 15
  logic [59:0] pf;            // flat control
  logic [0:59] pfa;           // flat ascending
  logic [74:15] pfo;          // flat offset range
  logic [7:0][14:0] p8;       // part-select actual
  logic [1:0][14:0] c0, c1;   // concat
  logic [59:0] pp;            // param width formal
  tap u2 [3:0] (.p(p2));
  tap u3 [3:0] (.p(p3));
  tap ua [3:0] (.p(pa));
  tap uw [3:0] (.p(pw));
  tap uf [3:0] (.p(pf));
  tap ufa [3:0] (.p(pfa));
  tap ufo [3:0] (.p(pfo));
  tap us [3:0] (.p(p8[5:2]));
  tap uc [3:0] (.p({c0, c1}));
  tapw #(.W(15)) up [3:0] (.p(pp));
  tap ui [1:0] (.p(pw[1]));
  tap ua2 [0:3] (.p(p2));
  initial begin
    for (int k = 0; k < 4; k++) p2[k] = 15'h100 + k;
    for (int k = 0; k < 4; k++) p3[k/2][k%2] = 15'h200 + k;
    for (int k = 0; k < 4; k++) pa[k] = 15'h300 + k;
    pw = {15'h403, 15'h402, 15'h401, 15'h400};
    pf = {15'h503, 15'h502, 15'h501, 15'h500};
    pfa = {15'h603, 15'h602, 15'h601, 15'h600};
    pfo = {15'h703, 15'h702, 15'h701, 15'h700};
    for (int k = 0; k < 8; k++) p8[k] = 15'h10 + k;
    c0 = {15'h23, 15'h22}; c1 = {15'h21, 15'h20};
    pp = {15'h33, 15'h32, 15'h31, 15'h30};
    #1;
    $display("T|2d %h %h %h %h", u2[0].cap, u2[1].cap, u2[2].cap, u2[3].cap);
    $display("T|3d %h %h %h %h", u3[0].cap, u3[1].cap, u3[2].cap, u3[3].cap);
    $display("T|asc %h %h %h %h", ua[0].cap, ua[1].cap, ua[2].cap, ua[3].cap);
    $display("T|wide %h %h %h %h", uw[0].cap, uw[1].cap, uw[2].cap, uw[3].cap);
    $display("T|flat %h %h %h %h", uf[0].cap, uf[1].cap, uf[2].cap, uf[3].cap);
    $display("T|flata %h %h %h %h", ufa[0].cap, ufa[1].cap, ufa[2].cap, ufa[3].cap);
    $display("T|flato %h %h %h %h", ufo[0].cap, ufo[1].cap, ufo[2].cap, ufo[3].cap);
    $display("T|psel %h %h %h %h", us[0].cap, us[1].cap, us[2].cap, us[3].cap);
    $display("T|cat %h %h %h %h", uc[0].cap, uc[1].cap, uc[2].cap, uc[3].cap);
    $display("T|parm %h %h %h %h", up[0].cap, up[1].cap, up[2].cap, up[3].cap);
    $display("T|idx %h %h", ui[0].cap, ui[1].cap);
    $display("T|a2 %h %h %h %h", ua2[0].cap, ua2[1].cap, ua2[2].cap, ua2[3].cap);
  end
endmodule
"#,
        &[
            "T|2d 0100 0101 0102 0103",
            "T|3d 0200 0201 0202 0203",
            "T|asc 0303 0302 0301 0300",
            "T|wide 0400 0401 0402 0403",
            "T|flat 0500 0501 0502 0503",
            "T|flata 0600 0601 0602 0603",
            "T|flato 0700 0701 0702 0703",
            "T|psel 0012 0013 0014 0015",
            "T|cat 0020 0021 0022 0023",
            "T|parm 0030 0031 0032 0033",
            "T|idx 0402 0403",
            "T|a2 0103 0102 0101 0100",
        ],
    );
}

#[test]
fn packed_output_actuals_collect_instance_bits() {
    expect(
        r#"
module src_ #(parameter int V = 0) (output logic [7:0] o);
  assign o = 8'(V);
endmodule
module srcv (input logic [7:0] i, output logic [7:0] o);
  assign o = i + 8'h10;
endmodule
module t;
  logic [3:0][7:0] o2;
  logic [0:3][7:0] oa;
  logic [1:0][1:0][7:0] o3;
  logic [3:0][7:0] i2;
  srcv s2 [3:0] (.i(i2), .o(o2));
  srcv sa [3:0] (.i(i2), .o(oa));
  srcv s3 [3:0] (.i(i2), .o(o3));
  initial begin
    i2 = {8'h04, 8'h03, 8'h02, 8'h01};
    #1;
    $display("T|o2 %h", o2);
    $display("T|oa %h", oa);
    $display("T|o3 %h", o3);
  end
endmodule
"#,
        &["T|o2 14131211", "T|oa 14131211", "T|o3 14131211"],
    );
}

#[test]
fn top_qualified_writes_keep_packed_and_unpacked_geometry() {
    expect(
        r#"
module cellm;
  reg [3:0][39:0] mem;
  reg [39:0] umem [0:3];
  reg [39:0] vec;
endmodule
module blk;
  cellm c0 ();
endmodule
module t;
  blk n_abs ();
  blk n_one ();
  blk a_one [0:0] ();
  blk a_two [0:1] ();
  initial begin
    for (int l = 0; l < 4; l++) t.n_abs.c0.mem[l] = 40'd300 + l;
  end
  initial t.n_one.c0.mem[2] = 40'd400;
  initial t.a_one[0].c0.mem[2] = 40'd500;
  initial t.a_one[0].c0.vec = 40'd501;
  initial t.a_two[1].c0.umem[1] = 40'd600;
  initial begin
    #1;
    $display("T|nabs %0d %0d %0d", n_abs.c0.mem[1], t.n_abs.c0.mem[3], t.n_abs.c0.mem);
    $display("T|none %0d %0d", n_one.c0.mem[2], t.n_one.c0.mem[2]);
    $display("T|aone %0d %0d vec %0d %0d", a_one[0].c0.mem[2], t.a_one[0].c0.mem[2], a_one[0].c0.vec, t.a_one[0].c0.vec);
    $display("T|atwo %0d %0d", a_two[1].c0.umem[1], t.a_two[1].c0.umem[1]);
  end
endmodule
"#,
        &[
            "T|nabs 301 303 402756082723194605087377488228705632556",
            "T|none 400 400",
            "T|aone 500 500 vec 501 501",
            "T|atwo 600 600",
        ],
    );
}

#[test]
fn top_qualified_writes_from_a_child_match_root_and_upward_paths() {
    expect(
        r#"
module cellm;
  reg [3:0][39:0] mem;
  reg [39:0] vec;
endmodule
module blk;
  cellm c0 ();
endmodule
module wr_abs;
  initial t.a_c[0].c0.vec = 40'd11;
  initial for (int l = 0; l < 4; l++) t.n_c.c0.mem[l] = 40'd20 + l;
endmodule
module wr_up;
  initial a_u[0].c0.vec = 40'd31;
  initial for (int l = 0; l < 4; l++) n_u.c0.mem[l] = 40'd40 + l;
endmodule
module t;
  blk a_c [0:0] ();
  blk n_c ();
  blk a_u [0:0] ();
  blk n_u ();
  blk a_r [0:0] ();
  blk n_r ();
  wr_abs w1 ();
  wr_up w2 ();
  initial $root.t.a_r[0].c0.vec = 40'd51;
  initial for (int l = 0; l < 4; l++) $root.t.n_r.c0.mem[l] = 40'd60 + l;
  initial begin
    #1;
    $display("T|abs-child %0d %0d", a_c[0].c0.vec, n_c.c0.mem[2]);
    $display("T|up-child %0d %0d", a_u[0].c0.vec, n_u.c0.mem[2]);
    $display("T|root %0d %0d", a_r[0].c0.vec, n_r.c0.mem[2]);
  end
endmodule
"#,
        &["T|abs-child 11 22", "T|up-child 31 42", "T|root 51 62"],
    );
}
