//! §25.9, §8.11: `this.cfg.vif == null` compares the vif BINDING of the
//! object `this.cfg` names. The `this`-rooted receiver chain resolved to no
//! handle, so the operand was unclassifiable and a bound vif compared equal
//! to null, while `cfg.vif` and a local alias of `this.cfg` were correct.
//! Expected values are the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn this_rooted_vif_null_comparison() {
    const SRC: &str = r#"interface bus_if; endinterface
class configuration;
  virtual bus_if vif;
endclass
class holder;
  configuration cfg;
endclass
class consumer;
  configuration bound, unbound, other;
  holder h;
  function new(configuration b, configuration u, configuration o);
    bound = b; unbound = u; other = o;
    h = new(); h.cfg = b;
    show();
  endfunction
  function void show();
    $display("T|bound==null %0d", this.bound.vif == null);
    $display("T|bound!=null %0d", this.bound.vif != null);
    $display("T|null==bound %0d", null == this.bound.vif);
    $display("T|nested==null %0d", this.h.cfg.vif == null);
    $display("T|unbound==null %0d", this.unbound.vif == null);
    $display("T|same-if %0d", this.bound.vif == this.h.cfg.vif);
    $display("T|other-if %0d", this.bound.vif == this.other.vif);
    if (this.bound.vif == null) $display("T|if null"); else $display("T|if bound");
  endfunction
endclass
module top;
  bus_if bus();
  bus_if bus2();
  initial begin
    configuration b = new(), u = new(), o = new();
    consumer c;
    b.vif = bus;
    o.vif = bus2;
    c = new(b, u, o);
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|bound==null 0",
            "T|bound!=null 1",
            "T|null==bound 0",
            "T|nested==null 0",
            "T|unbound==null 1",
            "T|same-if 1",
            "T|other-if 0",
            "T|if bound",
        ]
    );
}
