//! IEEE 1800-2023 §23.3.3.5: unpacked actuals split left-to-left in declared
//! instance order; scalar actuals broadcast. §7.2 preserves member types and
//! §6.6.7 resolves all drivers of a connected user-defined net together.

use xezim::simulate;

fn results(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|s| s.starts_with("R "))
        .collect()
}

#[test]
fn output_repro() {
    let src = r#"
typedef struct { int a; int b; } s_t;
module ch(input int x, output s_t o); assign o = '{x, 7}; endmodule
module top;
  int x; s_t w[2];
  ch u[1:0] (.x(x), .o(w));
  initial begin
    #1 x = 3;
    #1 $display("R %0d %0d %0d %0d", w[0].a, w[0].b, w[1].a, w[1].b);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 3 7 3 7"]);
    let nested = format!(
        "{}\nmodule top; wrapper w(); endmodule",
        src.replace("module top;", "module wrapper;")
    );
    assert_eq!(results(&nested), ["R 3 7 3 7"]);
}

/// §23.3.3.5: the leftmost instance takes the leftmost unpacked element,
/// independently of either range's numeric labels. Check inside each child
/// as well as at the parent: a reversed input/output round trip can hide a bug.
#[test]
fn split_input_output_declared_order() {
    for (instances, actuals, first, second) in [
        ("1:0", "4:5", 5, 4),
        ("0:1", "4:5", 4, 5),
        ("1:0", "5:4", 4, 5),
        ("0:1", "5:4", 5, 4),
    ] {
        let src = format!(
            r#"
typedef struct {{ int a; int b; }} s_t;
module ch(input s_t i, output s_t o);
  assign o = '{{i.a + 10, i.b + 20}};
endmodule
module top;
  s_t x[{actuals}], w[{actuals}];
  ch u[{instances}] (.o(w), .i(x));
  initial begin
    #1 x[4] = '{{4, 40}}; x[5] = '{{5, 50}};
    #1 $display("R %0d %0d %0d %0d %0d %0d", u[0].i.a, u[1].i.a,
                w[4].a, w[4].b, w[5].a, w[5].b);
    #1 x[4] = '{{6, 60}}; x[5] = '{{7, 70}};
    #1 $display("R %0d %0d %0d %0d", w[4].a, w[4].b, w[5].a, w[5].b);
  end
endmodule
"#
        );
        assert_eq!(
            results(&src),
            [
                format!("R {first} {second} 14 60 15 70"),
                "R 16 80 17 90".to_string()
            ],
            "instance range {instances}, actual range {actuals}"
        );
    }
}

/// §7.2 / §23.3.3.5: a single nested struct broadcasts, and fractional real
/// members survive both continuous and procedural whole-struct output writes.
#[test]
fn broadcast_nested_real_continuous_and_procedural() {
    let src = r#"
typedef struct { real v; int tag; } inner_t;
typedef struct { inner_t inner; real r; } s_t;
module comb(input s_t i, output s_t o); always_comb o = i; endmodule
module cont(input s_t i, output s_t o); assign o = i; endmodule
module top;
  s_t x, a[2], b[2];
  comb p[0:1] (.i(x), .o(a));
  cont c[1:0] (.i(x), .o(b));
  initial begin
    #1 x = '{'{1.25, 3}, 2.75};
    #1 $display("R %0.2f %0d %0.2f %0.2f %0d %0.2f",
                a[0].inner.v, a[1].inner.tag, a[1].r,
                b[0].inner.v, b[1].inner.tag, b[1].r);
    #1 x = '{'{4.50, 9}, 0.125};
    #1 $display("R %0.2f %0d %0.3f %0.2f %0d %0.3f",
                a[1].inner.v, a[0].inner.tag, a[0].r,
                b[1].inner.v, b[0].inner.tag, b[0].r);
  end
endmodule
"#;
    assert_eq!(
        results(src),
        ["R 1.25 3 2.75 1.25 3 2.75", "R 4.50 9 0.125 4.50 9 0.125"]
    );
}

