//! §6.8/§7.2 — an unpacked-struct local of a function, task or class method.
//! Its member leaves were all seeded x, so a 2-state member (`int n`,
//! `bit [2:0] k`) read x instead of 0, and a packed-struct member had no field
//! layout, so `m.p.a = 7` wrote nothing and `m.p` stayed x.
//!
//! The expected lines were cross-checked against the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

const SRC: &str = r#"
typedef struct packed { logic [3:0] a; logic [3:0] b; } pk_t;
typedef struct { pk_t p; int n; logic [3:0] l; } mix_t;
typedef struct { mix_t inner; bit [2:0] k; } outer_t;
class C;
  function automatic mix_t mk();
    mix_t m;
    m.p.a = 4'h3;
    return m;
  endfunction
endclass
module top;
  function automatic int f();
    mix_t m;
    m.p.a = 4'h7; m.p.b = 4'h8;
    $display("f: m=%p n=%0d", m, m.n);
    return m.p;
  endfunction
  task automatic t();
    outer_t o;
    o.inner.p.b = 4'h9;
    $display("t: o=%p", o);
  endtask
  initial begin
    C c = new;
    mix_t r;
    $display("f=%h", f());
    t();
    r = c.mk();
    $display("mk=%p", r);
  end
endmodule
"#;

#[test]
fn subroutine_struct_local_defaults_and_packed_members() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        got,
        vec![
            "f: m='{p:'{a:'h7, b:'h8}, n:0, l:'hX} n=0",
            "f=00000078",
            "t: o='{inner:'{p:'{a:'hX, b:'h9}, n:0, l:'hX}, k:'h0}",
            "mk='{p:'{a:'h3, b:'hX}, n:0, l:'hX}",
        ]
    );
}
