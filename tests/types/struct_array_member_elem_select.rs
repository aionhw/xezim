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

/// The same selects read in an `always` block and a continuous assignment,
/// at constant, run-time and x indices. The second index took one bit of the
/// first index's slice.
#[test]
fn compiled_reads_of_elements_of_a_packed_member() {
    let msgs = messages(
        r#"typedef logic [2:0] t3_t;
typedef struct packed { logic [15:0] pad; t3_t [3:0][7:0] f0; } s2_t;
typedef struct packed { logic [7:0] pad; logic [1:0][3:0][7:0] f0; } s3_t;
module t;
  s2_t sa [2];
  s3_t s3 [2];
  logic clk = 0;
  int i = 1, j = 3, k = 7;
  logic [2:0] r1, r2, r3;
  logic [7:0] r4, r5;
  logic r6;
  wire [2:0] c1 = sa[1].f0[3][7];
  wire [2:0] c2 = sa[i].f0[j][k];
  always @(posedge clk) begin
    r1 = sa[1].f0[3][7];
    r2 = sa[i].f0[j][k];
    r3 = sa[1].f0[1]['x];
    r4 = s3[1].f0[1][2];
    r5 = s3[i].f0[0][j - 2];
    r6 = s3[1].f0[1][2][3];
  end
  initial begin
    sa[1] = '0; sa[1].f0[3][7] = 3'd6;
    s3[1] = '0; s3[1].f0[1][2] = 8'h3c; s3[1].f0[0][1] = 8'ha5;
    #1 clk = 1;
    #1 $display("r1=%0d r2=%0d r3=%b r4=%h r5=%h r6=%b c1=%0d c2=%0d", r1, r2, r3, r4, r5, r6, c1, c2);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["r1=6 r2=6 r3=xxx r4=3c r5=a5 r6=1 c1=6 c2=6"]);
}

/// Two- and three-dimensional unpacked arrays of packed structs.
#[test]
fn compiled_reads_through_multi_dimensional_struct_arrays() {
    let msgs = messages(
        r#"typedef struct packed { logic [7:0] pad; logic [1:0][3:0][7:0] f0; } s3_t;
typedef logic [2:0] t3_t;
typedef struct packed { logic [15:0] pad; t3_t [3:0][7:0] f0; } s2_t;
module t;
  s3_t s3 [2][2];
  s2_t q3 [2][2][2];
  logic clk = 0;
  int i = 1, j = 2;
  logic [7:0] r1, r2;
  logic r3;
  logic [2:0] r4;
  always @(posedge clk) begin
    r1 = s3[1][0].f0[1][2];
    r2 = s3[i][0].f0[0][j - 1];
    r3 = s3[1][0].f0[1][2][3];
    r4 = q3[1][0][1].f0[3][7];
  end
  initial begin
    s3[1][0] = '0; s3[1][0].f0[1][2] = 8'h3c; s3[1][0].f0[0][1] = 8'ha5;
    q3[1][0][1] = '0; q3[1][0][1].f0[3][7] = 3'd5;
    #1 clk = 1;
    #1 $display("r1=%h r2=%h r3=%b r4=%0d s3=%h q=%0d", r1, r2, r3, r4, s3[1][0], q3[1][0][1].f0[3][7]);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["r1=3c r2=a5 r3=1 r4=5 s3=00003c00000000a500 q=5"]);
}
