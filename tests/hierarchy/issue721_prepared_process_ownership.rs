//! #721: moving prepared process bodies into shared ownership must retain
//! their instance bindings, generated scopes and scheduling region.
//! IEEE 1800-2023 §§9.2.1, 9.2.2, 23.10, 24.3 and 27.5.

use xezim::simulate;

fn messages(source: &str) -> Vec<String> {
    simulate(source, 1_000)
        .expect("simulate prepared processes")
        .output
        .iter()
        .map(|line| line.message.clone())
        .filter(|line| line.starts_with("T|"))
        .collect()
}

/// §§9.2.1, 9.2.2, 23.10, 27.5: equal specializations share source bodies,
/// but their state and named-block locals belong to their own instances.
/// A different specialization must retain its selected generate branch.
#[test]
fn shared_and_distinct_specializations_keep_generated_processes() {
    let source = r#"
module leaf #(parameter int P = 2)(input logic clk, output int result);
  if (P % 2) begin : odd
    initial begin : init_value
      int seed;
      seed = P;
      result = seed + 100;
    end
    always @(posedge clk) begin : step_value
      int delta;
      delta = P;
      result <= result + delta;
    end
  end else begin : even
    initial begin : init_value
      int seed;
      seed = P;
      result = seed + 200;
    end
    always @(posedge clk) begin : step_value
      int delta;
      delta = P * 2;
      result <= result + delta;
    end
  end
endmodule
module tb;
  logic clk = 0;
  int a, b, c;
  leaf #(2) first(clk, a);
  leaf #(2) second(clk, b);
  leaf #(3) third(clk, c);
  initial begin
    #1 clk = 1;
    #1 clk = 0;
    #1 clk = 1;
    #1 $display("T|values=%0d,%0d,%0d", a, b, c);
    $finish;
  end
endmodule
"#;
    assert_eq!(messages(source), ["T|values=210,210,109"]);
}

/// §§6.11.2 and 9.2.2.2: shared always_comb sources preserve four-state
/// values, execute at time zero and react independently to each actual port.
#[test]
fn shared_always_comb_processes_keep_ports_and_four_state_values() {
    let source = r#"
module swap_pairs(input logic [3:0] value, output logic [3:0] result);
  always_comb begin : compute
    logic [3:0] temporary;
    temporary = value;
    result = {temporary[1:0], temporary[3:2]};
  end
endmodule
module tb;
  logic [3:0] first_input = 4'b10xx;
  logic [3:0] second_input = 4'b0110;
  wire [3:0] first_result, second_result;
  swap_pairs first(first_input, first_result);
  swap_pairs second(second_input, second_result);
  initial begin
    #1 $display("T|initial=%b,%b", first_result, second_result);
    first_input = 4'b1100;
    #1 $display("T|changed=%b,%b", first_result, second_result);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        messages(source),
        ["T|initial=xx10,1001", "T|changed=0011,1001"]
    );
}

/// §§4.4.2.6 and 24.3: program initial procedures run in the Reactive region,
/// so both instances observe the module's NBA update at the triggering edge.
#[test]
fn shared_program_initials_observe_nba_updates() {
    let source = r#"
program observer(input logic clk, input int value);
  initial begin
    @(posedge clk);
    $display("T|program=%0d", value);
  end
endprogram
module tb;
  logic clk = 0;
  int value = 0;
  always @(posedge clk) value <= 7;
  observer first(clk, value);
  observer second(clk, value);
  initial begin
    #1 clk = 1;
    #10 $display("T|program termination failed");
  end
endmodule
"#;
    assert_eq!(messages(source), ["T|program=7", "T|program=7"]);
}
