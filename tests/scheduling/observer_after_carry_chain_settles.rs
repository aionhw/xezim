//! IEEE 1800-2023 §4.5, §9.2.2.2: a level-sensitive `always` block is a
//! process of its own. When nothing else reads what it writes (an immediate
//! assertion, a `$display`, an error counter), it can run once the continuous
//! assignments it reads have settled, and then it sees only values that
//! coexist. A ripple-carry adder built from continuous assignments is a cycle
//! in the settle (`c` is one vector written bit by bit while it is read), and
//! the settle ran such a block inside that cycle, once per carry step: the
//! assertion failed on intermediate sums (#1500). Expected values were checked
//! on the reference simulator.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|") || l.contains("failed"))
        .collect()
}

#[test]
fn issue_repro_assertion_sees_only_settled_sums() {
    let got = run(r#"
module top;
  logic [7:0] a, b, s;
  logic cin;
  logic [8:0] c;
  assign c[0] = cin;
  for (genvar i = 0; i < 8; i++) begin : rc
    assign c[i+1] = (a[i] & b[i]) | ((a[i] ^ b[i]) & c[i]);
  end
  assign s = a ^ b ^ c[7:0];
  always @(*) assert ({c[8], s} == a + b + cin)
    else $display("star failed at %0t: s=%h c8=%b", $time, s, c[8]);
  initial begin
    a = 0; b = 0; cin = 0; #1;
    a = 8'h7f; b = 8'h01; #1;
    a = 8'hff; cin = 1; #1;
    a = 8'h55; b = 8'haa; cin = 0; #1;
    b = 8'hab; #1;
    $display("T|done");
    $finish;
  end
endmodule
"#);
    assert_eq!(got, ["T|done"]);
}

#[test]
fn always_comb_list_display_and_counting_observers() {
    let got = run(r#"
module top;
  logic [7:0] a, b, s;
  logic cin;
  logic [8:0] c;
  int n_comb, n_list, n_w;
  logic [8:0] chk;
  assign c[0] = cin;
  for (genvar i = 0; i < 8; i++) begin : rc
    assign c[i+1] = (a[i] & b[i]) | ((a[i] ^ b[i]) & c[i]);
  end
  assign s = a ^ b ^ c[7:0];
  always_comb assert ({c[8], s} == a + b + cin) else if ($time > 0) n_comb++;
  always @(a or b or cin or s or c) assert ({c[8], s} == a + b + cin) else n_list++;
  always @(*) $display("T|disp %0t s=%h c8=%b", $time, s, c[8]);
  always @(*) begin
    chk = {c[8], s};
    assert (chk == a + b + cin) else n_w++;
  end
  initial begin
    a = 0; b = 0; cin = 0; #1;
    a = 8'h7f; b = 8'h01; #1;
    a = 8'hff; cin = 1; #1;
    a = 8'h55; b = 8'haa; cin = 0; #1;
    b = 8'hab; #1;
    $display("T|comb=%0d list=%0d w=%0d", n_comb, n_list, n_w);
    $finish;
  end
endmodule
"#);
    assert_eq!(
        got,
        [
            "T|disp 0 s=00 c8=0",
            "T|disp 1 s=80 c8=0",
            "T|disp 2 s=01 c8=1",
            "T|disp 3 s=ff c8=0",
            "T|disp 4 s=00 c8=1",
            "T|comb=0 list=0 w=0",
        ]
    );
}

#[test]
fn inputs_from_a_clocked_block_reach_the_observer_settled() {
    let got = run(r#"
module top;
  logic [3:0] a, b, s;
  logic [4:0] c;
  logic clk = 0;
  int nf;
  assign c[0] = 1'b0;
  for (genvar i = 0; i < 4; i++) begin : rc
    assign c[i+1] = (a[i] & b[i]) | ((a[i] ^ b[i]) & c[i]);
  end
  assign s = a ^ b ^ c[3:0];
  always @(*) if ({c[4], s} !== a + b) begin nf++; $display("T|bad %0t a=%h b=%h s=%h", $time, a, b, s); end
  always #5 clk = ~clk;
  always @(posedge clk) begin a <= a + 4'd7; b <= b + 4'd3; end
  initial begin
    a = 0; b = 0;
    #100;
    $display("T|nf=%0d s=%h", nf, s);
    $finish;
  end
endmodule
"#);
    assert_eq!(got, ["T|nf=0 s=4"]);
}
