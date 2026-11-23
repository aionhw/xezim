//! §10.9.1, §11.6.1. Each ordered or replicated initializer item takes
//! the element type of the declared queue or dynamic array.
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
    static int sq[$] = '{1'b1 + 1'b1};
    static int sa[] = '{2{1'b1 + 1'b1}};
  endclass
  initial begin
    C c = new;
    $display("instance=%0d,%0d,%0d,%0d", c.q[0], c.a.size(), c.a[0], c.a[1]);
    $display("static=%0d,%0d,%0d,%0d", C::sq[0], C::sa.size(), C::sa[0], C::sa[1]);
  end
endmodule
"#,
        100,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(lines.contains(&"instance=2,2,2,2"), "{lines:?}");
    assert!(lines.contains(&"static=2,2,2,2"), "{lines:?}");
}
