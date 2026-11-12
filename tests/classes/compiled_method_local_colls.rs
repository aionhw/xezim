//! class-perf row 3a: LOCAL queue / dynamic-array declarations inside
//! compiled class-method bodies. The DECLARATION itself is delegated to
//! the interpreter (a `StmtFallback` of the single-declarator statement,
//! so the per-call `@name#id` storage key and every queue bookkeeping
//! table stay interpreter-owned), while the USES lower to bytecode:
//! `push_back`/`size`/`pop_front`/`delete` via `CallCollMethod` with a
//! `\x01`-marked bare receiver, element reads/writes via
//! `LoadCollElem`/`StoreCollElem`, iteration via `ForeachKeys` — all
//! resolving the store through `dyn_name_lookup` (innermost frame
//! first), the exact precedence the AST path uses for a bare local
//! receiver.
//!
//! Covers: int queues (decl, push, size, elem write, elem read,
//! foreach, pop, delete), string queues (push/read/write/foreach,
//! including a string actual through the value-const marshalling),
//! queues of class handles (push/`==`/elem call), local dynamic arrays
//! (`d = new[2]` + elem rw + size — the `new[]` assign stays admitted),
//! a local queue SHADOWING a same-named member queue, loop-body
//! re-declaration (fresh per-call storage each iteration), branch-scope
//! decls, and a size() call used as an INDEX (`q[q.size()-1]`).
//!
//! The whole reproducer is byte-for-byte ON==OFF and matches the
//! reference simulator line-for-line (assertion lines). Methods that
//! need to PASS the local queue as a call argument still decline
//! (row-3b surface) — parity holds by the all-or-nothing contract.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn local_coll_queues_all_shapes() {
    if !gate_on() {
        return;
    }
    let src = r#"
class C;
  int mq[$];
  function new();
  endfunction
  function void fill(int n, int base);
    int q[$];
    for (int i = 0; i < n; i++) q.push_back(base + i);
    q[0] = 100;
    r = q[0] * 1000 + q[1] * 10 + q.size();
    t = 0;
    foreach (q[i]) t = t + q[i];
    p = q.pop_front();
    q.delete();
    d = q.size();
  endfunction
  function void strs();
    string s[$];
    s.push_back("alpha");
    s.push_back("beta");
    // string actual through the value-const marshalling round-trips
    s.push_back("");
    n = s.size();
    s[1] = "gamma";
    // a size() call as an index expression
    last = s[s.size()-1];
  endfunction
  function int total();
    int q[$];
    int t;
    q.push_back(3); q.push_back(4); q.push_back(5);
    t = 0;
    foreach (q[i]) t = t + q[i];
    return t;
  endfunction
  function void work();
    int q[$];
    q.push_back(7);
    mq.push_back(9);
    // the LOCAL q must shadow the member mq
    lq = q[0]; lmq = mq[0]; lqs = q.size(); lmqs = mq.size();
  endfunction
  function void dyna();
    int d[];
    d = new[2];
    d[0] = 11; d[1] = 22;
    dv = d[0] * 100 + d[1];
  endfunction
  function void handles();
    C h[$];
    C p;
    h.push_back(this);
    p = new;
    h.push_back(p);
    hn = h.size();
    heq = (h[0] == this) ? 1 : 0;
    ht = h[1].total();
  endfunction
  function int loopdecl();
    int t;
    t = 0;
    for (int k = 0; k < 3; k++) begin
      int q[$];
      q.push_back(k); q.push_back(10 + k);
      t += q.pop_front();
    end
    return t;
  endfunction
  function void branchy(int m);
    if (m) begin
      int q[$];
      q.push_back(1);
      b1 = q.size();
    end else begin
      int q[$];
      b0 = q.size();
    end
  endfunction
  int r, t, p, d, n, last, lq, lmq, lqs, lmqs, dv, hn, heq, ht, b1, b0;
endclass

