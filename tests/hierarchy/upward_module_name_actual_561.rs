//! IEEE 1800-2023 §23.8 / §23.10.1: a port actual whose first segment is the
//! definition name of an enclosing instance (`dut.gclk_a`, written in a
//! module bound into `dut`) resolves upward to that instance. A statement
//! already resolved such a name at run time; an actual is bound at
//! elaboration and read X (#561, reported by Ganesh-TNL).

use xezim::simulate;

const SRC: &str = r#"
interface bus_if (input logic send_clk);
endinterface

module dut (input logic gclk_a, input logic gclk_b);
endmodule

module harness;
  bus_if IA (.send_clk(dut.gclk_a));     // upward reference by module name
  bus_if IB (.send_clk(t.u_dut.gclk_b)); // absolute path — control
  int a_fired = 0;
  int b_fired = 0;
  initial begin @(posedge IA.send_clk); a_fired = 1; end
  initial begin @(posedge IB.send_clk); b_fired = 1; end
endmodule

bind dut harness v_harness (.*);

module t;
  logic gclk_a_net, gclk_b_net;
  dut u_dut (.gclk_a(gclk_a_net), .gclk_b(gclk_b_net));
  initial begin
    gclk_a_net = 1'b0; gclk_b_net = 1'b0;
    #10 gclk_a_net = 1'b1;
    #10 gclk_b_net = 1'b1;
    #10 $display("A|%0d B|%0d", u_dut.v_harness.a_fired, u_dut.v_harness.b_fired);
  end
endmodule
"#;

#[test]
fn bound_module_resolves_an_enclosing_module_name_in_a_port_actual() {
    let sim = simulate(SRC, 100).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"A|1 B|1"), "{msgs:?}");
}

/// A child instance of the current scope named like an enclosing module
/// wins over the definition name (§23.8 searches instance names first).
#[test]
fn a_child_instance_shadows_the_enclosing_module_name() {
    let src = r#"
module sink (input logic clk);
  int fired = 0;
  initial begin @(posedge clk); fired = 1; end
endmodule
module leaf (output logic gclk_a);
  initial begin gclk_a = 0; #5 gclk_a = 1; end
endmodule
module harness;
  leaf dut ();
  sink s (.clk(dut.gclk_a));
endmodule
module dut (input logic gclk_a);
  harness v_harness ();
endmodule
module t;
  logic gclk_a_net = 1'b0;
  dut u_dut (.gclk_a(gclk_a_net));
  initial #20 $display("S|%0d", u_dut.v_harness.s.fired);
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate");
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(msgs.contains(&"S|1"), "{msgs:?}");
}
