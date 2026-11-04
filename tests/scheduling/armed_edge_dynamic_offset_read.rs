//! IEEE 1800-2017 §9.4.2: an `always_ff` runs on every clock edge. The armed
//! edge mode runs a flop block only when one of its inputs was written since
//! its last run; an input read through a dynamic offset (`addr_i[i]` under a
//! loop, the fused load) was not in the block's arming set, so a block whose
//! only changing input is read that way skipped edges. CVA6's SRAM model
//! then never latched a new read address and the icache returned stale tags.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn flop_rearms_on_an_input_read_through_a_dynamic_offset() {
    let sim = simulate(
        r#"
module ram #(parameter int unsigned NumPorts = 1, parameter int unsigned AddrWidth = 7,
             parameter type addr_t = logic [AddrWidth-1:0])
  (input logic clk_i, input logic rst_ni, input logic [NumPorts-1:0] req_i, input logic [NumPorts-1:0] we_i,
   input addr_t [NumPorts-1:0] addr_i, output addr_t [NumPorts-1:0] r_addr_o);
  addr_t [NumPorts-1:0] r_addr_q;
  always_ff @(posedge clk_i or negedge rst_ni) begin
    if (!rst_ni) begin
      for (int i = 0; i < NumPorts; i++) r_addr_q[i] <= {AddrWidth{1'b0}};
    end else begin
      for (int unsigned i = 0; i < NumPorts; i++) begin
        if (req_i[i]) begin
          if (!we_i[i]) r_addr_q[i] <= addr_i[i];
        end
      end
    end
  end
  assign r_addr_o = r_addr_q;
endmodule
module top;
  logic clk = 0, rst_n = 0; logic [0:0] req = 0, we = 0;
  logic [6:0] addr = 0; logic [6:0] q0, q1; logic [3:0] seen;
  ram u0 (.clk_i(clk), .rst_ni(rst_n), .req_i(req), .we_i(we), .addr_i(addr), .r_addr_o(q0));
  ram u1 (.clk_i(clk), .rst_ni(rst_n), .req_i(req), .we_i(we), .addr_i(addr), .r_addr_o(q1));
  task tick; #1 clk = 1; #1 clk = 0; endtask
  initial begin
    tick; rst_n = 1; req = 1;                 // request held high from here on
    for (int k = 0; k < 400; k++) tick;       // long idle stretch: addr constant
    addr = 1; tick; #1 $display("A q0=%h q1=%h", q0, q1);
    addr = 2; tick; #1 $display("B q0=%h q1=%h", q0, q1);
    tick; #1 $display("C q0=%h q1=%h", q0, q1);
    addr = 3; tick; #1 $display("D q0=%h q1=%h", q0, q1);
  end
endmodule
"#,
        2000,
    )
    .expect("sim");
    let msgs = messages(&sim);
    for want in ["A q0=01 q1=01", "B q0=02 q1=02", "C q0=02 q1=02", "D q0=03 q1=03"] {
        assert!(msgs.iter().any(|m| m == want), "missing {want:?}: {msgs:?}");
    }
}
