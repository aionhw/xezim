//! §20.7, §23.6: an array query over a generate block's variable from outside
//! the block (`$size(g[0].arr)`), at the top and in an instance. Expected
//! lines are the reference simulator's.
use xezim::simulate;

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

#[test]
fn an_array_query_over_a_blocks_variable_from_outside_the_block() {
    let src = r#"
module child;
  for (genvar i = 0; i < 2; i++) begin : g logic [7:0] arr [4]; end
  localparam int OUT = $size(g[0].arr);
  initial $display("T|child OUT=%0d bits=%0d", OUT, $bits(g[1].arr));
endmodule
module top;
  for (genvar i = 0; i < 2; i++) begin : g
    logic [7:0] arr [4];
    if (1) begin : n int deep [3][5]; end
  end
  localparam int OUT = $size(g[0].arr);
  localparam int D2 = $size(g[1].n.deep, 2);
  child c();
  initial $display("T|top OUT=%0d D2=%0d", OUT, D2);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|child OUT=4 bits=32", "T|top OUT=4 D2=5"]);
}
