//! §8.25, §25.9: a property declared by a base class as a bare type
//! parameter (`V vif;` in `class base #(type V = virtual bus_if)`) is a
//! virtual interface in a derived class whose `extends` binds `V` to one,
//! through the parameter's default or an explicit `#(virtual bus_if)`. Only
//! an object of the parameterized class itself was given the binding, so in
//! the derived class `vif.clk` read 0 and `@vif.cb` returned at once.
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
fn inherited_type_param_vif_binds_in_derived_class() {
    const SRC: &str = r#"
interface bus_if(input bit clk);
  clocking cb @(posedge clk);
  endclocking
  modport mon(clocking cb, input clk);
endinterface
class base_c #(type V = virtual bus_if);
  V vif;
  function new(V v);
    vif = v;
  endfunction
  virtual task sample(string tag);
    @vif.cb;
    $display("T|%s t=%0t clk=%0d", tag, $time, vif.clk);
  endtask
endclass
class dflt_c extends base_c;
  function new(virtual bus_if v);
    super.new(v);
  endfunction
endclass
class mid_c #(type T = virtual bus_if) extends base_c #(T);
  function new(T v);
    super.new(v);
  endfunction
endclass
class leaf_c extends mid_c;
  function new(virtual bus_if v);
    super.new(v);
  endfunction
endclass
class mp_c extends base_c #(virtual bus_if.mon);
  function new(virtual bus_if.mon v);
    super.new(v);
  endfunction
endclass
class int_c extends base_c #(int);
  function new();
    super.new(7);
  endfunction
endclass
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  bus_if bus(clk);
  initial begin
    dflt_c d = new(bus);
    leaf_c l = new(bus);
    mp_c m = new(bus);
    int_c i = new();
    $display("T|int vif=%0d", i.vif);
    d.sample("default");
    #1 l.sample("chain");
    #1 m.sample("modport");
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|int vif=7",
            "T|default t=5 clk=1",
            "T|chain t=15 clk=1",
            "T|modport t=25 clk=1",
        ]
    );
}

/// The derived class FORWARDS its own type parameter to the base (`class
/// fwd_c #(type W = virtual bus_if) extends base_c #(W)`): whether the
/// inherited `vif` is a virtual interface depends on the specialization.
#[test]
fn forwarded_type_param_vif_binds_per_specialization() {
    const SRC: &str = r#"
interface bus_if(input bit clk);
  clocking cb @(posedge clk);
  endclocking
endinterface
class base_c #(type V = virtual bus_if);
  V vif;
  function new(V v);
    vif = v;
  endfunction
  virtual task sample(string tag);
    @vif.cb;
    $display("T|%s t=%0t clk=%0d", tag, $time, vif.clk);
  endtask
endclass
class fwd_c #(type W = virtual bus_if) extends base_c #(W);
  function new(W v);
    super.new(v);
  endfunction
endclass
module top;
  bit clk = 0;
  always #5 clk = ~clk;
  bus_if bus(clk);
  initial begin
    fwd_c #() d = new(bus);
    fwd_c #(virtual bus_if) e = new(bus);
    fwd_c #(int) i = new(9);
    $display("T|int vif=%0d", i.vif);
    d.sample("default");
    #1 e.sample("explicit");
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|int vif=9",
            "T|default t=5 clk=1",
            "T|explicit t=15 clk=1",
        ]
    );
}
