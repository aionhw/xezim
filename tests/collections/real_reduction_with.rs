//! IEEE 1800-2023 §7.12.3: a reduction "returns a single value of ... the
//! type of the expression in the with clause", and the reduction operation
//! must be defined for that type. `+` and `*` are defined for real (Table
//! 11-1), so `sum` and `product` with a real `with` expression fold reals and
//! return a real (#1495). A real element type folds the same way. Values
//! checked on the reference simulator.

const SRC: &str = r#"
`timescale 1ns/1ps
module top;
  int q[$];
  int d[];
  int f[3];
  real s, p, c, r, rp, e;
  real ra[];
  real rq[$];
  shortreal sa[2];
  initial begin
    q = '{3, 1, 2};
    d = '{3, 1, 2};
    f = '{3, 1, 2};
    s = q.sum with (real'(item) * 0.5);
    p = q.product with (real'(item) + 0.5);
    $display("sum=%0g product=%0g", s, p);
    $display("dyn=%0g fixed=%0g", d.sum with (item * 0.25), f.product with (item / 2.0));
    c = q.sum with (item > 1 ? item * 1.5 : 0.25);
    $display("cond=%0g", c);
    $display("int=%0d %0d", q.sum with (int'(item * 0.5)), q.product with (int'(item * 0.5)));
    ra = '{1.25, 2.5, 0.5};
    rq = '{1.5, 2.0};
    sa = '{1.5, 0.25};
    r = ra.sum();
    rp = ra.product();
    e = rq.sum() + sa.sum();
    $display("reals=%0g %0g %0g", r, rp, e);
    $display("realwith=%0g", ra.sum with (item * 2.0));
  end
endmodule
"#;

#[test]
fn sum_and_product_fold_real_values() {
    let sim = xezim::simulate(SRC, 10).expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(
        lines,
        [
            "sum=3 product=13.125",
            "dyn=1.5 fixed=0.75",
            "cond=7.75",
            "int=4 2",
            "reals=4.25 1.5625 5.25",
            "realwith=8.5",
        ]
    );
}
