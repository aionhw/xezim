//! IEEE 1800-2023 §6.18, §8.13, §13.5.1. An inherited typedef retains
//! the packed range evaluated in its declaring class.
use xezim::simulate;

#[test]
fn folded_class_bound_sizes_formal_local_and_return() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let sim = simulate(
        r#"
module top;
  class base;
    parameter WIDTH = 128;
    typedef logic [WIDTH-1:0] word_t;
    virtual function word_t run(word_t a); return a; endfunction
    virtual function word_t pass(word_t a); return a; endfunction
  endclass
  class derived #(type T = byte) extends base;
    virtual function word_t run(word_t a);
      word_t saved = a;
      $display("widths=%0d,%0d", $bits(a), $bits(saved));
      run = saved;
    endfunction
    virtual function word_t pass(word_t a);
      $display("direct=%0d,%h", $bits(a), a);
      return a;
    endfunction
  endclass
  initial begin
    derived #() d = new;
    base b = d;
    $display("value=%h", b.run(128'h8000000000000000_0000000000000061));
    $display("returned=%h", b.pass(128'h8000000000000000_0000000000000061));
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(lines.contains(&"widths=128,128"), "{lines:?}");
    assert!(
        lines.contains(&"direct=128,80000000000000000000000000000061"),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"returned=80000000000000000000000000000061"),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"value=80000000000000000000000000000061"),
        "{lines:?}"
    );
}
