//! IEEE 1800-2023 §6.6.7, §7.2 and §23.3.3: a resolver-less nettype takes
//! its single driver's value, including through output/inout and input ports.
//! Sample a time step after each change (§4.7), and check the parent net,
//! the driver's port and a second child's input-port reader.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 10_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

fn check_struct_port(direction: &str, body: &str) {
    let src = format!(
        r#"
typedef struct {{ real V; real R; int code; logic [7:0] tag; }} e_t;
nettype e_t enet;
module driver(input real v, input int k, {direction} enet p);
  {body}
endmodule
module reader(input enet p, output real v, output real r,
              output int k, output logic [7:0] tag);
  assign v = p.V;
  assign r = p.R;
  assign k = p.code;
  assign tag = p.tag;
endmodule
module top;
  real v, seen_v, seen_r;
  int k, seen_k;
  logic [7:0] seen_tag;
  enet n;
  driver u(.v(v), .k(k), .p(n));
  reader sink(.p(n), .v(seen_v), .r(seen_r), .k(seen_k), .tag(seen_tag));
  initial begin
    #1 v = 2.5; k = 7;
    #1;
    $display("R %.2f %.2f %0d %h", n.V, n.R, n.code, n.tag);
    $display("R %.2f %.2f %0d %h", u.p.V, u.p.R, u.p.code, u.p.tag);
    $display("R %.2f %.2f %0d %h", seen_v, seen_r, seen_k, seen_tag);
    #1 v = -3.25; k = -9;
    #1;
    $display("R %.2f %.2f %0d %h", n.V, n.R, n.code, n.tag);
    $display("R %.2f %.2f %0d %h", u.p.V, u.p.R, u.p.code, u.p.tag);
    $display("R %.2f %.2f %0d %h", seen_v, seen_r, seen_k, seen_tag);
  end
endmodule
"#
    );
    assert_eq!(
        lines(&src),
        vec![
            "R 2.50 1.00 7 a5",
            "R 2.50 1.00 7 a5",
            "R 2.50 1.00 7 a5",
            "R -3.25 1.00 -9 a5",
            "R -3.25 1.00 -9 a5",
            "R -3.25 1.00 -9 a5",
        ],
        "{direction}: {body}"
    );
}

#[test]
fn output_ordered_pattern() {
    check_struct_port("output", "assign p = '{v, 1.0, k, 8'ha5};");
}

#[test]
fn inout_ordered_pattern() {
    check_struct_port("inout", "assign p = '{v, 1.0, k, 8'ha5};");
}

#[test]
fn output_named_pattern() {
    check_struct_port("output", "assign p = '{tag: 8'ha5, code: k, R: 1.0, V: v};");
}

#[test]
fn inout_named_pattern() {
    check_struct_port("inout", "assign p = '{tag: 8'ha5, code: k, R: 1.0, V: v};");
}

#[test]
fn output_struct_variable() {
    check_struct_port(
        "output",
        "e_t t; always_comb t = '{v, 1.0, k, 8'ha5}; assign p = t;",
    );
}

#[test]
fn inout_struct_variable() {
    check_struct_port(
        "inout",
        "e_t t; always_comb t = '{v, 1.0, k, 8'ha5}; assign p = t;",
    );
}

#[test]
fn direct_top_level_struct_control() {
    assert_eq!(
        lines(
            r#"
typedef struct { real V; real R; } e_t;
nettype e_t enet;
module top;
  real v;
  enet n;
  assign n = '{v, 1.0};
  initial begin
    #1 v = 2.5;
    #1 $display("R %.1f %.1f", n.V, n.R);
    #1 v = -3.25;
    #1 $display("R %.2f %.1f", n.V, n.R);
  end
endmodule
"#
        ),
        vec!["R 2.5 1.0", "R -3.25 1.0"]
    );
}

#[test]
fn scalar_real_port_control() {
    for direction in ["output", "inout"] {
        let src = format!(
            r#"
nettype real rnet;
module driver(input real v, {direction} rnet p); assign p = v; endmodule
module reader(input rnet p, output real v); assign v = p; endmodule
module top;
  real v, seen;
  rnet n;
  driver u(.v(v), .p(n));
  reader sink(.p(n), .v(seen));
  initial begin
    #1 v = 2.5;
    #1 $display("R %.2f %.2f %.2f", n, u.p, seen);
    #1 v = -3.25;
    #1 $display("R %.2f %.2f %.2f", n, u.p, seen);
  end
endmodule
"#
        );
        assert_eq!(
            lines(&src),
            vec!["R 2.50 2.50 2.50", "R -3.25 -3.25 -3.25"],
            "{direction}"
        );
    }
}

/// §13.4 and §6.21: a function's effects and its local struct stay local to
/// the call; moving its result through a nettype port must not replay it or
/// copy a same-named module struct instead.
#[test]
fn function_effects_and_local_struct_shadowing() {
    for direction in ["output", "inout"] {
        let src = format!(
            r#"
typedef struct {{ real V; int code; }} e_t;
nettype e_t enet;
module driver({direction} enet p);
  e_t t, staged;
  int calls = 0;
  function automatic e_t make(input real v, input int k);
    e_t t;
    calls++;
    t = '{{v, k}};
    return t;
  endfunction
  assign p = staged;
  initial begin
    t = '{{99.0, 99}};
    #1 staged = make(2.5, 7);
    #2 staged = make(-3.25, -9);
  end
endmodule
module top;
  enet n;
  driver u(.p(n));
  initial begin
    #2 $display("R %.2f %0d %.1f %0d %0d", n.V, n.code, u.t.V, u.t.code, u.calls);
    #2 $display("R %.2f %0d %.1f %0d %0d", n.V, n.code, u.t.V, u.t.code, u.calls);
  end
endmodule
"#
        );
        assert_eq!(
            lines(&src),
            vec!["R 2.50 7 99.0 99 1", "R -3.25 -9 99.0 99 2"],
            "{direction}"
        );
    }
}
