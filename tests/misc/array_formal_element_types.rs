//! §11.3.1 + §13.4 + §13.5: an element of an inlined function's dynamic-array
//! formal keeps its declared element type.
//!
//! A pure function with a dynamic-array formal fed by an assignment pattern
//! inlines with one register per element (the §6.6.7 resolver path). The
//! static type checks behind division and comparison did not know such an
//! element's type:
//!
//! * a `real` element: `d[k] / x` divided as integers and read 0 whenever the
//!   other operand was not itself real-typed (`1.0 / d[k]` and `d[0] / d[1]`
//!   with constant indices happened to work, which hid it);
//! * a signed integral element (`input int d[]`): the element was stored and
//!   compared unsigned, so `d[0] / 2` of -6 read 2147483645 and `d[0] < 0`
//!   was false.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .collect()
}

#[test]
fn element_dividend_divides_as_real() {
    let out = msgs(
        r#"
module top;
  function automatic real ratio(input real a[], input real b[]);
    real s; s = 0.0;
    foreach (a[k]) s += a[k] / b[k];
    return s;
  endfunction
  function automatic real by_first(input real a[], input real b[]);
    real s; s = 0.0;
    foreach (a[k]) s += a[k] / b[0];
    return s;
  endfunction
  function automatic real self_ratio(input real a[]);
    real s; s = 0.0;
    foreach (a[k]) s += a[k] / a[k];
    return s;
  endfunction
  function automatic real loop_ratio(input real a[], input real b[]);
    real s; s = 0.0;
    for (int k = 0; k < 2; k++) s += a[k] / b[k];
    return s;
  endfunction
  real r0, r1, r2, r3;
  assign r0 = ratio('{2.0, 3.0}, '{10.0, 20.0});
  assign r1 = by_first('{2.0, 3.0}, '{10.0, 20.0});
  assign r2 = self_ratio('{2.0, 4.0});
  assign r3 = loop_ratio('{2.0, 3.0}, '{10.0, 20.0});
  initial #1 $display("R %.3f %.3f %.3f %.3f", r0, r1, r2, r3);
endmodule
"#,
    );
    assert!(
        out.contains(&"R 0.350 0.500 2.000 0.350".to_string()),
        "{out:?}"
    );
}

#[test]
fn signed_elements_stay_signed() {
    let out = msgs(
        r#"
module top;
  function automatic int half(input int d[]);
    return d[0] / 2;
  endfunction
  function automatic int negs(input int d[]);
    int c; c = 0;
    foreach (d[k]) if (d[k] < 0) c++;
    return c;
  endfunction
  function automatic int sb(input byte d[]);
    return d[0] / 2 + (d[1] < 0 ? 1000 : 0);
  endfunction
  function automatic longint thirds(input longint d[]);
    longint s; s = 0;
    foreach (d[k]) s += d[k] / 3;
    return s;
  endfunction
  function automatic int ub(input bit [7:0] d[]);
    return d[0] / 2 + (d[1] < 8'd200 ? 1000 : 0);
  endfunction
  int m = -6;
  byte c = -100;
  bit [7:0] a = 8'd250, b = 8'd100;
  int r0, r1, r2, r4; longint r3;
  assign r0 = half('{m});
  assign r1 = negs('{m, 3, -1});
  assign r2 = sb('{c, c});
  assign r3 = thirds('{-9, 12, -3});
  assign r4 = ub('{a, b});
  initial #1 $display("R %0d %0d %0d %0d %0d", r0, r1, r2, r3, r4);
endmodule
"#,
    );
    assert!(out.contains(&"R -3 2 950 0 1125".to_string()), "{out:?}");
}
