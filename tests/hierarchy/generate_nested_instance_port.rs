//! IEEE 1800-2017 §23.6, §27.4, §27.5: a relative reference to an instance
//! declared in a nested generate (`if` / `for` / `if`) names that iteration's
//! instance. `tag_bank.key_i` from the innermost block was rewritten only
//! through the outer scopes, so the read was x while the absolute path saw
//! the driven port.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn relative_port_read_inside_nested_generate_scopes() {
    let src = r#"
module prim_like (
  input logic [31:0] key_i
);
endmodule
module top;
  logic [31:0] key_q;
  initial key_q = 32'h00001111;
  if (1) begin : gen_rams
    for (genvar way = 0; way < 2; way++) begin : gen_rams_inner
      if (1) begin : gen_scramble_rams
        prim_like tag_bank (.key_i(key_q + way));
        initial begin
          #1;
          if (tag_bank.key_i !== 32'h00001111 + way)
            $display("FAIL rel %0d %h", way, tag_bank.key_i);
          else
            $display("PASS rel %0d", way);
        end
      end
    end
  end
  initial begin
    #2;
    if (gen_rams.gen_rams_inner[0].gen_scramble_rams.tag_bank.key_i !== 32'h00001111)
      $display("FAIL abs %h", gen_rams.gen_rams_inner[0].gen_scramble_rams.tag_bank.key_i);
    else
      $display("PASS abs");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 50).expect("sim");
    let msgs = messages(&sim);
    for want in ["PASS rel 0", "PASS rel 1", "PASS abs"] {
        assert!(
            msgs.iter().any(|m| m == want),
            "missing {want}; output: {msgs:?}"
        );
    }
    assert!(
        msgs.iter().all(|m| !m.starts_with("FAIL")),
        "output: {msgs:?}"
    );
}

/// The same relative port, read from a concurrent assertion rather than an
/// initial block.
#[test]
fn relative_port_read_in_nested_generate_assertion() {
    let src = r#"
module prim_like (
  input logic clk_i,
  input logic key_valid_i,
  input logic [31:0] key_i
);
endmodule
module top;
  logic clk = 0;
  logic rst_ni = 0;
  logic vi = 0;
  logic [31:0] ki = 0;
  logic [31:0] kq;
  logic vq;
  always #1 clk = ~clk;
  always_ff @(posedge clk or negedge rst_ni) begin
    if (!rst_ni) begin
      kq <= 32'h1;
      vq <= 1'b0;
    end else begin
      if (vi) kq <= ki;
      vq <= vi;
    end
  end
  if (1) begin : gen_rams
    for (genvar way = 0; way < 2; way++) begin : gen_rams_inner
      if (1) begin : gen_scramble_rams
        prim_like tag_bank (
          .clk_i(clk),
          .key_valid_i(vq),
          .key_i(kq + way)
        );
        logic [31:0] sampled;
        always_ff @(posedge clk or negedge rst_ni) begin
          if (!rst_ni) sampled <= 'x;
          else if (vi) sampled <= ki;
        end
        assert property (@(posedge clk) disable iff (!rst_ni)
          vi |-> ##[0:10] tag_bank.key_valid_i && (tag_bank.key_i == sampled + way))
        else $display("FAIL hier %0d", way);
      end
    end
  end
  initial begin
    repeat (2) @(posedge clk);
    @(negedge clk);
    rst_ni = 1;
    @(negedge clk);
    ki = 32'h1111;
    vi = 1;
    @(negedge clk);
    vi = 0;
    repeat (12) @(posedge clk);
    $display("DONE");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 200).expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "DONE"),
        "simulation did not finish; output: {msgs:?}"
    );
    assert!(
        msgs.iter().all(|m| !m.starts_with("FAIL")),
        "output: {msgs:?}"
    );
}

/// The concurrent assertion is a generate item of a module instantiated below
/// the top, so it is renamed with the instance, not as a top-level item. Its
/// property and its action block both read the instance port.
#[test]
fn relative_port_read_in_nested_generate_assertion_below_top() {
    let src = r#"
module leaf (
  input logic v_i
);
endmodule
module dut (
  input logic clk,
  input logic v
);
  logic one;
  always_ff @(posedge clk) one <= 1'b1;
  for (genvar i = 0; i < 2; i++) begin : g_loop
    if (1) begin : g_on
      leaf u_leaf (.v_i(one));
      assert property (@(posedge clk) v |-> u_leaf.v_i)
        $display("PASS %0d %b", i, u_leaf.v_i);
      else
        $display("FAIL %0d %b", i, u_leaf.v_i);
    end
  end
endmodule
module top;
  logic clk = 0;
  logic v = 0;
  always #5 clk = ~clk;
  dut u (.clk(clk), .v(v));
  initial begin
    repeat (2) @(negedge clk);
    v = 1;
    @(negedge clk);
    v = 0;
    @(negedge clk);
    $display("DONE");
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("sim");
    let msgs = messages(&sim);
    for want in ["PASS 0 1", "PASS 1 1", "DONE"] {
        assert!(
            msgs.iter().any(|m| m == want),
            "missing {want}; output: {msgs:?}"
        );
    }
    assert!(
        msgs.iter().all(|m| !m.starts_with("FAIL")),
        "output: {msgs:?}"
    );
}
