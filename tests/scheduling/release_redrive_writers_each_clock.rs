//! §10.6.2: a `release` returns a net to the value of its continuous drivers
//! and leaves a variable at its last value. The drivers of a released net are
//! found through an index of the comb entries that write each signal, built at
//! the first release; this releases nets with a computed driver, a constant
//! driver and a part-selected concatenation, and a variable, once and then on
//! every clock, as a testbench that releases a signal each cycle does, and
//! forces and releases a second part of a net afterwards. Expected values are
//! Verilator 5.046's.

#[test]
fn releases_each_clock_redrive_every_writer() {
    let src = r#"
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [3:0] a = 4'h3;
  wire [3:0] w1 = a + 4'h1;
  wire [3:0] w2 = 4'h5;
  wire [7:0] w3;
  assign w3 = {a, ~a};
  logic [3:0] q;
  always @(posedge clk) begin
    a <= a + 4'h1;
    q <= w1;
  end
  initial begin
    #1 force w1 = 4'hf; force w2 = 4'h0; force w3[3:0] = 4'h0; force q = 4'h9;
    #10 $display("f %h %h %h %h", w1, w2, w3, q);
    release w1; release w2; release w3[3:0]; release q;
    $display("r %h %h %h %h", w1, w2, w3, q);
    repeat (3) begin
      @(negedge clk);
      release w1; release w2; release w3[3:0]; release q;
      $display("c %h %h %h %h", w1, w2, w3, q);
    end
    force w3[7:4] = 4'h0;
    @(negedge clk) $display("p %h", w3);
    release w3[7:4];
    $display("q %h", w3);
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
        "f f 0 40 9",
        "r 5 5 4b 9",
        "c 6 5 5a 5",
        "c 7 5 69 6",
        "c 8 5 78 7",
        "p 07",
        "q 87",
    ];
    for w in want {
        assert!(lines.iter().any(|l| l == w), "missing {w:?} in {lines:?}");
    }
}