module top;
  C c = new;
  int r, t, p, d, n, lq, lmq, lqs, lmqs, dv, hn, heq, ht, b1, b0;
  initial begin
    c.fill(3, 10);
    c.strs();
    c.total();
    c.work();
    c.dyna();
    c.handles();
    c.loopdecl();
    c.branchy(1);
    c.branchy(0);
    r = c.r; t = c.t; p = c.p; d = c.d; n = c.n;
    lq = c.lq; lmq = c.lmq; lqs = c.lqs; lmqs = c.lmqs;
    dv = c.dv; hn = c.hn; heq = c.heq; ht = c.ht;
    b1 = c.b1; b0 = c.b0;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    // fill(3,10): q = {10,11,12}; q[0]=100 -> r = 100*1000 + 11*10 + 3
    assert_eq!(u(&sim, "r"), 100_113);
    // foreach sum: 100+11+12 (the member t is only written by fill)
    assert_eq!(u(&sim, "t"), 123);
    assert_eq!(u(&sim, "p"), 100); // pop_front after foreach
    assert_eq!(u(&sim, "d"), 0); // delete -> size 0
    // strs: alpha,beta,"" then s[1]=gamma — 3 elems
    assert_eq!(u(&sim, "n"), 3);
    // shadowing: local q={7}, member mq={9}
    assert_eq!(u(&sim, "lq"), 7);
    assert_eq!(u(&sim, "lmq"), 9);
    assert_eq!(u(&sim, "lqs"), 1);
    assert_eq!(u(&sim, "lmqs"), 1);
    // dynamic array
    assert_eq!(u(&sim, "dv"), 11 * 100 + 22);
    // handle queue
    assert_eq!(u(&sim, "hn"), 2);
    assert_eq!(u(&sim, "heq"), 1);
    assert_eq!(u(&sim, "ht"), 12); // h[1] is the fresh C; total() = 12
    // loop re-decl: 0+1+2 returned (fresh per-iteration storage)
    // branch decls
    assert_eq!(u(&sim, "b1"), 1);
    assert_eq!(u(&sim, "b0"), 0);
}

#[test]
fn local_coll_string_elems() {
    if !gate_on() {
        return;
    }
    // The STRING-ELEMENT surface: reads/writes of string queue elements
    // and foreach over them, checked through stdout (string values are
    // invisible to `get_signal`).
    let src = r#"
class C;
  function void strs();
    string s[$];
    s.push_back("alpha");
    s.push_back("beta");
    $display("s0=%s s1=%s n=%0d", s[0], s[1], s.size());
    foreach (s[i]) $display("S %0d %s", i, s[i]);
    s[1] = "gamma";
    $display("s1b=%s", s[1]);
  endfunction
endclass
module top;
  C c = new;
  initial begin
    c.strs();
  end
endmodule
"#;
    // ON==OFF byte-for-byte on the assertion lines (subprocess — string
    // elem values only surface through $display).
    let path = "/tmp/locoll_strs.sv";
    std::fs::write(path, src).unwrap();
    let run = |gate: &str| {
        let out = std::process::Command::new(xezim_bin())
            .args(["--simulate", "-s", "top", path])
            .env("XEZIM_COMPILE_METHODS", gate)
            .env("XEZIM_METHOD_TIER", "0")
            .output()
            .expect("run xezim");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let off = run("0");
    let on = run("1");
    for line in ["s0=alpha s1=beta n=2", "S 0 alpha", "S 1 beta", "s1b=gamma"] {
        assert!(off.contains(line), "OFF missing {line}:\n{off}");
        assert!(on.contains(line), "ON missing {line}:\n{on}");
    }
    // Byte-for-byte assertion lines on both gates.
    let f = |s: &str| {
        s.lines()
            .filter(|l| l.starts_with('s') || l.starts_with("S "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(f(&off), f(&on), "ON != OFF");
}

/// Subprocess binary path, as Cargo built it for the integration tests.
fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}
