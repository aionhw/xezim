//! LRM §25.5.4 / §25.9 interface gaps:
//!
//! 1. §25.5.4 — an interface FUNCTION called through the instance
//!    (`bus.snoop()`). Elaboration inlines the interface's subroutines under
//!    the instance-prefixed key ("bus.snoop") but the call dispatched on the
//!    LAST path segment only ("snoop" — not found), returning 0/X. The body's
//!    bare member names (`data`) must also resolve under the INSTANCE scope
//!    (`bus.data`), via the name-resolve-hint mechanism.
//!
//! 2. §25.9 — a plain block-local `virtual bus_if vif; vif = bus;` variable
//!    must alias the instance: `vif.member` reads and writes dispatch to
//!    `bus.member`. The task-formal (`task t(virtual bus_if v)`) and
//!    config_db paths already aliased; plain variable assignment did not.
//!
//! 3. §25.9 re-binding — a vif variable switched between two instances
//!    dispatches member access AND interface-function calls (`vif.snoop()`)
//!    to the CURRENTLY bound instance.

use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// §25.5.4: interface function called through the interface instance runs
/// with the instance's scope, so `return data;` reads `bus.data`.
#[test]
fn interface_function_called_via_instance() {
    const SRC: &str = r#"
interface bus_if (input logic clk);
  logic [7:0] data;
  logic valid;
  modport drv (output data, valid, input clk);
  function automatic logic [7:0] snoop(); return data; endfunction
endinterface
module tb;
  logic clk = 0;
  bus_if bus (.clk(clk));
  initial begin
    #1;
    bus.data = 8'h3C;
    $display("A=%h", bus.snoop());
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out = output_of(&sim);
    assert!(
        out.contains("A=3c"),
        "bus.snoop() must read bus.data (want A=3c):\n{}",
        out
    );
}

/// §25.9: a block-local `virtual bus_if vif; vif = bus;` aliases the
/// instance — `vif.data` reads bus.data and `vif.data = v` writes it.
#[test]
fn local_vif_variable_aliases_instance() {
    const SRC: &str = r#"
interface bus_if (input logic clk);
  logic [7:0] data;
  logic valid;
  modport drv (output data, valid, input clk);
  function automatic logic [7:0] snoop(); return data; endfunction
endinterface
module tb;
  logic clk = 0;
  bus_if bus (.clk(clk));
  initial begin
    #1;
    bus.data = 8'h3C;
    begin
      virtual bus_if vif;
      vif = bus;
      $display("B=%h", vif.data);
      vif.data = 8'h55;
      #1;
      $display("C=%h", bus.data);
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out = output_of(&sim);
    assert!(
        out.contains("B=3c"),
        "local vif read must alias bus.data (want B=3c):\n{}",
        out
    );
    assert!(
        out.contains("C=55"),
        "local vif write must reach bus.data (want C=55):\n{}",
        out
    );
    // The write went to the instance's signal, not a phantom `vif.data`.
    assert_eq!(
        sim.get_signal("bus.data").and_then(|v| v.to_u64()),
        Some(0x55),
        "bus.data signal must hold the vif-written value"
    );
}

/// §25.9 + §25.5.4: a vif variable re-bound between TWO instances dispatches
/// both member access and interface-function calls per the CURRENT binding.
#[test]
fn vif_switches_between_two_instances_with_snoop() {
    const SRC: &str = r#"
interface bus_if (input logic clk);
  logic [7:0] data;
  function automatic logic [7:0] snoop(); return data; endfunction
endinterface
module tb;
  logic clk = 0;
  bus_if bus1 (.clk(clk));
  bus_if bus2 (.clk(clk));
  initial begin
    #1;
    bus1.data = 8'h11;
    bus2.data = 8'h22;
    #1;
    $display("D=%h", bus1.snoop());
    $display("E=%h", bus2.snoop());
    begin
      virtual bus_if vif;
      vif = bus1;
      $display("F=%h", vif.snoop());
      $display("G=%h", vif.data);
      vif = bus2;
      $display("H=%h", vif.snoop());
      vif.data = 8'h33;
      #1;
      $display("I=%h", bus2.data);
      $display("J=%h", bus1.data);
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    let out = output_of(&sim);
    for (tag, want) in [
        ("D=11", "bus1.snoop() reads bus1.data"),
        ("E=22", "bus2.snoop() reads bus2.data"),
        ("F=11", "vif.snoop() dispatches to bus1 while bound to bus1"),
        ("G=11", "vif.data reads bus1.data while bound to bus1"),
        ("H=22", "vif.snoop() re-dispatches to bus2 after re-bind"),
        ("I=33", "vif.data write reaches bus2.data after re-bind"),
        ("J=11", "bus1.data untouched by the re-bound vif write"),
    ] {
        assert!(out.contains(tag), "{} (want {}):\n{}", want, tag, out);
    }
}
