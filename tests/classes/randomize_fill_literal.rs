//! IEEE 1800-2023 §5.7.1 / §18.3: an unsized `'1` or `'0` in a constraint
//! fills the width of the other operand. The joint solver and the
//! constraint check both took `'1` as the 1-bit value 1, so `y == '1` on an
//! 8-bit `y` produced `8'h01` and `y != '1` admitted `8'hff`.
use xezim::simulate;

#[test]
fn unsized_fill_literals_take_the_context_width_in_constraints() {
    let sim = simulate(
        "typedef struct packed { logic en; logic [34:0] data; logic stop; } narrow_t;
class c;
  rand narrow_t x;
  rand bit [7:0] y;
  rand bit [7:0] a[2];
  rand narrow_t rows[][2];
endclass
module t;
  initial begin
    c o = new();
    bit [7:0] w;
    bit ok1, ok2, ok3, ok4, ok5, ok6;
    ok1 = o.randomize() with { y == '1; };
    $display(\"y=%h ok=%0d\", o.y, ok1);
    ok2 = o.randomize() with { x == '1; };
    $display(\"x=%h ok=%0d\", o.x, ok2);
    ok3 = o.randomize() with { a[1] == '1; };
    $display(\"a1=%h ok=%0d\", o.a[1], ok3);
    ok4 = o.randomize() with { y != '1; y > 8'hfd; };
    $display(\"yne=%h ok=%0d\", o.y, ok4);
    ok5 = o.randomize() with { rows.size() == 1; rows[0][1] == '1; };
    $display(\"row=%h ok=%0d\", o.rows[0][1], ok5);
    ok6 = std::randomize(w) with { w == '1; };
    $display(\"w=%h ok=%0d\", w, ok6);
  end
endmodule
",
        1_000,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    for want in [
        "y=ff ok=1",
        "x=1fffffffff ok=1",
        "a1=ff ok=1",
        "yne=fe ok=1",
        "row=1fffffffff ok=1",
        "w=ff ok=1",
    ] {
        assert!(msgs.iter().any(|m| m == want), "missing {want:?} in {msgs:?}");
    }
}
