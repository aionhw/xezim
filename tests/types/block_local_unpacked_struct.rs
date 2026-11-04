//! §7.2/§6.8 — an unpacked-struct local of an `initial` block (no call
//! frame). Its member leaves were created only by writes that happened to
//! name a whole leaf, so a write through a selected member
//! (`t.rows[0].level = 2`), a packed member's field (`m.p.a = 5`) and a
//! declaration initializer (`row_t r = '{...}`) were lost, and untouched
//! 2-state members read x instead of 0. The leaves are now seeded with their
//! declared defaults when the declaration executes, as module-scope structs
//! and subroutine locals already were.
//!
//! The expected lines were cross-checked against the reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

const SRC: &str = r#"
typedef struct { int level; logic [3:0] w; } row_t;
typedef struct { row_t rows[2]; int k; logic [7:0] tag; } tab_t;
typedef struct packed { logic [3:0] a; logic [3:0] b; } pk_t;
typedef struct { pk_t p; int n; } mix_t;
module top;
  function automatic row_t mk(int l);
    row_t r;
    r.level = l; r.w = 4'h3;
    return r;
  endfunction
  initial begin
    row_t a, b, c;
    tab_t t, u;
    mix_t m;
    $display("fresh a=%p t=%p", a, t);
    a.level = 1; a.w = 4'h2;
    b = a;
    $display("b.level=%0d b.w=%0d", b.level, b.w);
    c = mk(9);
    $display("c.level=%0d c.w=%0d", c.level, c.w);
    t.rows[0] = a;
    t.rows[1].level = 7;
    t.tag = 8'hA5;
    u = t;
    $display("u=%p", u);
    $display("u.rows[0].w=%0d u.rows[1].level=%0d", u.rows[0].w, u.rows[1].level);
    m.p.a = 4'h5; m.p.b = 4'h6; m.n = 3;
    $display("m=%p m.p=%h", m, m.p);
    for (int i = 0; i < 2; i++) t.rows[i].level = 10 + i;
    $display("loop %0d %0d", t.rows[0].level, t.rows[1].level);
  end
  initial begin
    row_t q[$];
    row_t e;
    e.level = 3;
    q.push_back(e);
    $display("q[0].level=%0d q.size=%0d", q[0].level, q.size());
  end
  initial begin
    row_t r = '{level: 4, w: 4'h1};
    tab_t s = '{rows: '{'{1, 4'h2}, '{3, 4'h4}}, k: 5, tag: 8'h6};
    $display("r=%p s.rows[1].level=%0d s.k=%0d", r, s.rows[1].level, s.k);
    s.rows[1].level = 8;
    $display("s=%p", s);
  end
endmodule
"#;

#[test]
fn initial_block_struct_locals() {
    let sim = simulate(SRC, 1000).expect("simulate failed");
    let got: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        got,
        vec![
            "fresh a='{level:0, w:'hX} t='{rows:'{'{level:0, w:'hX}, '{level:0, w:'hX}}, k:0, tag:'hXX}",
            "b.level=1 b.w=2",
            "c.level=9 c.w=3",
            "u='{rows:'{'{level:1, w:'h2}, '{level:7, w:'hX}}, k:0, tag:'ha5}",
            "u.rows[0].w=2 u.rows[1].level=7",
            "m='{p:'{a:'h5, b:'h6}, n:3} m.p=56",
            "loop 10 11",
            "q[0].level=3 q.size=1",
            "r='{level:4, w:'h1} s.rows[1].level=3 s.k=5",
            "s='{rows:'{'{level:1, w:'h2}, '{level:8, w:'h4}}, k:5, tag:'h6}",
        ]
    );
}
