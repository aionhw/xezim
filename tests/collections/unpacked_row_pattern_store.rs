//! IEEE 1800-2017 §10.9.1, §10.10, §7.6: an assignment pattern written to a
//! row of a multi-dimensional unpacked array (`c[0] = '{V, V}` on
//! `logic [39:0] c [2][2]`) assigns each element of that row. Each element
//! is its own signal; the row was taken as one value with no storage of its
//! own, so every element read x. Positional, keyed, `default:` and nested
//! patterns, any element width, any dimension direction, and constant or
//! run-time row indices; blocking stores and continuous assignments (§10.3).
//! Expected values are from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m.trim() == want),
        "missing {want:?} in {msgs:#?}"
    );
}

#[test]
fn positional_row_patterns_of_any_element_width() {
    let msgs = messages(
        "module t;
  localparam logic [39:0] V = 40'hab00000001;
  logic [39:0] v = 40'hab00000001;
  logic [3:0]  a4 [2][2];
  logic [31:0] a32 [2][2];
  logic [32:0] a33 [2][2];
  logic [39:0] a40 [2][2], l40 [2][2], n40 [2][2];
  logic [63:0] a64 [2][2];
  initial begin
    a4[0] = '{4'h1, 4'h2};
    a32[0] = '{32'h11112222, 32'h33334444};
    a33[0] = '{33'h1_1111_2222, 33'h0_3333_4444};
    a40[0] = '{V, V};
    l40[0] = '{40'hab00000001, 40'hcd00000002};
    n40[0] = '{v, v};
    a64[0] = '{64'h1111222233334444, 64'h5555666677778888};
    #1 $display(\"a4=%h%h a32=%h%h a33=%h%h\", a4[0][0], a4[0][1], a32[0][0], a32[0][1],
                a33[0][0], a33[0][1]);
    $display(\"a40=%h%h l40=%h%h n40=%h%h a64=%h%h\", a40[0][0], a40[0][1], l40[0][0],
             l40[0][1], n40[0][0], n40[0][1], a64[0][0], a64[0][1]);
  end
endmodule",
    );
    assert_line(&msgs, "a4=12 a32=1111222233334444 a33=111112222033334444");
    assert_line(
        &msgs,
        "a40=ab00000001ab00000001 l40=ab00000001cd00000002 n40=ab00000001ab00000001 \
         a64=11112222333344445555666677778888",
    );
}

#[test]
fn keyed_default_nested_and_three_dimensional_rows() {
    let msgs = messages(
        "module t;
  logic [7:0] k [2][2], d3 [2][2][2], sz [2][3];
  logic [39:0] d40 [2][2];
  logic [7:0] y = 8'h10;
  initial begin
    k[0] = '{0: 8'h44, 1: 8'h55};
    d3[1][0] = '{8'h66, 8'h77};
    d3[0] = '{'{8'h01, 8'h02}, '{8'h03, 8'h04}};
    sz[1] = '{8'ha1, 8'ha2, 8'ha3};
    sz[0] = '{y + 8'h1, y << 1, ~y};
    d40[1] = '{default: 40'hee000000ee};
    #1 $display(\"k=%h%h d3a=%h%h d3b=%h%h%h%h sz1=%h%h%h sz0=%h%h%h d40=%h%h\",
                k[0][0], k[0][1], d3[1][0][0], d3[1][0][1], d3[0][0][0], d3[0][0][1],
                d3[0][1][0], d3[0][1][1], sz[1][0], sz[1][1], sz[1][2], sz[0][0],
                sz[0][1], sz[0][2], d40[1][0], d40[1][1]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "k=4455 d3a=6677 d3b=01020304 sz1=a1a2a3 sz0=1120ef d40=ee000000eeee000000ee",
    );
}

