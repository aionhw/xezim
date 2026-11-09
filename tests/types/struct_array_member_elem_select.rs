//! IEEE 1800-2023 §7.4.1, §7.2, §10.4.2: elements of a multi-dimensional
//! packed member of a struct element of an unpacked array (`sa[i].f0[j][k]`,
//! #984). Expected values are from the reference simulator.

use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn assert_lines(msgs: &[String], want: &[&str]) {
    for w in want {
        assert!(
            msgs.iter().any(|m| m.trim() == *w),
            "missing {w:?} in {msgs:#?}"
        );
    }
}

/// Blocking and nonblocking stores two and three indices below the member,
/// on one- and two-dimensional arrays of structs and typedef elements. They
/// were dropped, and a nonblocking store kept one bit.
#[test]
fn stores_to_elements_of_a_packed_member() {
    let msgs = messages(
        r#"typedef logic [2:0] t3_t;
typedef struct packed { logic [15:0] pad; t3_t [3:0][7:0] f0; } s2_t;
typedef struct packed { logic [15:0] pad; logic [3:0][7:0] f0; } s1_t;
typedef struct packed { logic [7:0] pad; logic [1:0][3:0][7:0] f0; } s3_t;
module t;
  s2_t sa [2];
  s1_t s1 [2];
  s3_t s3 [2][2];
  s2_t sn [2];
  logic clk = 0;
  int i = 1, j = 3, k = 7;
  always @(posedge clk) begin
    sn[i].f0[j][k] <= 3'd5;
    sn[i].f0[0][1] <= 3'd2;
  end
  initial begin
    sa[1] = '0; sa[1].f0[3][7] = 3'd6; sa[1].f0[i][k-1] = 3'd3;
    s1[1] = '0; s1[1].f0[3][7] = 1'b1; s1[1].f0[0][0] = 1'b1;
    s3[1][0] = '0; s3[1][0].f0[1][2][3] = 1'b1; s3[1][0].f0[0][1] = 8'h5a;
    sn[1] = '0;
    #1 clk = 1;
    #1;
    $display("sa=%h v=%0d %0d row=%h", sa[1], sa[1].f0[3][7], sa[i].f0[1][6], sa[1].f0[3]);
    $display("s1=%h s3=%h", s1[1], s3[1][0]);
    $display("sn=%h", sn[1]);
  end
endmodule"#,
    );
    assert_lines(
        &msgs,
        &[
            "sa=0000c000000000000c0000000000 v=6 3 row=c00000",
            "s1=000080000001 s3=000008000000005a00",
            "sn=0000a00000000000000000000010",
        ],
    );
}
