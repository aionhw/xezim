//! §10.4.2, §7.6: a nonblocking assignment to a row of a multi-dimensional
//! unpacked array (`q[1] <= x`) updates every element of that row. Each
//! element is its own signal, so the single update queued against `q[1]`
//! landed on no storage and the row kept its old value.

use xezim::simulate;

fn run(src: &str) -> Vec<String> {
    let sim = simulate(src, 200).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn row_nonblocking_assign_updates_each_element() {
    let msgs = run(r#"
module top;
  logic clk = 0;
  logic [3:0] in_i [2];
  logic [3:0] a [2][2], b [2][2], c [2][2], e [2][2], f [2][2], k [2][1:0];
  always #5 clk = ~clk;
  always @(posedge clk) begin
    a[0] <= '{default: 0};
    b[1] <= in_i;
    c[0] <= '{4'h1, 4'h2};
    e[1] <= e[0];
    f[0][1] <= 4'h7;
    k[0] <= '{0: 4'h1, 1: 4'h2};
  end
  initial begin
    in_i[0] = 4'h3; in_i[1] = 4'h5;
    e[0][0] = 4'h3; e[0][1] = 4'h4;
    #6 $display("a=%h%h b=%h%h c=%h%h e=%h%h f=%h k=%h%h",
                a[0][0], a[0][1], b[1][0], b[1][1], c[0][0], c[0][1],
                e[1][0], e[1][1], f[0][1], k[0][0], k[0][1]);
    $finish;
  end
endmodule
"#);
    assert_eq!(msgs, ["a=00 b=35 c=12 e=34 f=7 k=12"]);
}

#[test]
fn row_nonblocking_delay_line_through_row_port() {
    let msgs = run(r#"
module tail(input logic [3:0] p [2], output logic [3:0] s0, output logic [3:0] s1);
  assign s0 = p[0]; assign s1 = p[1];
endmodule
module top;
  logic clk = 0; logic rst_n = 0;
  always #5 clk = ~clk;
  logic [3:0] ic [2];
  logic [3:0] q [2][2];
  logic [3:0] s0, s1;
  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      for (int i = 0; i < 2; i++) q[i] <= '{default: 0};
    end else begin
      for (int i = 0; i < 1; i++) q[i] <= q[i+1];
      q[1] <= ic;
    end
  end
  tail u (.p(q[0]), .s0(s0), .s1(s1));
  initial begin
    ic[0] = 4'h3; ic[1] = 4'h5;
    #12 rst_n = 1;
    repeat (4) begin
      @(posedge clk); #1 $display("s=%h%h", s0, s1);
      ic[0] = ic[0] + 1; ic[1] = ic[1] + 1;
    end
    $finish;
  end
endmodule
"#);
    assert_eq!(msgs, ["s=00", "s=35", "s=46", "s=57"]);
}
