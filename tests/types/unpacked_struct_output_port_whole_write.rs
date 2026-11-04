//! §7.2 + §23.3.3: a child module writing a WHOLE unpacked struct to its
//! output port.
//!
//! An unpacked struct is stored one signal per member, and the port's own
//! container signal only carries its width. Whole-struct writes in the child
//! never reached those members:
//!
//!     module ch(input int x, output S o);
//!       assign o = '{x, 7};          // parent read '{a:0, b:0}
//!     endmodule
//!
//! * a continuous assign in a lazily inlined child stayed in the pending
//!   list that the member-wise expansion never visited, and a NAMED pattern
//!   (`'{a: x, b: 7}`) had no member-wise form at all;
//! * the child's processes (`always_comb o = '{..}`, `o.a = v` in an
//!   `initial`) wrote slices of the width-only container, because the
//!   inlined port registered a packed field layout the root module's ports
//!   never get.
//!
//! The same declarations in the root module worked; member-wise `assign
//! o.a = x;` worked. The visible failure in practice was an electrical
//! (EEnet-style) model: a struct nettype driven through child output ports
//! resolved to all zeros.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    simulate(src, 1_000_000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("R "))
        .collect()
}

#[test]
fn continuous_whole_struct_writes_reach_the_parent() {
    let out = lines(
        r#"
typedef struct { int a; int b; } si_t;
typedef struct { real a; real b; } sr_t;
module c_ord(input int x, output si_t o);   assign o = '{x, 7};         endmodule
module c_named(input int x, output si_t o); assign o = '{a: x, b: 7};   endmodule
module c_var(input int x, output si_t o);   si_t t; assign t = '{x, 7}; assign o = t; endmodule
module c_real(input real x, output sr_t o); assign o = '{b: 0.5, a: x}; endmodule
module top;
  int x; si_t p, q, r; sr_t s;
  c_ord u0(.x(x), .o(p)); c_named u1(.x(x), .o(q)); c_var u2(.x(x), .o(r));
  c_real u3(.x(real'(x) * 1.5), .o(s));
  initial begin
    x = 3; #1 $display("R %0d %0d %0d %0d %0d %0d %.2f %.2f", p.a, p.b, q.a, q.b, r.a, r.b, s.a, s.b);
    x = 4; #1 $display("R %0d %0d %0d %0d %0d %0d %.2f %.2f", p.a, p.b, q.a, q.b, r.a, r.b, s.a, s.b);
  end
endmodule
"#,
    );
    assert_eq!(
        out,
        vec!["R 3 7 3 7 3 7 4.50 0.50", "R 4 7 4 7 4 7 6.00 0.50"]
    );
}

#[test]
fn procedural_writes_in_the_child_reach_the_parent() {
    let out = lines(
        r#"
typedef struct { int a; int b; } si_t;
module c_comb(input int x, output si_t o); always_comb o = '{x, 7};         endmodule
module c_star(input int x, output si_t o); always @* o = '{a: x, b: 7};     endmodule
module c_init(output si_t o);              initial #1 o = '{5, 6};          endmodule
module c_mem(output si_t o);               initial #1 begin o.a = 5; o.b = 6; end endmodule
module top;
  int x; si_t p, q, r, s;
  c_comb u0(.x(x), .o(p)); c_star u1(.x(x), .o(q)); c_init u2(.o(r)); c_mem u3(.o(s));
  initial begin
    x = 3; #2 $display("R %0d %0d %0d %0d %0d %0d %0d %0d", p.a, p.b, q.a, q.b, r.a, r.b, s.a, s.b);
    $display("R %0d %0d", u3.o.a, u3.o.b);
  end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 3 7 3 7 5 6 5 6", "R 5 6"]);
}

/// §6.6.7: a struct nettype (EEnet-style V/I/R Thevenin drivers) resolved
/// from drivers that sit in child modules behind output/inout ports.
#[test]
fn struct_nettype_resolves_drivers_behind_child_ports() {
    let out = lines(
        r#"
package ee;
  typedef struct { real V; real I; real R; } ee_t;
  function automatic ee_t res(input ee_t d[]);
    real g, i; ee_t r;
    g = 0.0; i = 0.0;
    foreach (d[k]) begin g += 1.0 / d[k].R; i += d[k].V / d[k].R + d[k].I; end
    r.V = i / g; r.R = 1.0 / g; r.I = i;
    return r;
  endfunction
  nettype ee_t eenet with res;
endpackage
import ee::*;
module vsrc(input real v, output eenet p); assign p = '{V: v, I: 0.0, R: 1.0e3}; endmodule
module rgnd(inout eenet p);                assign p = '{V: 0.0, I: 0.0, R: 1.0e3}; endmodule
module top;
  real v; eenet n;
  vsrc u0(.v(v), .p(n)); rgnd u1(.p(n));
  initial begin v = 2.0; #1 $display("R %.3f %.1f", n.V, n.R); end
endmodule
"#,
    );
    assert_eq!(out, vec!["R 1.000 500.0"]);
}
