//! §7.4.6, §10.4.2: sample overlapping unpacked slices before any NBA store.

#[test]
fn overlapping_unpacked_slice_nba_samples_elements_in_order() {
    let sim = xezim::simulate(
        r#"
module top;
  bit clk = 0;
  logic a[2:0], b[0:2];
  assign a[0] = 1'b1;
  assign b[0] = 1'b1;
  always @(posedge clk) a[2:1] <= a[1:0];
  always @(posedge clk) b[1:2] <= b[0:1];

  initial begin
    repeat (2) begin
      #1 clk = 1;
      #1 $display("a=%b%b%b b=%b%b%b", a[2], a[1], a[0], b[2], b[1], b[0]);
      clk = 0;
    end
    $finish;
  end
endmodule
"#,
        1_000_000,
    )
    .expect("simulation");
    let result: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.trim())
        .filter(|line| line.starts_with("a="))
        .collect();
    assert_eq!(result, ["a=x11 b=x11", "a=111 b=111"]);
}
