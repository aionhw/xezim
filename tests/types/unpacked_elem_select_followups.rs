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
