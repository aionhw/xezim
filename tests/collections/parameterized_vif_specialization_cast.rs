//! §8.25, §25.9: a class specialized by a virtual interface type keeps its
//! class identity when the interface has a parameter. `$cast` back to
//! `res#(virtual mon_if)` must accept the object `res#(virtual mon_if)::make`
//! built. With an integer interface parameter the specialization is entered
//! as `res<virtual mon_if#(32)>`, and a split on `#` stopped inside that
//! signature, so the object was no longer a `res`. A class named like the
//! interface without the space (`virtualmon_if`) is a different type.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{n}")))
        .unwrap_or_else(|| panic!("signal not found: {n}"))
        .to_u64()
        .unwrap_or_else(|| panic!("{n} not u64-able"))
        & 0xFFFF_FFFF
}

#[test]
fn cast_of_integer_parameter_virtual_interface() {
    let src = r#"
class res_base;
endclass

class res #(type T = int) extends res_base;
  typedef res#(T) this_type;
  T val;
  static function res_base make();
    res#(T) r = new;
    return r;
  endfunction
  static function int from_q(res_base q[$]);
    this_type rsrc;
    int first;
    for (first = 0; first < q.size() && !$cast(rsrc, q[first]); first++) ;
    return rsrc != null;
  endfunction
endclass

interface str_if #(parameter string IfName = "main");
endinterface

interface plain_if;
  int x;
endinterface

interface mon_if #(parameter DATA_WIDTH = 32);
endinterface

class virtualmon_if;
endclass

class virtualplain_if;
endclass

class box #(type T = int);
  T vif;
endclass

module top;
  int str_ok, plain_ok, mon_ok, wide_ok, class_ok, plain_cls, prop_ok;
  res_base cq[$], pq[$], mq[$], wq[$], vq[$], pq2[$];
  plain_if bus();
  box#(virtual plain_if) b;
  initial begin
    b = new;
    b.vif = bus;
    bus.x = 4;
    prop_ok = b.vif.x;
    cq.push_back(res#(virtual str_if)::make());
    pq.push_back(res#(virtual plain_if)::make());
    mq.push_back(res#(virtual mon_if)::make());
    wq.push_back(res#(virtual mon_if#(64))::make());
    vq.push_back(res#(virtualmon_if)::make());
    pq2.push_back(res#(virtualplain_if)::make());
    str_ok = res#(virtual str_if)::from_q(cq);
    plain_ok = res#(virtual plain_if)::from_q(pq);
    mon_ok = res#(virtual mon_if)::from_q(mq);
    wide_ok = res#(virtual mon_if)::from_q(wq);
    class_ok = res#(virtual mon_if)::from_q(vq);
    plain_cls = res#(virtual plain_if)::from_q(pq2);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(
        (
            u(&sim, "str_ok"),
            u(&sim, "plain_ok"),
            u(&sim, "mon_ok"),
            u(&sim, "wide_ok"),
            u(&sim, "class_ok"),
            u(&sim, "plain_cls"),
            u(&sim, "prop_ok"),
        ),
        (1, 1, 1, 0, 0, 0, 4),
        "string-param, plain, integer-param, a different integer parameter, \
         classes named like the interfaces, and a property typed by the interface"
    );
}

/// A parameterized class named like a parameterized interface keeps its own
/// defaults: `virtualmon_if#()` is `virtualmon_if#(8)`, not the interface's
/// `#(32)`, so both spellings share one specialization's statics.
#[test]
fn class_named_like_interface_keeps_its_defaults() {
    let src = r#"
interface mon_if #(parameter N = 32);
endinterface

class virtualmon_if #(int N = 8);
endclass

class holder #(type T = virtualmon_if#());
  static int n;
endclass

module top;
  int dflt, empty, vif;
  initial begin
    holder#(virtualmon_if#(8))::n = 5;
    dflt = holder#()::n;
    empty = holder#(virtualmon_if#())::n;
    vif = holder#(virtual mon_if)::n;
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(
        (u(&sim, "dflt"), u(&sim, "empty"), u(&sim, "vif")),
        (5, 5, 0),
        "default, empty #(), and the virtual interface"
    );
}
