//! IEEE 1800-2017 §7.2 / §10.7 — a member of an unpacked struct has its
//! declared type, and an assignment to it extends the right-hand side to the
//! member's width and stores it with the MEMBER's signedness. A block local's
//! leaves were seeded without the member's signedness, so an `int` stored
//! into a `longint` member kept the right bits but read back unsigned
//! (`-4` displayed as 18446744073709551612, `z.r < 0` false). The same leaf
//! filled by a struct-returning function, and a subroutine local's unsigned
//! member stored from a signed value, took the rvalue's signedness instead of
//! the member's.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

const SRC: &str = r#"
module tb;
  typedef struct { longint r; int i; byte unsigned u; } s_t;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } arr;
  function automatic arr mk(longint r); arr a; a.r = r; return a; endfunction
  function automatic arr wrap(longint r); return mk(r); endfunction
  function automatic longint f(int b);
    s_t a;
    a.r = b; a.u = b;
    $display("F|%0d %0d", a.r, a.u);
    return a.r;
  endfunction
  initial begin
    int b; shortint sh; byte by;
    s_t z, t; s_t q[$]; s_t d[]; arr x;
    b = -4; sh = -5; by = -6;
    z.r = b;
    $display("A|%0d", z.r);
    z.r = longint'(b);
    $display("B|%0d", z.r);
    z.i = sh;
    $display("C|%0d", z.i);
    z.i = by;
    $display("D|%0d", z.i);
    z.r = b; z.u = b;
    $display("E|%0d %0d %0d", z.r < 0, z.r + 1, z.u);
    q.push_back(z); t = q[0]; d = new[1]; d[0] = z;
    $display("G|%0d %0d %0d", q[0].r, t.r, d[0].r);
    x = wrap(b);
    $display("H|%0d", x.r);
    x = mk(b);
    $display("I|%0d", x.r);
    $display("J|%0d", f(b));
  end
endmodule
"#;

#[test]
fn narrower_signed_store_keeps_member_signedness() {
    let sim = simulate(SRC, 20).expect("simulate failed");
    let got = msgs(&sim);
    for want in [
        "A|-4",
        "B|-4",
        "C|-5",
        "D|-6",
        "E|1 -3 252",
        "F|-4 252",
        "G|-4 -4 -4",
        "H|-4",
        "I|-4",
        "J|-4",
    ] {
        assert!(got.iter().any(|m| m == want), "missing {want:?} in {got:?}");
    }
}
