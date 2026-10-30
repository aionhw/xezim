//! §11.4.11: the conditional operator evaluates only the operand its
//! condition selects. Reading the other arm to learn its type evaluated a
//! dotted name too, so `(h != null) ? h.v : -1` with a null `h` reported a
//! fatal null dereference and ended the process.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> i64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_i64()
        .unwrap_or_else(|| panic!("{} not i64-able (x/z?)", n))
}

const SRC: &str = r#"
class obj; int v; string name; endclass
module top;
  obj o = null, p;
  int a = 5, b = 5, done = 0;
  string s;
  initial begin
    a = (o != null) ? o.v : -1;
    b = (o == null) ? -2 : o.v;
    s = (o != null) ? o.name : "none";
    p = new;
    p.v = 7;
    if (((p != null) ? p.v : -1) == 7) done = 1;
  end
endmodule
"#;

#[test]
fn unselected_arm_is_not_read() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "a"), -1);
    assert_eq!(u(&sim, "b"), -2);
    assert_eq!(u(&sim, "done"), 1, "the process went on");
}
