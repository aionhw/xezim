//! §10.6.2: a `force` of part of a net holds those bits and leaves the others
//! to the net's continuous drivers, and their `release` re-drives the bits
//! from them. The drivers of a partially forced net are found through the
//! comb writers index; this forces and releases parts of nets with a computed
//! driver, a concatenation driver and a constant driver on every clock.
//! Expected values from the reference simulator.

#[test]
fn partial_forces_each_clock_follow_every_writer() {
    let src = r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] a = 4'h3;
  logic [3:0] f = 4'h0;
  wire [7:0] w1 = {a, a + 4'h1};
  wire [7:0] w2;
  assign w2 = {~a, 4'h6};
  wire [7:0] w3 = 8'h5a;
  always @(posedge clk) begin
    a <= a + 4'h1;
    f <= f + 4'h2;
  end
  initial begin
    repeat (3) begin
      @(negedge clk);
      force w1[3:0] = f;
      force w2[7:4] = 4'h0;
      force w3[1:0] = 2'b11;
      $display("f %h %h %h", w1, w2, w3);
      @(posedge clk) #1 $display("g %h %h %h", w1, w2, w3);
      release w1[3:0];
      force w1[7:6] = 2'b00;
      release w2[7:4];
      release w3[1:0];
      $display("r %h %h %h", w1, w2, w3);
      release w1[7:6];
      $display("s %h", w1);
    end
    $finish;
  end
endmodule
"#;
    let result = xezim::simulate(src, 1_000).unwrap();
    let lines: Vec<String> = result
        .output
        .iter()
        .map(|entry| entry.message.clone())
        .collect();
    let want = [
        "f 42 06 5b",
        "g 54 06 5b",
        "r 16 a6 5a",
        "s 56",
        "f 54 06 5b",
        "g 66 06 5b",
        "r 27 96 5a",
        "s 67",
        "f 66 06 5b",
        "g 78 06 5b",
        "r 38 86 5a",
        "s 78",
    ];
    let got: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|l| l.len() > 1 && l.as_bytes()[1] == b' ')
        .collect();
    assert_eq!(got, want, "in {lines:?}");
}
