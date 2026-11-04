//! IEEE 1800-2017 §18.8, taking `sub` as a random variable of `this` (an
//! interpretation; the object-form reading turns off every variable in
//! `sub`): inside a class, `sub.rand_mode(0)` on a `rand` class-handle
//! property names that variable. "If the random variable is an object
//! handle, only the mode of the variable is changed, not the mode of random
//! variables within that object." With the handle inactive the object it
//! holds is not randomized and its constraints are not solved: an inactive
//! variable "is treated the same as if it had not been declared rand"
//! (§18.8), and §18.5.9 a) randomizes only rand objects that are active.
//! Every random variable inside the object used to be switched off
//! instead, so its constraints were still checked against the current
//! values and the enclosing randomize() failed.

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
fn inactive_rand_handle_skips_the_sub_object() {
    let out = tagged(
        r#"
class sub_c;
  rand bit [7:0] a;
  rand bit [7:0] b;
  constraint gt_c { a > b; }
endclass
class outer_c;
  rand bit [7:0] x;
  rand sub_c sub;
  constraint x_c { x inside {[1:10]}; }
  function new();
    sub = new();
    sub.rand_mode(0);
  endfunction
  function void sub_on(); sub.rand_mode(1); endfunction
endclass
module top;
  initial begin
    outer_c o = new();
    int ok;
    ok = o.randomize();
    $display("T off ok=%0d x_ok=%0d a=%0d b=%0d sub=%0d sub.a=%0d", ok,
             o.x inside {[1:10]}, o.sub.a, o.sub.b, o.sub.rand_mode(), o.sub.a.rand_mode());
    o.sub_on();
    ok = o.randomize();
    $display("T on ok=%0d gt=%0d sub=%0d", ok, o.sub.a > o.sub.b, o.sub.rand_mode());
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        [
            "T off ok=1 x_ok=1 a=0 b=0 sub=0 sub.a=1",
            "T on ok=1 gt=1 sub=1",
        ]
    );
}
