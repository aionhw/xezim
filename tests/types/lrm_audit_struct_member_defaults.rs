//! IEEE 1800-2023 §7.2 / §7.2.2 / §6.8: a 2-state member of an unpacked
//! struct starts at 0 wherever the struct lives (module, block, class
//! property, array element), and a member's declared default applies to a
//! block-local or class-property struct too. Expected values come from the
//! reference simulator.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn t_lines(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn two_state_members_start_at_zero() {
    const SRC: &str = r#"
module rsd;
  typedef struct { int a; logic [3:0] b = 4'h5; } s1_t;
  typedef struct { int a; logic [3:0] b; } s2_t;
  typedef struct { int a; bit [3:0] c; byte d; } s3_t;
  s1_t m1;
  s2_t m2;
  s3_t m3;
  initial begin
    $display("T|r1|a=%0d b=%h", m1.a, m1.b);
    $display("T|r2|a=%0d b=%h", m2.a, m2.b);
    $display("T|r3|a=%0d c=%h d=%0d", m3.a, m3.c, m3.d);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        ["T|r1|a=0 b=5", "T|r2|a=0 b=x", "T|r3|a=0 c=0 d=0",],
    );
}

#[test]
fn member_defaults_on_block_local() {
    const SRC: &str = r#"
module rsd2;
  typedef struct { int a = 3; int b = 4; } dflt_t;
  dflt_t m1;
  initial begin
    dflt_t l1;
    $display("T|r1|%0d %0d / %0d %0d", m1.a, m1.b, l1.a, l1.b);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(t_lines(&sim), ["T|r1|3 4 / 3 4",],);
}

#[test]
fn struct_defaults_every_storage_kind() {
    const SRC: &str = r#"
module s1;
  typedef struct { int a; logic [3:0] b = 4'h5; } s1_t;
  typedef struct { int a; logic [3:0] b; } s2_t;
  typedef struct { int a; bit [3:0] c; byte d; } s3_t;
  typedef struct { s3_t in; integer e; shortint f; } nest_t;
  typedef struct { int a = 3; int b = 4; } dflt_t;
  s1_t m1;
  s2_t m2;
  s3_t m3;
  nest_t mn;
  s3_t arr [2];
  dflt_t md;
  struct { int x; longint y; real r; string s; } anon;
  class K; s3_t p; nest_t n; s3_t pa[2]; dflt_t d; endclass
  initial begin
    K k = new;
    s3_t l3;
    nest_t ln;
    dflt_t ld;
    $display("T|r1|a=%0d b=%h", m1.a, m1.b);
    $display("T|r2|a=%0d b=%h", m2.a, m2.b);
    $display("T|r3|a=%0d c=%h d=%0d", m3.a, m3.c, m3.d);
    $display("T|r4|%0d %0d %0d %0d", mn.in.a, mn.in.d, mn.e, mn.f);
    $display("T|r5|%0d %0d %0d", arr[1].a, arr[0].d, arr[1].c);
    $display("T|r6|%0d %0d %0d %s|", anon.x, anon.y, anon.r == 0.0, anon.s);
    $display("T|r7|%0d %0d %0d %0d", k.p.a, k.p.d, k.n.in.a, k.n.f);
    $display("T|r8|%0d %0d", k.pa[1].a, k.pa[0].d);
    $display("T|r9|%0d %0d %0d %0d", l3.a, l3.d, ln.in.a, ln.f);
    $display("T|r10|%0d %0d %0d %0d %0d %0d", md.a, md.b, ld.a, ld.b, k.d.a, k.d.b);
    $display("T|r11|%p", m3);
  end
endmodule"#;
    let sim = simulate(SRC, 1000).expect("sim");
    assert_eq!(
        t_lines(&sim),
        [
            "T|r1|a=0 b=5",
            "T|r2|a=0 b=x",
            "T|r3|a=0 c=0 d=0",
            "T|r4|0 0 x 0",
            "T|r5|0 0 0",
            "T|r6|0 0 1 |",
            "T|r7|0 0 0 0",
            "T|r8|0 0",
            "T|r9|0 0 0 0",
            "T|r10|3 4 3 4 3 4",
            "T|r11|'{a:0, c:'h0, d:0}",
        ],
    );
}
