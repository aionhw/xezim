//! IEEE 1800-2023 §7.4.1, §7.4.5, §11.5.1: follow-ups to packed selects
//! inside unpacked array elements in compiled code (#985). Expected values
//! are from the reference simulator.

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

/// §11.5.1: an x or z literal index into a packed element reads x, in a
/// procedure and in a continuous assignment; it was folded to index 0.
#[test]
fn xz_literal_index_into_a_packed_element_reads_x() {
    let msgs = messages(
        r#"module t;
  logic clk = 0;
  logic [1:0][7:0] d [2];
  logic [1:0][7:0] pk;
  logic [7:0] r1, r2;
  wire [7:0] c1 = d[1]['x];
  wire [7:0] c2 = pk[1'bz];
  always @(posedge clk) begin
    r1 = d[1]['x];
    r2 = pk['x];
  end
  initial begin
    d[1] = 16'hc3a5;
    pk = 16'h5a5a;
    #1 clk = 1;
    #1 $display("r1=%h r2=%h c1=%h c2=%h", r1, r2, c1, c2);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["r1=xx r2=xx c1=xx c2=xx"]);
}

/// §7.4.1, §7.4.5: an element of a three-dimensional unpacked array read at
/// run-time indices, whole, by bit, by part-select, or by packed element, in
/// an `always` block. It read bits of the array's placeholder.
#[test]
fn three_dimensional_elements_at_run_time_indices() {
    let msgs = messages(
        r#"module t;
  logic clk = 0;
  logic [15:0] a3 [2][2][2];
  logic [1:0][7:0] p3 [2][0:1][2];
  int i, j, k, b;
  logic [15:0] r1;
  logic r2;
  logic [3:0] r3;
  logic [7:0] r4, r5;
  wire [15:0] c1 = a3[i][j][k];
  always @(posedge clk) begin
    r1 = a3[i][j][k];
    r2 = a3[i][j][k][b];
    r3 = a3[i][1][k][b +: 4];
    r4 = p3[i][j][k][1];
    r5 = p3[1][j][1][0];
  end
  initial begin
    a3[1][1][1] = 16'h1234;
    p3[1][1][1] = 16'habcd;
    i = 1; j = 1; k = 1; b = 4;
    #1 clk = 1;
    #1 $display("r1=%h r2=%b r3=%h r4=%h r5=%h c1=%h", r1, r2, r3, r4, r5, c1);
  end
endmodule"#,
    );
    assert_lines(&msgs, &["r1=1234 r2=1 r3=3 r4=ab r5=cd c1=1234"]);
}
