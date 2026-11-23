//! IEEE 1800-2023 §8.25. A class constructed through a type parameter
//! retains its specialization when the declaring module is instantiated.
use xezim::simulate;

#[test]
fn factory_preserves_method_local_type_in_child_module() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let sim = simulate(
        r#"
module child;
  class box_t #(type E = byte);
    function int keep(int v);
      E saved = v;
      return saved;
    endfunction
  endclass
  class factory #(type T = box_t);
    static function T make();
      T obj = new;
      return obj;
    endfunction
  endclass
  initial begin
    box_t #(byte) b8 = factory #(box_t #(byte))::make();
    box_t #(shortint) b16 = factory #(box_t #(shortint))::make();
    box_t #(int) b32 = factory #(box_t #(int))::make();
    $display("values=%h,%h,%h", b8.keep(32'h12345678),
             b16.keep(32'h12345678), b32.keep(32'h12345678));
  end
endmodule
module top; child u(); endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(
        lines.contains(&"values=00000078,00005678,12345678"),
        "{lines:?}"
    );
}
