//! §14.3, §14.16: clockvars reached through a select.
//!
//! An unpacked-array clocking input (`input ready;` for `logic ready[2]`)
//! has no signal under its own name, so it was never sampled and
//! `cb.ready[i]` read x. A drive of a select of an output clockvar
//! (`cb.v[2] <= 1`, `cb.x[3:2] <= 2'b11`, `cb.u[1] <= 1`) was not
//! recognized as a clocking drive and wrote nothing. Selected drives mature exactly
//! when whole drives do. Expected values are the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 1_000)
        .expect("simulate clocking selects")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn unpacked_array_clocking_input_samples_each_element() {
    const SRC: &str = r#"
interface bus_if(input bit clk);
  logic ready[2];
  clocking cb @(posedge clk);
    input ready;
  endclocking
  task automatic own_read();
    @cb;
    $display("T|iface t=%0t r0=%b r1=%b", $time, cb.ready[0], cb.ready[1]);
  endtask
endinterface
class reader;
  virtual bus_if vif;
  function new(virtual bus_if v);
    vif = v;
  endfunction
  task sample(int i);
    @vif.cb;
    $display("T|vif t=%0t r[%0d]=%b", $time, i, vif.cb.ready[i]);
  endtask
endclass
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  bus_if bus(clk);
  initial begin
    reader r = new(bus);
    bus.ready[0] = 1;
    bus.ready[1] = 0;
    fork
      r.sample(0);
      bus.own_read();
    join
    bus.ready[1] <= 1;
    @(bus.cb);
    $display("T|top t=%0t r0=%b r1=%b", $time, bus.cb.ready[0], bus.cb.ready[1]);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|vif t=5 r[0]=1",
            "T|iface t=5 r0=1 r1=0",
            "T|top t=15 r0=1 r1=1",
        ]
    );
}

#[test]
fn selected_clocking_output_drives_only_the_selection() {
    const SRC: &str = r#"
interface bus_if(input bit clk);
  logic [3:0] a = 0, b = 0, s = 0, s2 = 0, c = 0, c2 = 0, v = 0;
  logic u[2] = '{0, 0};
  clocking cb @(posedge clk);
    output a, b, c, c2, v, u;
    output #2 s, s2;
  endclocking
endinterface
class driver;
  virtual bus_if vif;
  function new(virtual bus_if v);
    vif = v;
  endfunction
  task drive();
    @vif.cb;
    vif.cb.v[2] <= 1;
    vif.cb.u[1] <= 1;
  endtask
endclass
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  bus_if bus(clk);
  task show();
    $display("T|t=%0t a=%b b=%b s=%b s2=%b c=%b c2=%b v=%b u1=%b",
             $time, bus.a, bus.b, bus.s, bus.s2, bus.c, bus.c2, bus.v, bus.u[1]);
  endtask
  initial begin
    driver d = new(bus);
    fork d.drive(); join_none
    @(bus.cb);
    bus.cb.a <= 4'b0001;
    bus.cb.b[0] <= 1;
    bus.cb.b[2] <= 1;
    bus.cb.s <= 4'b1000;
    bus.cb.s2[3] <= 1;
    #1 show();
    bus.cb.a <= 4'b0011;
    bus.cb.b[1] <= 1;
    bus.cb.c <= ##2 4'b1111;
    bus.cb.c2[1:0] <= ##2 2'b11;
    #2 show();
    #8 show();
    #10 show();
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|t=6 a=0001 b=0101 s=0000 s2=0000 c=0000 c2=0000 v=0100 u1=1",
            "T|t=8 a=0001 b=0101 s=1000 s2=1000 c=0000 c2=0000 v=0100 u1=1",
            "T|t=16 a=0011 b=0111 s=1000 s2=1000 c=0000 c2=0000 v=0100 u1=1",
            "T|t=26 a=0011 b=0111 s=1000 s2=1000 c=1111 c2=0011 v=0100 u1=1",
        ]
    );
}
