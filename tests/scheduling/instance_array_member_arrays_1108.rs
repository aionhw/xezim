//! §23.3.2, §7.4, §10.4.2, §12.7.3, §13.5 (#1108): an unpacked array that
//! is a member of an element of an array of instances (`lane[0].payload`) is
//! an array wherever a member of a single instance (`one.payload`) is: a
//! nonblocking write to an element stores the whole element, a whole-array
//! read, assignment and subroutine actual carry every element, `foreach` and
//! `$size` see its dimension, and an element of a multidimensional member
//! (`lane[0].nest[1][j]`, `lane[0].cube[1][0][2]`) is read and written, in an
//! `initial` block, in an `always_comb` and in a nested module. The path
//! stayed a member access on a select, which those operations did not take
//! for an array. The expected lines are the reference simulator's.

const DESIGN: &str = r#"
interface lane_if;
  logic [31:0] payload [6];
  logic [31:0] nest [2][6];
  logic [31:0] cube [2][2][6];
endinterface
module hold_mod;
  logic [31:0] nest [2][6];
endmodule
module top;
  lane_if lane [3] ();
  lane_if one ();
  comb_reads u_comb ();
  nest_owner u_n ();
  hold_mod hm [3] ();
  logic [31:0] w [6];
  function automatic logic [31:0] pick_word(logic [31:0] p [6], int i);
    return p[i];
  endfunction
  initial begin
    for (int j = 0; j < 6; j++) lane[0].payload[j] <= 32'h10 + j;
    for (int j = 0; j < 6; j++) one.payload[j] <= 32'h20 + j;
    #1 $display("S22 %h %h | one %h", lane[0].payload[2], lane[0].payload[5], one.payload[2]);
    $display("S23 %h | one %h", pick_word(lane[0].payload, 2), pick_word(one.payload, 2));
    w = lane[0].payload; $display("S43 %h %h", w[2], w[5]);
    w = one.payload; $display("S43one %h %h", w[2], w[5]);
    foreach (lane[0].payload[i]) if (i == 2) $display("S48 %0d %h", i, lane[0].payload[i]);
    foreach (one.payload[i]) if (i == 2) $display("S48one %0d %h", i, one.payload[i]);
    for (int j = 0; j < 6; j++) lane[0].nest[1][j] = 32'h30 + j;
    for (int j = 0; j < 6; j++) hm[0].nest[1][j] = 32'h40 + j;
    for (int j = 0; j < 6; j++) one.nest[1][j] = 32'h50 + j;
    $display("S12 %h %h | hm %h | one %h", lane[0].nest[1][2], lane[0].nest[1][5], hm[0].nest[1][2], one.nest[1][2]);
    lane[0].cube[1][0][2] = 32'h12; one.cube[1][0][2] = 32'h13;
    $display("S25 %h | one %h", lane[0].cube[1][0][2], one.cube[1][0][2]);
    lane[1].payload = '{1,2,3,4,5,6}; $display("WA %h %h", lane[1].payload[0], lane[1].payload[5]);
    lane[2].payload = lane[1].payload; $display("WB %h %h", lane[2].payload[0], lane[2].payload[5]);
    $display("SZ %0d %0d", $size(lane[0].payload), $size(lane[0].nest, 2));
    lane[1].payload[3] <= #2 32'hbeef; #3 $display("NB %h", lane[1].payload[3]);
    #20 $finish;
  end
endmodule
module nest_owner;
  lane_if lane [2] ();
  logic [31:0] w [6];
  logic [31:0] n2;
  always_comb n2 = lane[1].nest[0][3];
  initial begin
    for (int j = 0; j < 6; j++) lane[1].payload[j] <= 32'h60 + j;
    lane[1].nest[0][3] = 32'h63;
    #1 w = lane[1].payload;
    $display("N %h %h %h %0d", w[2], w[5], n2, $size(lane[1].payload));
    foreach (lane[1].payload[i]) if (i == 4) $display("Nfe %0d %h", i, lane[1].payload[i]);
  end
endmodule
module comb_reads;
  lane_if lane [3] ();
  lane_if one ();
  logic [31:0] s12, s12one, s25, s25one, s48, s48one;
  always_comb s12 = lane[0].nest[1][2];
  always_comb s12one = one.nest[1][2];
  always_comb s25 = lane[0].cube[1][0][2];
  always_comb s25one = one.cube[1][0][2];
  always_comb begin
    s48 = 0;
    foreach (lane[0].payload[i]) if (i == 2) s48 = lane[0].payload[i];
  end
  always_comb begin
    s48one = 0;
    foreach (one.payload[i]) if (i == 2) s48one = one.payload[i];
  end
  initial begin
    for (int j = 0; j < 6; j++) begin lane[0].nest[1][j] = 32'h10 + j; one.nest[1][j] = 32'h10 + j; lane[0].payload[j] = 32'h30 + j; one.payload[j] = 32'h30 + j; end
    lane[0].cube[1][0][2] = 32'h12; one.cube[1][0][2] = 32'h12;
    #7 $display("c0 %h %h | %h %h | %h %h", s12, s12one, s25, s25one, s48, s48one);
    for (int j = 0; j < 6; j++) begin lane[0].nest[1][j] = 32'hA0 + j; one.nest[1][j] = 32'hA0 + j; lane[0].payload[j] = 32'hB0 + j; one.payload[j] = 32'hB0 + j; end
    lane[0].cube[1][0][2] = 32'hA2; one.cube[1][0][2] = 32'hA2;
    #7 $display("c1 %h %h | %h %h | %h %h", s12, s12one, s25, s25one, s48, s48one);
    foreach (lane[0].payload[i]) if (i == 2) $display("fe %0d", i);
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "N 00000062 00000065 00000063 6",
    "Nfe 4 00000064",
    "S22 00000012 00000015 | one 00000022",
    "S23 00000012 | one 00000022",
    "S43 00000012 00000015",
    "S43one 00000022 00000025",
    "S48 2 00000012",
    "S48one 2 00000022",
    "S12 00000032 00000035 | hm 00000042 | one 00000052",
    "S25 00000012 | one 00000013",
    "WA 00000001 00000006",
    "WB 00000001 00000006",
    "SZ 6 6",
    "NB 0000beef",
    "c0 00000012 00000012 | 00000012 00000012 | 00000032 00000032",
    "c1 000000a2 000000a2 | 000000a2 000000a2 | 000000b2 000000b2",
    "fe 2",
];

#[test]
fn array_member_of_instance_array_element_is_an_array() {
    let actual: Vec<_> = xezim::simulate(DESIGN, 1_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect();
    assert_eq!(actual, EXPECTED);
}
