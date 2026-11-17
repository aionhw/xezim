//! IEEE 1800 §8.25, §25.9: a homonymous interface instance does not
//! change a virtual-interface type argument or its specialization storage.
const SOURCE: &str = r#"interface clk_rst_if;
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

#[test]
fn virtual_interface_type_keeps_shared_static_storage() {
    let out = xezim::simulate(SOURCE, 10).expect("simulate");
    assert!(
        out.output
            .iter()
            .any(|line| line.message.contains("found=1")),
        "{:?}",
        out.output
    );
}

#[test]
fn explicit_interface_keyword_keeps_shared_static_storage() {
    let source = SOURCE.replace("virtual clk_rst_if", "virtual interface clk_rst_if");
    let out = xezim::simulate(&source, 10).expect("simulate");
    assert!(
        out.output
            .iter()
            .any(|line| line.message.contains("found=1")),
        "{:?}",
        out.output
    );
}
