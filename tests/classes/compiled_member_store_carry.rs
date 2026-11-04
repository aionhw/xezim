//! IEEE 1800-2017 §11.6.1, §10.4: the left-hand side of `recv.field = a + b`
//! takes part in sizing the right-hand side, so a 9-bit field keeps the carry
//! of two 8-bit operands. Each method runs past the compile threshold, so the
//! compiled store is exercised as well as the interpreted one.

use xezim::simulate;

fn check(source: &str) {
    let sim = simulate(source, 100).expect("simulation");
    let out = sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n");
    assert!(out.contains("PASS") && !out.contains("Fatal"), "{out}");
}

#[test]
fn compiled_store_to_another_objects_field_keeps_the_carry() {
    check(
        r#"
class packet; bit [8:0] f; endclass
class holder;
  packet d = new();
  function void add(bit [7:0] a, bit [7:0] b); d.f = a + b; endfunction
endclass
module top;
  initial begin
    holder h = new();
    for (int i = 0; i < 2005; i++) begin
      h.add(8'd200, 8'd100);
      if (h.d.f != 9'd300) $fatal(1, "iteration=%0d d.f=%0d", i, h.d.f);
    end
    $display("PASS");
  end
endmodule
"#,
    );
}

#[test]
fn compiled_store_through_this_keeps_the_carry() {
    check(
        r#"
class holder;
  bit [8:0] g;
  function void add(bit [7:0] a, bit [7:0] b); this.g = a + b; endfunction
endclass
module top;
  initial begin
    holder h = new();
    for (int i = 0; i < 2005; i++) begin
      h.add(8'd200, 8'd100);
      if (h.g != 9'd300) $fatal(1, "iteration=%0d this.g=%0d", i, h.g);
    end
    $display("PASS");
  end
endmodule
"#,
    );
}
