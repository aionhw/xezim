//! `%p` assignment-pattern format (IEEE 1800-2017 §21.2.1.7).
//!
//! Aggregates must print with NAMED members in DECLARATION order:
//!   struct  -> '{data:10, addr:20, vld:1}
//!   class   -> '{id:1, a:7, s:"hi", r:2.5}   (base-class members first)
//!   null    -> null
//! Previously xezim printed unnamed values (packed: reverse order; unpacked:
//! alphabetical) and a bare handle for class objects.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct packed { bit [7:0] data; bit [7:0] addr; bit vld; } ppkt_t;
  typedef struct { bit [7:0] data; bit [7:0] addr; } upkt_t;
  class Base; int id = 1; endclass
  class Derived extends Base;
    int    a = 7;
    string s = "hi";
    real   r = 2.5;
  endclass
  ppkt_t  p;
  upkt_t  u;
  Derived d;
  Base    nul;
  initial begin
    p.data = 10; p.addr = 20; p.vld = 1;
    u.data = 30; u.addr = 40;
    d = new();
    $display("PACKED=%p",   p);
    $display("UNPACKED=%p", u);
    $display("CLASS=%p",    d);
    $display("NULL=%p",     nul);
  end
endmodule
"#;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

#[test]
fn p_format_prints_named_members_in_declaration_order() {
    let sim = simulate(SRC, 100).expect("simulate failed");

    // Packed struct: named, declaration order (not bit-offset order).
    assert_eq!(line(&sim, "PACKED="), "PACKED='{data:10, addr:20, vld:1}");
    // Unpacked struct: named, declaration order (not alphabetical).
    assert_eq!(line(&sim, "UNPACKED="), "UNPACKED='{data:30, addr:40}");
    // Class: base-class member first, string quoted, real as real.
    assert_eq!(line(&sim, "CLASS="), r#"CLASS='{id:1, a:7, s:"hi", r:2.5}"#);
    // Null handle.
    assert_eq!(line(&sim, "NULL="), "NULL=null");
}
