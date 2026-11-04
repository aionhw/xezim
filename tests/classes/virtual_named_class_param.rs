//! §8.25, §25.9: a class whose name merely starts with "virtual"
//! (`virtual_cfg`) passed as a type parameter is an ordinary class type.
//! The specialization's unspaced argument (`virtual_cfg`) was read as
//! `virtual` + interface `_cfg`, so `T item;` became a virtual-interface
//! property and a handle stored in it read back null. Expected values are
//! the reference simulator's.

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
fn class_named_virtual_is_not_a_virtual_interface() {
    const SRC: &str = r#"
class virtual_cfg;
  int x = 42;
endclass
class holder #(type T = int);
  T item;
endclass
module top;
  holder #(virtual_cfg) h;
  initial begin
    h = new();
    h.item = new();
    $display("T|null=%0d x=%0d", h.item == null, h.item.x);
  end
endmodule
"#;
    assert_eq!(t_lines(SRC), ["T|null=0 x=42"]);
}
