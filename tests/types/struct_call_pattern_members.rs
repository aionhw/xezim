//! Unpacked structs with dynamic members passed through calls and
//! assignment patterns.
//!
//! * IEEE 1800-2017 §10.9.2 / §13.4.1: `return '{...}` (and a function that
//!   falls off its end after `f = '{...}`) returns the members the pattern
//!   assigns. The packed return value cannot carry a dynamic member, and the
//!   pattern was never staged for the caller, so the caller copied the
//!   dynamic members a previous call had left behind (or none).
//! * §10.9.2 / §13.5.1: a pattern actual whose items are structs (variables
//!   or calls) binds the formal member by member, dynamic members included;
//!   it bound nothing at all.
//! * §13.5 / §7.2: a struct-returning call passed directly as an actual binds
//!   formal members of their declared signedness; the call's packed result
//!   is an unsigned bit pattern, so a negative `longint` member read back
//!   unsigned.
//! * §13.4.1 / §7.5.1: a method of a dynamic member of a call result
//!   (`f(x).d.size()`) sees the member the call returned; it saw an empty
//!   array.

use xezim::simulate;

fn msgs(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn expect(src: &str, want: &[&str]) {
    let sim = simulate(src, 20).expect("simulate failed");
    let got = msgs(&sim);
    for w in want {
        assert!(got.iter().any(|m| m == w), "missing {w:?} in {got:?}");
    }
}

#[test]
fn pattern_with_struct_members_returns_and_binds_dynamic_members() {
    let src = r#"
module tb;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } arr;
  typedef struct { arr f0; int f1; arr f2; } rec;
  function automatic arr mka(int n);
    arr a; byte unsigned t[]; t = new[n]; foreach (t[i]) t[i] = 1;
    a.l = 1; a.r = n; a.dn = 0; a.d = t; return a;
  endfunction
  function automatic rec mk(int n);
    rec x; x.f0 = mka(n); x.f1 = n; x.f2 = mka(2); return x;
  endfunction
  function automatic rec mk2(int n);
    return '{mka(n), n, mka(2)};
  endfunction
  function automatic rec mk3(int n);
    mk3 = '{f0: mka(n), f1: n, f2: mka(n + 1)};
  endfunction
  function automatic void take(rec v);
    $display("T|%0d %0d %0d %0d", v.f0.d.size(), v.f1, v.f2.d.size(), v.f0.r);
  endfunction
  initial begin
    rec c, d, e; arr a; rec q[$];
    c = mk(3);
    $display("C|%0d %0d %0d", c.f0.d.size(), c.f1, c.f2.d.size());
    d = mk2(4);
    $display("D|%0d %0d %0d %0d", d.f0.d.size(), d.f1, d.f2.d.size(), d.f0.r);
    d = mk3(5);
    $display("E|%0d %0d %0d", d.f0.d.size(), d.f1, d.f2.d.size());
    a = mka(6);
    take('{a, 6, mka(1)});
    take('{mka(7), 7, a});
    e = '{mka(8), 8, a};
    q.push_back('{mka(9), 9, mka(2)});
    q.push_back(e);
    $display("Q|%0d %0d %0d|%0d %0d %0d", q[0].f0.d.size(), q[0].f1, q[0].f2.d.size(),
             q[1].f0.d.size(), q[1].f1, q[1].f2.d.size());
  end
endmodule
"#;
    expect(
        src,
        &[
            "C|3 3 2",
            "D|4 4 2 4",
            "E|5 5 6",
            "T|6 6 1 6",
            "T|7 7 6 7",
            "Q|9 9 2|8 8 6",
        ],
    );
}

#[test]
fn call_actual_binds_members_with_declared_signedness() {
    let src = r#"
module tb;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } arr;
  function automatic arr mk(longint l, longint r, bit dn, byte unsigned t[]);
    arr a; a.l = l; a.r = r; a.dn = dn; a.d = t; return a;
  endfunction
  function automatic arr make(int n);
    byte unsigned t[]; t = new[n]; return mk(5, 6 - n, 1, t);
  endfunction
  function automatic string show(input arr p);
    return $sformatf("r=%0d lt0=%0d n=%0d", p.r, p.r < 0, p.d.size());
  endfunction
  initial begin
    arr x;
    x = make(8);
    $display("V|%s", show(x));
    $display("C|%s", show(make(8)));
  end
endmodule
"#;
    expect(src, &["V|r=-2 lt0=1 n=8", "C|r=-2 lt0=1 n=8"]);
}

#[test]
fn dynamic_member_of_call_result() {
    let src = r#"
module tb;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } arr;
  function automatic arr id2(input arr a);
    if (a.l == 12345) $display("never");
    return a;
  endfunction
  function automatic arr mk(int n);
    arr a; a.l = 3; a.r = -4; a.dn = 1; a.d = new[n]; return a;
  endfunction
  function automatic int use2(input arr p);
    return id2(p).d.size();
  endfunction
  initial begin
    arr x;
    x = mk(8);
    x.d[1] = 7;
    $display("A|%0d %0d %0d", id2(x).d.size(), use2(x), id2(x).r);
    $display("B|%0d %0d", mk(5).d.size(), id2(x).d[1]);
  end
endmodule
"#;
    expect(src, &["A|8 8 -4", "B|5 7"]);
}
