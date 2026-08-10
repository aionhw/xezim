//! Pure-SystemVerilog regression for associative arrays with DOTTED string
//! keys (`assoc["a.b"]`), the exact pattern UVM's `uvm_port_base` uses when
//! its `resolve_bindings` stores implementation ports under
//! `m_imp_list[get_full_name()]` (hierarchical names like
//! `uvm_test_top.env.agent.sqr.seq_item_export`).
//!
//! `assoc_top_level_keys` (which powers `num()`, `size()`, and `foreach`)
//! carried a `.filter(|key| !key.contains('.'))` added to ignore struct
//! member-wise leaves. It over-aggressively dropped REAL dotted string keys
//! (e.g. `"a.b"`), so `m_imp_list[dotted]=this; num()` returned 0 and every
//! UVM sequencer connection check failed with "connection count of 0 does
//! not meet required minimum of 1". Struct leaves collapse onto one key via
//! dedup, so the dot filter is unnecessary.
use xezim::simulate;

fn out_line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no {} line", tag))
}

/// A string-keyed associative array class member with a dotted key.
#[test]
fn dotted_string_key_num_counts() {
    const SRC: &str = r#"
package pk;
  class if_base;
    function new; endfunction
  endclass
  virtual class C #(type IF = if_base) extends IF;
    typedef C #(IF) this_type;
    local this_type m_list [string];
    function int cnt(); return m_list.num(); endfunction
    function void add(string k);
      m_list[k] = this;
    endfunction
  endclass
  class D extends C #(if_base);
    function new; endfunction
  endclass
endpackage
module top;
  import pk::*;
  initial begin
    D d;
    d = new;
    d.add("uvm_test_top.env.sqr.seq_item_export");
    d.add("plain");
    $display("DRES=%0d", d.cnt());
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(out_line(&sim, "DRES="), "DRES=2");
}

/// A module-scope assoc of class keyed by dotted strings (int/str mix).
#[test]
fn module_assoc_dotted_key_num() {
    const SRC: &str = r#"
module top;
  class K; int v; function new(int x); v=x; endfunction endclass
  K m [string];
  K a, b;
  initial begin
    a = new(1); b = new(2);
    m["env.agent.a"] = a;
    m["env.agent.b"] = b;
    $display("MDOT=%0d", m.num());
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(out_line(&sim, "MDOT="), "MDOT=2");
}