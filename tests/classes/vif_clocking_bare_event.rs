//! §9.4.2, §14.13, §25.9: `@vif.cb` is `@(vif.cb)`. The bare form of an
//! event control naming a member (a clocking block through a virtual
//! interface held by an object) got no sensitivity and returned at once, so
//! a `while (vif.reset_n === 0) @vif.cb;` loop spun forever at time 0.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn bare_vif_clocking_event_waits_for_clock() {
    const SRC: &str = r#"`timescale 1ns/1ps
interface bus_if(input bit clk);
  bit reset_n = 0;
  clocking monitor_cb @(posedge clk);
    input reset_n;
  endclocking
endinterface
class monitor;
  virtual bus_if vif;
  function new(virtual bus_if vif);
    this.vif = vif;
  endfunction
  task run();
    while (vif.reset_n === 1'b0)
      @vif.monitor_cb;
    $display("T|bare t=%0t", $time);
  endtask
  task run_this();
    @(this.vif.monitor_cb);
    $display("T|this t=%0t", $time);
  endtask
endclass
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  bus_if bus(clk);
  initial begin
    monitor m = new(bus);
    fork
      begin #12 bus.reset_n = 1; end
      m.run();
      m.run_this();
    join
    $finish;
  end
endmodule
"#;
    assert_eq!(t_lines(SRC), ["T|this t=5000", "T|bare t=15000"]);
}