/// §23.3.3 / §7.2: explicit element actuals must use the same member-wise
/// connection as elements synthesized by instance-array expansion.
#[test]
fn explicit_element_actuals() {
    let src = r#"
typedef struct { real r; int n; } s_t;
module ch(input s_t i, output s_t o); always_comb o = i; endmodule
module top;
  s_t x[2], w[2];
  ch a (.i(x[1]), .o(w[0]));
  ch b (.i(x[0]), .o(w[1]));
  initial begin
    #1 x[0] = '{1.25, 3}; x[1] = '{2.75, 4};
    #1 $display("R %0.2f %0d %0.2f %0d", w[0].r, w[0].n, w[1].r, w[1].n);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 2.75 4 1.25 3"]);
}

/// §7.2 / §23.3.3.5: split nested structs with fractional members in both
/// directions, and retain each formal's own member storage in the child.
#[test]
fn split_nested_real_ports() {
    let src = r#"
typedef struct { real v; int tag; } inner_t;
typedef struct { inner_t inner; real r; } s_t;
module ch(input s_t i, output s_t o);
  always_comb o = '{'{i.inner.v + 0.25, i.inner.tag + 1}, i.r + 0.125};
endmodule
module top;
  s_t x[3:2], w[8:9];
  ch u[0:1] (.i(x), .o(w));
  initial begin
    #1 x[3] = '{'{1.25, 3}, 2.5}; x[2] = '{'{4.5, 7}, 0.25};
    #1 $display("R %0.2f %0d %0.3f %0.2f %0d %0.3f",
                w[8].inner.v, w[8].inner.tag, w[8].r,
                w[9].inner.v, w[9].inner.tag, w[9].r);
    $display("R %0.2f %0.2f", u[0].i.inner.v, u[1].i.inner.v);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 1.50 4 2.625 4.75 8 0.375", "R 1.25 4.50"]);
}

/// §13.4 / §23.3.3.5: copying a returned struct evaluates a call once, and
/// a function-local `o` shadows the identically named output port.
#[test]
fn procedural_function_side_effect_and_shadowing() {
    let src = r#"
typedef struct { int a; int b; } s_t;
module ch(input int x, output s_t o);
  int calls = 0;
  function automatic s_t make(input int v);
    s_t o;
    calls++;
    o.a = v;
    o.b = v + 7;
    return o;
  endfunction
  always_comb o = make(x);
endmodule
module top;
  int x, before0, before1; s_t w[2];
  ch u[1:0] (.x(x), .o(w));
  initial begin
    #1 before0 = u[0].calls; before1 = u[1].calls; x = 3;
    #1 $display("R %0d %0d %0d %0d %0d %0d", w[0].a, w[0].b,
                w[1].a, w[1].b, u[0].calls-before0, u[1].calls-before1);
  end
endmodule
"#;
    assert_eq!(results(src), ["R 3 10 3 10 1 1"]);
}

/// §6.6.7 / §23.3.3.5: all four terminal drivers of an instance array
/// collapse onto one node. Averaging V and summing conductance is deliberately
/// non-associative, so resolving each port separately cannot pass this test.
#[test]
fn resolved_nettype_broadcast_two_terminals() {
    let src = r#"
typedef struct { real V; real I; real R; int count; } ee_t;
function automatic ee_t resolve(input ee_t drivers[]);
  ee_t r;
  real conductance;
  r = '{0.0, 0.0, 0.0, 0};
  conductance = 0.0;
  foreach (drivers[k]) begin
    r.V += drivers[k].V;
    r.I += drivers[k].I;
    conductance += 1.0 / drivers[k].R;
    r.count++;
  end
  r.V /= drivers.size();
  r.R = 1.0 / conductance;
  return r;
endfunction
nettype ee_t EEnet with resolve;
module element(input real v, inout EEnet p, n);
  ee_t drive;
  always_comb drive = '{v, 0.25, 4.0, 0};
  assign p = drive;
  assign n = '{2.0, -0.125, 8.0, 0};
endmodule
module wrapper;
  real v;
  EEnet node;
  element u[1:0] (.v(v), .p(node), .n(node));
  initial begin
    #1 v = 6.0;
    #1 $display("R %0.2f %0.2f %0.3f %0d", node.V, node.I, node.R, node.count);
    #1 v = 10.0;
    #1 $display("R %0.2f %0.2f %0.3f %0d", node.V, node.I, node.R, node.count);
  end
endmodule
module top; wrapper w(); endmodule
"#;
    for range in ["1:0", "0:1"] {
        assert_eq!(
            results(&src.replace("u[1:0]", &format!("u[{range}]"))),
            ["R 4.00 0.25 1.333 4", "R 6.00 0.25 1.333 4"]
        );
    }
}
