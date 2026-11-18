//! IEEE 1800-2023 §9.2, §9.3, §23.10: shared block syntax must
//! detach for parameter and instance rewrites without sharing runtime state.

fn passes(source: &str, marker: &str) {
    let run = xezim::simulate(source, 20).expect("independent block instances");
    assert!(
        run.output.iter().any(|line| line.message == marker),
        "{:?}",
        run.output
    );
}

#[test]
fn initial_and_always_bodies_keep_instance_parameters_and_names() {
    passes(
        r#"
module leaf #(parameter P = 1)(input logic clk, input logic [7:0] x, output logic [7:0] y);
  initial begin : init_value
    automatic logic [P:0] a;
    a = '1;
    #1; y = a;
  end
  always @(posedge clk) begin : update_value
    automatic int value;
    value = int'(x) + P;
    y <= value;
  end
endmodule
module top;
  logic clk = 0;
  logic [7:0] a, b;
  leaf #(.P(1)) u0(clk, 8'd10, a);
  leaf #(.P(3)) u1(clk, 8'd20, b);
  initial begin
    #2;
    if (a !== 3 || b !== 15) $fatal(1, "initial width leaked between instances");
    clk = 1; #1;
    if (a !== 11 || b !== 23) $fatal(1, "always reference leaked between instances");
    clk = 0; #1; clk = 1; #1;
    if (a !== 11 || b !== 23) $fatal(1, "reused body changed instance ownership");
    $display("PASS shared initial and always bodies");
  end
endmodule
"#,
        "PASS shared initial and always bodies",
    );
}

#[test]
fn parallel_bodies_keep_local_state_and_instance_rewrites() {
    passes(
        r#"
module leaf #(parameter P = 1)(output int q);
  initial begin : run
    int sum;
    sum = 0;
    fork : workers
      begin : first
        automatic int t;
        t = P + 1; #1; sum += t;
      end
      begin : second
        automatic int t;
        t = P + 2; #2; sum += t;
      end
    join
    q = sum;
  end
endmodule
module top;
  wire signed [31:0] a, b;
  leaf #(.P(1)) u0(a);
  leaf #(.P(5)) u1(b);
  initial begin
    #3;
    if (a !== 5 || b !== 13) $fatal(1, "fork state leaked between instances");
    $display("PASS shared fork bodies");
  end
endmodule
"#,
        "PASS shared fork bodies",
    );
}
