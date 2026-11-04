//! §8.25, §25.9: a module-scope variable of a class specialized by a
//! virtual interface type (`holder #(virtual bus_if) h;`) is built as that
//! specialization by `h = new(...)`, so its `T item;` is a virtual
//! interface. The parser dropped the `virtual` type argument of a
//! module-scope declaration, the object was the default specialization, and
//! `h.item.w` read x. Procedural declarations were unaffected. Expected
//! values are the reference simulator's.

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
fn module_var_specialized_by_virtual_interface() {
    const SRC: &str = r#"
interface bus_if #(parameter int W = 1);
  logic [W-1:0] w;
  modport mon(input w);
endinterface
class holder #(type T = int);
  T item;
  function new(T v = null); item = v; endfunction
  function int get(); return item.w; endfunction
endclass
class env;
  holder #(virtual bus_if) p;
  function new(virtual bus_if b); p = new(b); endfunction
endclass
module top;
  bus_if bus();
  bus_if #(4) wide();
  holder #(virtual bus_if) m1;
  holder #(virtual bus_if.mon) mp;
  holder #(virtual bus_if #(4)) mw;
  holder #(int) mi;
  env e;
  initial begin
    bus.w = 1; wide.w = 4'ha;
    m1 = new(bus); mp = new(bus); mw = new(wide); mi = new(7);
    e = new(bus);
    #1 $display("T|m1=%0d mp=%0d mw=%0h prop=%0d int=%0d", m1.get(), mp.item.w, mw.get(), e.p.get(), mi.item);
  end
endmodule
"#;
    assert_eq!(t_lines(SRC), ["T|m1=1 mp=1 mw=a prop=1 int=7"]);
}
