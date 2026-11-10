//! §9.6.2: `disable` of a block by path stops that block, not a block of the
//! same label in another generate scope or loop iteration, from the module,
//! from a block of the same label and from a parent instance. Every expected
//! line is the reference simulator's.
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
fn a_disable_by_path_stops_only_that_blocks_namesake() {
    let src = r#"
module top;
  if (1) begin : g1
    initial begin : blk #10 $display("T|g1.blk"); end
  end
  if (1) begin : g2
    initial begin : blk #10 $display("T|g2.blk"); end
  end
  initial begin #1 disable g1.blk; #20 $display("T|done"); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|done", "T|g2.blk"]);
}

#[test]
fn a_disable_by_loop_path_stops_that_iterations_block() {
    let src = r#"
module top;
  for (genvar i = 0; i < 2; i++) begin : r
    initial begin : w #10 $display("T|r[%0d].w", i); end
  end
  initial begin #1 disable r[0].w; #20 $display("T|done"); end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|done", "T|r[1].w"]);
}

#[test]
fn a_disable_by_path_from_a_block_of_the_same_label_stops_the_other() {
    let src = r#"
module top;
  if (1) begin : g1
    initial begin : blk #5 $display("T|g1.blk"); end
  end
  initial begin : blk
    #1 disable g1.blk;
    $display("T|after");
    #10 $display("T|top.blk");
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|after", "T|top.blk"]);
}

#[test]
fn disables_by_path_inside_an_instance_and_from_its_parent() {
    let src = r#"
module child;
  if (1) begin : g1 initial begin : blk #10 $display("T|g1.blk"); end end
  if (1) begin : g2 initial begin : blk #10 $display("T|g2.blk"); end end
  for (genvar i = 0; i < 2; i++) begin : r
    initial begin : w #10 $display("T|r[%0d].w", i); end
  end
  initial begin #1 disable g1.blk; disable r[0].w; #20 $display("T|child done"); end
endmodule
module top; child u(); initial begin #2 disable u.g2.blk; end endmodule
"#;
    assert_eq!(t_lines(src), ["T|child done", "T|r[1].w"]);
}
