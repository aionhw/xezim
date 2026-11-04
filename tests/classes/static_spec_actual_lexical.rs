//! §8.25, §13.5: a specialization written in an actual argument
//! (`registry#(base)::add(sequencer#(T)::cb)` inside `driver#(byte)`)
//! resolves `T` in the caller's class. The callee's specialization was
//! already active while the actuals were evaluated, so `T` read as
//! `registry`'s parameter and the call received another specialization's
//! (null) static. Expected values are the reference simulator's.

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
fn specialized_static_actual_resolves_in_the_caller() {
    const SRC: &str = r#"
class callback_base;
endclass
class callback_derived extends callback_base;
endclass
class registry #(type T = callback_base);
  static bit got_null;
  static function void add(T cb);
    got_null = (cb == null);
  endfunction
  static function bit is_null(T cb);
    return cb == null;
  endfunction
endclass
class sequencer #(type T = int);
  static callback_derived cb;
  static function void init();
    if (cb == null) cb = new();
  endfunction
  static function callback_derived get();
    return cb;
  endfunction
endclass
class driver #(type T = int);
  function new();
    sequencer#(T)::init();
    registry#(callback_base)::add(sequencer#(T)::cb);
    $display("T|static read: null=%0d", registry#(callback_base)::got_null);
    $display("T|static call: null=%0d",
             registry#(callback_base)::is_null(sequencer#(T)::get()));
    $display("T|other spec: null=%0d", sequencer#(callback_base)::cb == null);
  endfunction
endclass
module top;
  initial begin
    driver#(byte) d = new();
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|static read: null=0",
            "T|static call: null=0",
            "T|other spec: null=1",
        ]
    );
}
