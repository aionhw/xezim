//! §10.9.1, §11.6.1. Each ordered, replicated, or `default:` initializer item
//! takes the element type of the declared queue, dynamic, or fixed-size array.
use xezim::simulate;

#[test]
fn property_items_take_the_declared_width() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let sim = simulate(
        r#"
module top;
  class C;
    int q[$] = '{1'b1 + 1'b1};
    int a[] = '{2{1'b1 + 1'b1}};
    int fix_a[2] = '{1'b1 + 1'b1, 2'b11 + 2'b01};
    int fix_rep[2] = '{2{1'b1 + 1'b1}};
    int fix_def[2] = '{default: 1'b1 + 1'b1};
    static int sq[$] = '{1'b1 + 1'b1};
    static int sa[] = '{2{1'b1 + 1'b1}};
    static int sfix[2] = '{1'b1 + 1'b1, 2'b11 + 2'b01};
  endclass
  initial begin
    C c = new;
    $display("instance=%0d,%0d,%0d,%0d", c.q[0], c.a.size(), c.a[0], c.a[1]);
    $display("fixed=%0d,%0d,%0d,%0d,%0d,%0d",
      c.fix_a[0], c.fix_a[1], c.fix_rep[0], c.fix_rep[1], c.fix_def[0], c.fix_def[1]);
    $display("static=%0d,%0d,%0d,%0d,%0d,%0d",
      C::sq[0], C::sa.size(), C::sa[0], C::sa[1], C::sfix[0], C::sfix[1]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(lines.contains(&"instance=2,2,2,2"), "{lines:?}");
    assert!(lines.contains(&"fixed=2,4,2,2,2,2"), "{lines:?}");
    assert!(lines.contains(&"static=2,2,2,2,2,4"), "{lines:?}");
}
