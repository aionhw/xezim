//! §8.25, §25.9: a class specialized by a virtual interface type keeps its
//! class identity when the interface has a parameter. `$cast` back to
//! `res#(virtual mon_if)` must accept the object `res#(virtual mon_if)::make`
//! built. With an integer interface parameter the specialization is entered
//! as `res<virtualmon_if#(32)>`, and a split on `#` stopped inside that
//! signature, so the object was no longer a `res`.

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
endinterface

interface mon_if #(parameter DATA_WIDTH = 32);
endinterface

module top;
  int str_ok, plain_ok, mon_ok, wide_ok;
  res_base cq[$], pq[$], mq[$], wq[$];
  initial begin
    cq.push_back(res#(virtual str_if)::make());
    pq.push_back(res#(virtual plain_if)::make());
    mq.push_back(res#(virtual mon_if)::make());
    wq.push_back(res#(virtual mon_if#(64))::make());
    str_ok = res#(virtual str_if)::from_q(cq);
    plain_ok = res#(virtual plain_if)::from_q(pq);
    mon_ok = res#(virtual mon_if)::from_q(mq);
    wide_ok = res#(virtual mon_if)::from_q(wq);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert_eq!(
        (
            u(&sim, "str_ok"),
            u(&sim, "plain_ok"),
            u(&sim, "mon_ok"),
            u(&sim, "wide_ok")
        ),
        (1, 1, 1, 0),
        "string-param, plain, integer-param, and a different integer parameter"
    );
}
