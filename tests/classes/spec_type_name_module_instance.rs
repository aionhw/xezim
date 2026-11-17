//! §8.25 / §25.9 / §3.13: `virtual clk_rst_if` in a class specialization names
//! the interface type. An interface instance with the same spelling must not
//! change the specialization key when a package consumer reads the static store.
use xezim::simulate;

fn output_of(sim: &xezim::compiler::Simulator) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn module_set_visible_to_package_get_with_same_type_name() {
    const SRC: &str = r#"
interface clk_rst_if;
  logic clk;
endinterface
package p;
  class db #(type T = int);
    static T store[string];
    static function void set(string k, T v);
      store[k] = v;
    endfunction
    static function bit get(string k, output T v);
      if (!store.exists(k)) return 0;
      v = store[k];
      return 1;
    endfunction
  endclass
  class reader;
    static function bit probe();
      virtual clk_rst_if vif;
      return db#(virtual clk_rst_if)::get("vif", vif);
    endfunction
  endclass
endpackage
module u_tb;
  import p::*;
  clk_rst_if clk_rst_if();
  initial begin
    db#(virtual clk_rst_if)::set("vif", clk_rst_if);
  end
endmodule
module top;
  import p::*;
  u_tb u();
  initial begin
    bit found;
    #0;
    found = reader::probe();
    $display("found=%0d", found);
    $finish;
  end
endmodule
"#;
    let out = output_of(&simulate(SRC, 500).expect("sim"));
    assert!(
        out.contains("found=1"),
        "package get must see the module set through the same virtual-interface specialization:\n{out}"
    );
}