#[test]
fn descending_loop_return_discards_rolled_back_fixups() {
    let src = include_str!("inline_return_rollback.sv");
    let sim = xezim::simulate(src, 20).expect("descending loop must not crash during inlining");
    assert!(
        sim.output
            .iter()
            .any(|line| line.message == "INLINE_RETURN_ROLLBACK_PASS"),
        "early and fall-through returns must retain their function semantics"
    );
}

#[test]
fn long_loop_return_discards_rolled_back_fixups() {
    // Exercise the unroll budget independently of signed descending bounds.
    let src = include_str!("inline_return_rollback.sv")
        .replace("int i = 1; i >= 0; i--", "int i = 0; i < 1024; i++")
        .replace("x[i]", "x[i & 1]");
    let sim = xezim::simulate(&src, 20).expect("unroll budget must allow fallback without a crash");
    assert!(
        sim.output
            .iter()
            .any(|line| line.message == "INLINE_RETURN_ROLLBACK_PASS")
    );
}

#[test]
fn failed_inline_preserves_caller_loop_fixups() {
    // IEEE 1800-2023 §12.8: the caller's break and continue target its own
    // loop, regardless of how the called function is compiled.
    let src = r#"
`timescale 1ns/1ns
module top;
  bit clk = 0;
  int result = 0;
  int visits = 0;
  int edges = 0;

  function automatic int highest_bit(input logic [1:0] x);
    for (int j = 1; j >= 0; j--)
      if (x[j]) return j + 1;
    return 0;
  endfunction

  always #1 clk = ~clk;
  always @(posedge clk) begin
    int i;
    i = 0;
    while (i < 4) begin
      if (i == 3) break;
      i = i + 1;
      if (i == 1) continue;
      result = highest_bit(1);
      visits = visits + 1;
    end
    edges = edges + 1;
  end

  initial begin
    #4;
    if (result !== 1 || visits !== 4 || edges !== 2)
      $fatal(1, "unexpected result=%0d visits=%0d edges=%0d", result, visits, edges);
    $display("CALLER_LOOP_ROLLBACK_PASS");
    $finish;
  end
endmodule
"#;
    let task_src = src
        .replace(
            "function automatic int highest_bit(input logic [1:0] x);",
            "task automatic highest_bit(input logic [1:0] x, output int result);",
        )
        .replace("return j + 1;", "begin result = j + 1; return; end")
        .replace("return 0;", "result = 0;")
        .replace("endfunction", "endtask")
        .replace("result = highest_bit(1);", "highest_bit(1, result);");
    for src in [src, task_src.as_str()] {
        let sim =
            xezim::simulate(src, 20).expect("caller loop must terminate after failed inlining");
        assert!(
            sim.output
                .iter()
                .any(|line| line.message == "CALLER_LOOP_ROLLBACK_PASS")
        );
    }
}

#[test]
fn unsupported_inlined_call_discards_partial_control_flow() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  logic clk = 0;
  logic [31:0] state_word = 0;
  logic [63:0] result_word;
  integer edge_count = 0;

  always #1 clk = ~clk;

  function automatic logic [63:0] make_word(input logic [31:0] source_word);
    logic [63:0] temporary_word;
    temporary_word = 0;
    if (source_word[0]) temporary_word[39:36] = 6;
    return temporary_word;
  endfunction

  always @(posedge clk) begin
    edge_count <= edge_count + 1;
    state_word <= state_word + 1;
    result_word <= make_word(state_word);
  end

  initial begin
    #100;
    if (edge_count != 50 || state_word != 50 || result_word != 64'h6000000000)
      $fatal(1, "unexpected state count=%0d state=%0d result=%h",
             edge_count, state_word, result_word);
    $display("ROLLBACK_PASS");
    $finish;
  end
endmodule
"#;

    let sim = xezim::simulate(src, 200).expect("simulation must terminate");
    assert!(
        sim.output
            .iter()
            .any(|line| line.message == "ROLLBACK_PASS")
    );
}
