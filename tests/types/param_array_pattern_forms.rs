//! §10.9.1/§23.10: an unpacked array parameter initialized or overridden with
//! a replication (`'{3{5}}`), keyed items (`'{1: 9, default: 5}`) or a lone
//! `default:` gets one value per element. Every element read 0: the
//! initializer was split into its items and compared with the element count,
//! so any of these forms failed to register, and keyed items were placed by
//! position instead of by index. Expected values from the reference simulator.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_line(msgs: &[String], want: &str) {
    assert!(
        msgs.iter().any(|m| m == want),
        "missing {want:?} in {msgs:?}"
    );
}

#[test]
fn replication_and_default_initializers() {
    let msgs = messages(
        "typedef struct packed { bit [31:0] s; bit [1:0] x; } p_t;
module tb;
  localparam int N = 3;
  localparam int A1 [3] = '{ 3{5} };
  localparam int A2 [4] = '{ 2{5, 6} };
  localparam int A3 [3] = '{ default: 5 };
  localparam int A7 [3] = '{ N{8} };
  localparam byte BY [3] = '{ 3{8'hff} };
  localparam int signed SN [2] = '{ default: -2 };
  localparam p_t P1 [3] = '{ 3{ '{7, 2'd1} } };
  localparam p_t P2 [3] = '{ default: '{7, 2'd1} };
  initial begin
    $display(\"A1 %0d %0d %0d\", A1[0], A1[1], A1[2]);
    $display(\"A2 %0d %0d %0d %0d\", A2[0], A2[1], A2[2], A2[3]);
    $display(\"A3 %0d %0d %0d\", A3[0], A3[1], A3[2]);
    $display(\"A7 %0d %0d %0d\", A7[0], A7[1], A7[2]);
    $display(\"BY %0d %0d %0d\", BY[0], BY[1], BY[2]);
    $display(\"SN %0d %0d %0d\", SN[0], SN[1], SN[0] < 0);
    $display(\"P1 %0d/%0d %0d/%0d\", P1[0].s, P1[0].x, P1[2].s, P1[2].x);
    $display(\"P2 %0d/%0d %0d/%0d\", P2[0].s, P2[0].x, P2[2].s, P2[2].x);
  end
endmodule",
    );
    assert_line(&msgs, "A1 5 5 5");
    assert_line(&msgs, "A2 5 6 5 6");
    assert_line(&msgs, "A3 5 5 5");
    assert_line(&msgs, "A7 8 8 8");
    assert_line(&msgs, "BY -1 -1 -1");
    assert_line(&msgs, "SN -2 -2 1");
    assert_line(&msgs, "P1 7/1 7/1");
    assert_line(&msgs, "P2 7/1 7/1");
}

#[test]
fn keyed_items_land_at_their_index() {
    let msgs = messages(
        "typedef struct packed { bit [31:0] s; bit [1:0] x; } p_t;
module tb;
  localparam int K = 1;
  localparam int A4 [3] = '{ K: 9, default: 5 };
  localparam int A5 [2:0] = '{ 0: 7, default: 1 };
  localparam int A8 [1:3] = '{ 3: 4, default: 2 };
  localparam int MIX [4] = '{ 0: 1, 3: 4, default: 9 };
  localparam p_t P3 [2] = '{ 1: '{9, 2'd2}, default: '{7, 2'd1} };
  initial begin
    $display(\"A4 %0d %0d %0d\", A4[0], A4[1], A4[2]);
    $display(\"A5 %0d %0d %0d\", A5[2], A5[1], A5[0]);
    $display(\"A8 %0d %0d %0d\", A8[1], A8[2], A8[3]);
    $display(\"MIX %0d %0d %0d %0d\", MIX[0], MIX[1], MIX[2], MIX[3]);
    $display(\"P3 %0d/%0d %0d/%0d\", P3[0].s, P3[0].x, P3[1].s, P3[1].x);
  end
endmodule",
    );
    assert_line(&msgs, "A4 5 9 5");
    assert_line(&msgs, "A5 1 1 7");
    assert_line(&msgs, "A8 2 2 4");
    assert_line(&msgs, "MIX 1 9 9 4");
    assert_line(&msgs, "P3 7/1 9/2");
}

#[test]
fn nested_arrays_package_and_body_params() {
    let msgs = messages(
        "package pk;
  localparam int PB [4] = '{2{1, 2}};
  localparam int PC [3] = '{2: 30, default: 10};
endpackage
module sub #(parameter int N = 2) ();
  localparam int BL [N] = '{default: N * 3};
  localparam int BK [N] = '{N-1: 7, default: 1};
  initial #1 $display(\"%m BL %0d %0d BK %0d %0d\", BL[0], BL[N-1], BK[0], BK[N-1]);
endmodule
module tb;
  localparam int M1 [2][3] = '{ default: 5 };
  localparam int M2 [2][3] = '{ 2{ '{1, 2, 3} } };
  localparam int M4 [2][3] = '{ 1: '{default: 9}, default: '{3{2}} };
  localparam int D3 [2][2][2] = '{ default: 3 };
  localparam int DK [1:0][0:1] = '{ 1: '{1: 4, default: 5}, default: '{default: 6} };
  sub #(.N(3)) s3 ();
  initial begin
    $display(\"M1 %0d %0d %0d %0d\", M1[0][0], M1[0][2], M1[1][0], M1[1][2]);
    $display(\"M2 %0d %0d %0d %0d\", M2[0][0], M2[0][2], M2[1][0], M2[1][2]);
    $display(\"M4 %0d %0d %0d %0d\", M4[0][0], M4[0][2], M4[1][0], M4[1][2]);
    $display(\"D3 %0d %0d\", D3[0][0][0], D3[1][1][1]);
    $display(\"DK %0d %0d %0d %0d\", DK[1][0], DK[1][1], DK[0][0], DK[0][1]);
    $display(\"PK %0d %0d %0d %0d\", pk::PB[2], pk::PB[3], pk::PC[0], pk::PC[2]);
    $display(\"SZ %0d %0d\", $size(M2), $size(D3));
  end
endmodule",
    );
    assert_line(&msgs, "M1 5 5 5 5");
    assert_line(&msgs, "M2 1 3 1 3");
    assert_line(&msgs, "M4 2 2 9 9");
    assert_line(&msgs, "D3 3 3");
    assert_line(&msgs, "DK 5 4 6 6");
    assert_line(&msgs, "PK 1 2 10 30");
    assert_line(&msgs, "SZ 2 2");
    assert_line(&msgs, "tb.s3 BL 9 9 BK 1 7");
}

#[test]
fn instance_overrides_with_pattern_forms() {
    let msgs = messages(
        "typedef struct packed { bit [31:0] s; bit [1:0] x; } p_t;
module child #(parameter int P [3] = '{0, 0, 0}, parameter int M [2][2] = '{'{0,0},'{0,0}},
               parameter p_t S [2] = '{'{1, 2'd1}, '{1, 2'd1}}, parameter int W = 3,
               parameter int R [W] = '{W{9}}) ();
  initial #1 $display(\"%m P %0d %0d %0d M %0d %0d %0d %0d S %0d/%0d %0d/%0d R %0d %0d\",
                      P[0], P[1], P[2], M[0][0], M[0][1], M[1][0], M[1][1],
                      S[0].s, S[0].x, S[1].s, S[1].x, R[0], R[W-1]);
endmodule
module tb;
  localparam int K = 2;
  child #(.P('{1, 2, 3})) ord ();
  child #(.P('{3{6}})) rep ();
  child #(.P('{default: 7})) dfl ();
  child #(.P('{K: 5, default: 3})) key ();
  child #(.M('{default: 4})) m2d ();
  child #(.M('{1: '{2{8}}, default: '{0: 1, 1: 2}})) m2k ();
  child #(.S('{'{5, 2'd2}, '{6, 2'd3}})) sord ();
  child #(.S('{default: '{7, 2'd1}})) sdfl ();
  child #(.W(4)) wdef ();
  child #(.R('{3{K}})) wrep ();
endmodule",
    );
    assert_line(&msgs, "tb.ord P 1 2 3 M 0 0 0 0 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.rep P 6 6 6 M 0 0 0 0 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.dfl P 7 7 7 M 0 0 0 0 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.key P 3 3 5 M 0 0 0 0 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.m2d P 0 0 0 M 4 4 4 4 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.m2k P 0 0 0 M 1 2 8 8 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.sord P 0 0 0 M 0 0 0 0 S 5/2 6/3 R 9 9");
    assert_line(&msgs, "tb.sdfl P 0 0 0 M 0 0 0 0 S 7/1 7/1 R 9 9");
    assert_line(&msgs, "tb.wdef P 0 0 0 M 0 0 0 0 S 1/1 1/1 R 9 9");
    assert_line(&msgs, "tb.wrep P 0 0 0 M 0 0 0 0 S 1/1 1/1 R 2 2");
}
