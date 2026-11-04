//! IEEE 1800-2017 §18.5.4: an item of weight 0 is not in a `dist`'s set, so
//! a dist whose weights are all 0 admits no value and `randomize()` fails,
//! whether the weights are literals or variables and for `:=` and `:/`
//! alike. It returned 1 with the first item (#770). Items with a nonzero
//! weight still solve, a zero-weight item is never drawn, and a value forced
//! onto a zero-weight item fails.

use xezim::simulate;

fn tagged(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T "))
        .collect()
}

#[test]
fn a_dist_whose_weights_are_all_zero_is_unsatisfiable() {
    let out = tagged(
        r#"
class one;  rand bit [3:0] v; constraint c { v dist { 5 := 0 }; } endclass
class two;  rand bit [3:0] v; constraint c { v dist { 0 := 0, [1:15] := 0 }; } endclass
class div;  rand bit [3:0] v; constraint c { v dist { [0:15] :/ 0 }; } endclass
class cfg;  rand bit [3:0] v; int w0, w1; constraint c { v dist { 0 := w0, 1 := w1 }; } endclass
class pair; rand bit [3:0] v; rand bit [3:0] u; constraint c { v dist { 5 := 0 }; u < 4; } endclass
class mix;  rand bit [3:0] v; constraint c { v dist { 0 := 0, [1:15] := 1 }; } endclass
class frc;  rand bit [3:0] v; constraint c { v dist { 0 := 0, [1:15] := 1 }; v == 0; } endclass
class off;  rand bit [3:0] v; constraint c { if (0) v dist { 5 := 0 }; } endclass
class half; rand bit [3:0] v; constraint c { v dist { [0:3] :/ 0, [4:7] :/ 2 }; } endclass
module tb;
  initial begin
    one a = new; two b = new; div c = new; cfg d = new; pair e = new;
    mix m = new; frc f = new; off g = new; half h = new;
    int ok, low, x;
    $display("T one %0d two %0d div %0d", a.randomize(), b.randomize(), c.randomize());
    $display("T cfg zero %0d", d.randomize());
    d.w1 = 3;
    $display("T cfg some %0d v %0d", d.randomize(), d.v);
    $display("T pair %0d", e.randomize());
    for (int k = 0; k < 200; k++) begin if (m.randomize()) ok++; if (m.v == 0) low++; end
    $display("T mix %0d zero %0d", ok, low);
    $display("T forced %0d", f.randomize());
    $display("T guarded off %0d", g.randomize());
    ok = 0; low = 0;
    for (int k = 0; k < 200; k++) begin if (h.randomize()) ok++; if (h.v < 4) low++; end
    $display("T half %0d low %0d", ok, low);
    $display("T std %0d", std::randomize(x) with { x dist { 1 := 0 }; });
  end
endmodule
"#,
    );
    let want = [
        "T one 0 two 0 div 0",
        "T cfg zero 0",
        "T cfg some 1 v 1",
        "T pair 0",
        "T mix 200 zero 0",
        "T forced 0",
        "T guarded off 1",
        "T half 200 low 0",
        "T std 0",
    ];
    assert_eq!(out, want, "{out:#?}");
}
