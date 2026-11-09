//! #955 review: the compiled foreach check must judge each comparison as
//! §11.8.1 does. A packed struct member of signed type (`int m`) compares
//! sign-extended, an unsigned element against a negative bound is an
//! unsigned comparison (`u[i] > -1` compares against 32'hFFFFFFFF and has
//! no solution), and a 64-bit unsigned element at or above 2^63 is a large
//! positive value. Expected values come from the reference simulator.

use xezim::simulate;

#[test]
fn compiled_foreach_check_follows_operand_signedness() {
    let src = r#"
typedef struct packed { int m; bit [3:0] t; } s_t;
class A;
  rand s_t p[3];
  constraint c { foreach (p[i]) { p[i].m inside {[-8:-2]}; p[i].t == 4'h9; } }
endclass
class U;
  rand bit [7:0] u[3];
  constraint c { foreach (u[i]) u[i] > -1; }
endclass
class S;
  rand int v[4];
  constraint c { foreach (v[i]) { v[i] < -10; v[i] >= -20; } }
endclass
class W;
  rand bit [63:0] w[2];
  constraint c { foreach (w[i]) w[i] >= 64'h8000_0000_0000_0000; }
endclass
module top;
  A a; U u; S s; W w; int ok, bad;
  initial begin
    a = new; u = new; s = new; w = new;
    ok = a.randomize();
    foreach (a.p[i]) if ($signed(a.p[i].m) < -8 || $signed(a.p[i].m) > -2 || a.p[i].t != 4'h9) bad++;
    $display("A ok=%0d bad=%0d", ok, bad);
    ok = u.randomize(); $display("U ok=%0d", ok);
    bad = 0;
    ok = s.randomize();
    foreach (s.v[i]) if (s.v[i] >= -10 || s.v[i] < -20) bad++;
    $display("S ok=%0d bad=%0d", ok, bad);
    ok = w.randomize(); $display("W ok=%0d top=%b", ok, w.w[0][63] & w.w[1][63]);
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.len() > 1 && m.as_bytes()[1] == b' ')
        .collect();
    assert_eq!(
        out,
        ["A ok=1 bad=0", "U ok=0", "S ok=1 bad=0", "W ok=1 top=1"]
    );
}
