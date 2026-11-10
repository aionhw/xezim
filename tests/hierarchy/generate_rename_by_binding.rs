//! §23.9, §27.6: a name in a generate block binds from its block outward, and
//! the rename gives it the key of the declaration it binds to, through every
//! enclosing block. Outputs as Verilator 5.055 prints them.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("{e}\n{src}"));
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|m| !m.is_empty() && !m.contains("$finish") && !m.contains("[xezim]"))
        .collect()
}

#[test]
fn a_typedef_of_a_nested_block_keeps_its_own_key_through_both_renames() {
    // §6.18, §23.9, §27.6: `t` in `g.h` is h's 8-bit type, in `g` g's 4-bit type.
    let src = r#"module top;
  if (1) begin : g
    typedef logic [3:0] t;
    t w = '1;
    if (1) begin : h
      typedef logic [7:0] t;
      t v = '1;
      initial #2 $display("h %0d %0d", $bits(v), v);
    end
    initial #1 $display("g %0d %0d", $bits(w), w);
  end
endmodule
"#;
    assert_eq!(lines(src), ["g 4 15", "h 8 255"]);
}

#[test]
fn a_property_and_a_sequence_of_a_nested_block_are_the_blocks_own() {
    // §16.8, §16.12, §23.9: `p` and `s` in `g.h` read h's `x`, which is 1.
    let src = r#"module top;
  logic clk = 0;
  always #5 clk = ~clk;
  if (1) begin : g
    logic x = 0;
    sequence s; x; endsequence
    property p; @(posedge clk) x; endproperty
    if (1) begin : h
      logic x = 1;
      sequence s; x; endsequence
      property p; @(posedge clk) x; endproperty
      ap: assert property (p) $display("p pass %0t", $time); else $display("p fail %0t", $time);
      as: assert property (@(posedge clk) s) $display("s pass %0t", $time); else $display("s fail %0t", $time);
    end
  end
  initial #12 $finish;
endmodule
"#;
    assert_eq!(lines(src), ["p pass 5", "s pass 5"]);
}

#[test]
fn a_class_extends_the_typedef_of_its_own_nested_block() {
    // §6.18, §8.13, §23.9: `base_t` in `g.h` names h's `B`.
    let src = r#"module top;
  if (1) begin : g
    class A; function int v(); return 1; endfunction endclass
    typedef A base_t;
    if (1) begin : h
      class B; function int v(); return 2; endfunction endclass
      typedef B base_t;
      class D extends base_t; endclass
      initial begin D d = new; $display("h %0d", d.v()); end
    end
  end
endmodule
"#;
    assert_eq!(lines(src), ["h 2"]);
}

#[test]
fn a_clocking_block_of_a_nested_block_is_clocked_by_the_blocks_signal() {
    // §14.3, §23.9: the clocking event names h's `c`, which rises at 1 and 3.
    let src = r#"module top;
  if (1) begin : g
    logic c = 0;
    if (1) begin : h
      logic c = 0;
      clocking cb @(posedge c); endclocking
      initial begin #1 c = 1; #1 c = 0; #1 c = 1; end
      initial begin @(cb); @(cb); $display("h cb %0t", $time); end
    end
  end
  initial #10 $finish;
endmodule
"#;
    assert_eq!(lines(src), ["h cb 3"]);
}

#[test]
fn a_path_through_the_outer_block_calls_the_outer_blocks_function() {
    // §23.6, §23.9: `g.f` in `g.h` is g's `f`, not h's: 10 + (1 + 0).
    let src = r#"module top;
  if (1) begin : g
    function automatic int f(int x); f = 1; f += x; endfunction
    if (1) begin : h
      function automatic int f(int x); f = 10; f += g.f(x - 1); endfunction
      initial $display("%0d", f(1));
    end
  end
endmodule
"#;
    assert_eq!(lines(src), ["11"]);
}

#[test]
fn an_interface_instance_of_a_block_passed_as_a_value_is_declared() {
    // §25.9, §23.9: `b` is the block's interface instance; a class keeps it as a virtual interface.
    let src = r#"interface bus_if; int value; endinterface
class cfg;
  virtual bus_if vif;
  function void set(virtual bus_if v); vif = v; endfunction
endclass
module top;
  if (1) begin : g
    bus_if b();
    initial begin
      cfg c = new;
      c.set(b);
      c.vif.value = 17;
      #1 $display("value=%0d", b.value);
    end
  end
endmodule
"#;
    assert_eq!(lines(src), ["value=17"]);
}

#[test]
fn a_trigger_of_a_nested_blocks_event_wakes_that_blocks_waiter() {
    // §15.5.1, §23.9: `-> ev` in each block fires that block's event.
    let src = r#"module top;
  if (1) begin : g
    event ev;
    initial #1 -> ev;
    initial begin @(ev); $display("g ev %0t", $time); end
    if (1) begin : h
      event ev;
      initial #2 -> ev;
      initial begin @(ev); $display("h ev %0t", $time); end
    end
  end
endmodule
"#;
    assert_eq!(lines(src), ["g ev 1", "h ev 2"]);
}
