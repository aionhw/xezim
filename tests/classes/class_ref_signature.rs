//! IEEE 1800-2017 §8.24 / §13.5.2: method signatures belong to their
//! classes, not a package's bare-name function table.
use xezim::simulate;

#[test]
fn out_of_block_route_methods_keep_their_formal_directions() {
    let sim = simulate(
        r#"package mon_pkg;
  class ic_model;
    extern function void route(input int txn, ref int ports[$], ref int sync);
  endclass
  function void ic_model::route(input int txn, ref int ports[$], ref int sync);
    sync = txn;
  endfunction
  class monitor_model;
    extern function void route(input string mode, input int txn, input int addr, ref int ports[$]);
    extern function void run();
  endclass
  function void monitor_model::route(input string mode, input int txn, input int addr, ref int ports[$]);
    ports.push_back(addr);
  endfunction
  function void monitor_model::run();
    int ports[$];
    route("dvm", 1, 0, ports);
    route(.mode("read"), .txn(2), .addr(7), .ports(ports));
    $display("monitor ports %0d %0d size %0d", ports[0], ports[1], ports.size());
  endfunction
endpackage
module top;
  import mon_pkg::*;
  initial begin
    monitor_model m = new();
    ic_model ic = new();
    int ports[$];
    int sync = 0;
    m.run();
    ic.route(23, ports, sync);
    $display("interconnect sync %0d", sync);
    $finish;
  end
endmodule"#,
        10,
    )
    .expect("each method uses its class signature");
    let out: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.as_str())
        .collect();
    assert_eq!(out, ["monitor ports 0 7 size 2", "interconnect sync 23"]);
}
