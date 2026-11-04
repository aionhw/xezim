//! §8.25, §13.5: the actuals of a call to another specialization's static
//! method (`registry#(cb_t)::add(cb)`) are evaluated in the caller's scope.
//! A static of the caller's own parameterized class read there (`cb` inside
//! `seq#(int)`) keyed generically instead of by `seq#(int)`, so the callee
//! received null. Expected values are the reference simulator's.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

#[test]
fn caller_static_passed_to_other_specialization() {
    const SRC: &str = r#"class cb_t;
  int id;
endclass
class registry #(type T = cb_t);
  static function void add(T cb);
    $display("T|add null=%0d id=%0d", cb == null, cb == null ? -1 : cb.id);
  endfunction
endclass
class seq #(type T = int);
  static cb_t cb;
  function new(int id);
    if (cb == null) begin cb = new(); cb.id = id; end
    registry#(cb_t)::add(cb);
  endfunction
endclass
module top;
  initial begin
    seq#(int) a = new(1);
    seq#(real) b = new(2);
    seq#(int) c = new(3);
  end
endmodule
"#;
    assert_eq!(
        t_lines(SRC),
        [
            "T|add null=0 id=1",
            "T|add null=0 id=2",
            "T|add null=0 id=1"
        ]
    );
}
