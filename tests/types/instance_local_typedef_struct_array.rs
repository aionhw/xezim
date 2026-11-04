//! IEEE 1800-2017 §6.18, §23.10: a typedef declared inside a module is a
//! different type in every instance of that module. An inlined declaration
//! kept the BARE typedef name, which after inlining resolved to a 32-bit
//! default (or to the first instance's type), so a member write into an
//! element of `s_t [0:0] q` landed at the wrong bit once the struct exceeded
//! 32 bits — CVA6's ID stage never cleared `issue_n[0].valid` and the core
//! re-issued its first instruction forever.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn each_instance_resolves_its_own_local_typedef() {
    let sim = simulate(
        r#"
module m #(parameter int W = 8) (output logic [W:0] o);
  typedef struct packed { logic valid; logic [W-1:0] data; } s_t;
  s_t [0:0] q;
  initial begin q = '0; q[0].valid = 1; q[0].data = 'h11; end
  assign o = q;
endmodule
module top;
  logic [8:0] o8; logic [40:0] o40; logic [64:0] o64;
  m #(.W(8)) u8 (.o(o8));
  m #(.W(40)) u40 (.o(o40));
  m #(.W(64)) u64 (.o(o64));
  initial #1 $display("o8=%h o40=%h o64=%h", o8, o40, o64);
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(
        msgs.iter().any(|m| m == "o8=111 o40=10000000011 o64=10000000000000011"),
        "{msgs:?}"
    );
}

#[test]
fn issue_register_clear_in_an_instance() {
    // The CVA6 id_stage shape: clear a member of a struct array element
    // under an ack, then refill it from an assignment pattern.
    let sim = simulate(
        r#"
module idst (input logic clk, input logic rst_n, input logic ack, input logic fv, input logic [39:0] d,
             output logic ready, output logic valid_o);
  typedef struct packed { logic valid; logic [39:0] data; } issue_t;
  issue_t [0:0] issue_n, issue_q;
  assign valid_o = issue_q[0].valid;
  always_comb begin
    issue_n = issue_q;
    ready = 1'b0;
    if (ack) issue_n[0].valid = 1'b0;
    if (!issue_n[0].valid && fv) begin
      ready = 1'b1;
      issue_n[0] = '{1'b1, d};
    end
  end
  always_ff @(posedge clk) if (!rst_n) issue_q <= '0; else issue_q <= issue_n;
endmodule
module top;
  logic clk = 0, rst_n = 0, ack = 0, fv = 0, ready, valid_o; logic [39:0] d = 40'h11;
  idst u (.clk, .rst_n, .ack, .fv, .d, .ready, .valid_o);
  initial begin
    #1 clk = 1; #1 clk = 0; rst_n = 1; fv = 1; #1 clk = 1; #1 clk = 0; #1;
    $display("A ready=%b valid=%b", ready, valid_o);
    ack = 1; #1 $display("B ready=%b valid=%b", ready, valid_o);
    clk = 1; #1 clk = 0; fv = 0; #1 $display("C ready=%b valid=%b", ready, valid_o);
    clk = 1; #1 clk = 0; #1 $display("D ready=%b valid=%b", ready, valid_o);
  end
endmodule
"#,
        20,
    )
    .expect("sim");
    let msgs = messages(&sim);
    for want in ["A ready=0 valid=1", "B ready=1 valid=1", "C ready=0 valid=1", "D ready=0 valid=0"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want:?}: {msgs:?}");
    }
}