#[test]
fn ascending_descending_offset_and_run_time_rows() {
    let msgs = messages(
        "module t;
  logic [7:0] asc [0:1][0:1], dsc [1:0][1:0], off [2:3][4:5], dyn [2][2];
  int i = 1;
  initial begin
    asc[1] = '{8'h01, 8'h02};
    dsc[1] = '{8'h03, 8'h04};
    off[3] = '{8'h05, 8'h06};
    off[2] = '{5: 8'h07, 4: 8'h08};
    dyn[i] = '{8'h09, 8'h0a};
    #1 $display(\"asc=%h%h dsc=%h%h off3=%h%h off2=%h%h dyn=%h%h\", asc[1][0], asc[1][1],
                dsc[1][1], dsc[1][0], off[3][4], off[3][5], off[2][4], off[2][5],
                dyn[1][0], dyn[1][1]);
  end
endmodule",
    );
    assert_line(&msgs, "asc=0102 dsc=0304 off3=0506 off2=0807 dyn=090a");
}

#[test]
fn row_pattern_in_an_always_block() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [7:0] al [2][2];
  logic [7:0] x8 = 8'h5a;
  always @(posedge clk) al[1] = '{x8, 8'h33};
  initial begin
    #1 clk = 1;
    #1 $display(\"al=%h%h\", al[1][0], al[1][1]);
  end
endmodule",
    );
    assert_line(&msgs, "al=5a33");
}

#[test]
fn continuous_row_patterns() {
    let msgs = messages(
        "module t;
  logic [7:0] s = 8'h10;
  logic [7:0] ca [2][2], ck [2][2], cd [2][2], c3 [2][2][2], cs [2][2], cw [2][2];
  logic [39:0] c40 [2][2];
  assign ca[1] = '{8'hc1, 8'hc2};
  assign ck[0] = '{1: 8'hd1, 0: 8'hd0};
  assign cd[1] = '{default: 8'hee};
  assign c3[1] = '{'{8'h01, 8'h02}, '{8'h03, 8'h04}};
  assign cs[0] = '{s, s + 8'h1};
  assign c40[0] = '{40'hab00000001, 40'hcd00000002};
  assign cw = '{'{8'h11, 8'h12}, '{8'h13, 8'h14}};
  initial begin
    #1 $display(\"ca=%h%h ck=%h%h cd=%h%h c3=%h%h%h%h cs=%h%h c40=%h%h cw=%h%h%h%h\",
                ca[1][0], ca[1][1], ck[0][0], ck[0][1], cd[1][0], cd[1][1], c3[1][0][0],
                c3[1][0][1], c3[1][1][0], c3[1][1][1], cs[0][0], cs[0][1], c40[0][0],
                c40[0][1], cw[0][0], cw[0][1], cw[1][0], cw[1][1]);
    s = 8'h20;
    #1 $display(\"cs2=%h%h\", cs[0][0], cs[0][1]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "ca=c1c2 ck=d0d1 cd=eeee c3=01020304 cs=1011 c40=ab00000001cd00000002 cw=11121314",
    );
    assert_line(&msgs, "cs2=2021");
}

/// Guard: a nonblocking row pattern, a row copied from a 1-D array, a
/// declaration initializer, a whole-array pattern on a 1-D array of packed
/// elements and an element store already work.
#[test]
fn row_stores_that_already_work() {
    let msgs = messages(
        "module t;
  logic clk = 0;
  logic [7:0] nb [2][2], el [2][2];
  logic [7:0] w [2][2] = '{'{8'h01, 8'h02}, '{8'h03, 8'h04}};
  logic [39:0] r40 [2][2];
  logic [39:0] row [2];
  logic [1:0][39:0] pp [2];
  always @(posedge clk) nb[0] <= '{8'h11, 8'h22};
  initial begin
    row = '{40'h1100000011, 40'h2200000022};
    r40[1] = row;
    pp = '{default: 80'h0};
    el[1][0] = 8'h77;
    #1 clk = 1;
    #1 $display(\"nb=%h%h w=%h%h%h%h r40=%h%h pp=%h el=%h\", nb[0][0], nb[0][1], w[0][0],
                w[0][1], w[1][0], w[1][1], r40[1][0], r40[1][1], pp[1], el[1][0]);
  end
endmodule",
    );
    assert_line(
        &msgs,
        "nb=1122 w=01020304 r40=11000000112200000022 pp=00000000000000000000 el=77",
    );
}
