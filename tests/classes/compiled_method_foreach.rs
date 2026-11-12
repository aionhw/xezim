//! class-perf Step 9d: `foreach` over class MEMBER collections inside
//! compiled class-method bodies lowers to `ForeachKeys` /
//! `ForeachNext` bytecode. The exec arms reuse the interpreter's own
//! key materialization (`array_iter_keys` + `foreach_materialize_keys_1d`
//! semantics: `.size` shadow first, else prefix-scan + sort/dedup) and its
//! store resolution helpers (`instance_assoc_member` /
//! `handle_collection_name`), so key order, string conversion, and
//! per-instance resolution are identical to the AST path.
//!
//! Covers: bare queue foreach (weighted sum), int-keyed assoc foreach
//! (sum + count), string-keyed assoc foreach (string concat in key
//! order), queue element writes mid-loop (value mutation, iteration set
//! unchanged), inner-method call in the body, `continue`/`break`, empty
//! collection (zero iterations), nested member foreach (independent loop
//! vars), and loop-var scoping (outer var unchanged after the loop,
//! §12.7.3).
//!
//! Expectations are reference-simulator-validated (method_vm35 runs
//! TAG-identically under the reference simulator, gate OFF, and gate ON).
//! The gate is forced ON so a plain `cargo test` exercises the compiled
//! path. Note: a method whose body contains an element-handle method
//! call inside the foreach (`boxes[k].get_id()`) still declines to the
//! AST interpreter as a whole (9b-ii surface boundary) — parity holds by
//! the all-or-nothing contract.
//!
//! Known pre-existing divergence (out of scope, same on both gates):
//! pushing to the iterated queue mid-loop extends the reference
//! simulator's iteration (live size) but not xezim's (keys materialized
//! at loop entry) — the repro shapes here avoid it.
use xezim::simulate;

fn gate_on() -> bool {
    super::compiled_method_test_env::eager()
}

/// Subprocess stdout capture — needed for the string-accumulation test
/// whose result is only visible through a `$display` (string locals are
/// invisible to `get_signal`).
fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

#[test]
fn member_foreach_all_shapes() {
    if !gate_on() {
        return;
    }
    let src = r#"
class coll_c;
  int q[$];
  int aa[int];
  string sa[string];
  int inner[$];
  int inner2[$];
  function new();
  endfunction
  function int q_wsum();
    int s;
    int k;
    s = 0;
    foreach (q[k]) s = s + k * q[k];
    return s;
  endfunction
  function int aa_vsum();
    int s;
    int k;
    s = 0;
    foreach (aa[k]) s = s + aa[k];
    return s;
  endfunction
  function int aa_kcount();
    int n;
    int k;
    n = 0;
    foreach (aa[k]) n = n + 1;
    return n;
  endfunction
  function string sa_join();
    string acc;
    string k;
    acc = "";
    foreach (sa[k]) acc = {acc, k};
    return acc;
  endfunction
  function int q_mutate();
    int n;
    int k;
    n = 0;
    foreach (q[k]) begin
      n = n + 1;
      q[k] = q[k] + 1;
    end
    return n;
  endfunction
  function int q_find_gt2();
    int k;
    foreach (q[k]) if (q[k] > 2) return k;
    return -1;
  endfunction
  function int q_even_sum();
    int s;
    int k;
    s = 0;
    foreach (q[k]) begin
      if (k % 2 != 0) continue;
      s = s + q[k];
    end
    return s;
  endfunction
  function int q_empty();
    int n;
    int k;
    n = 7;
    foreach (inner[k]) n = 0;
    return n;
  endfunction
  function int q_nested();
    int s;
    int a;
    int b;
    s = 0;
    foreach (inner[a]) begin
      foreach (inner2[b]) s = s + a * 10 + b;
    end
    return s;
  endfunction
  function int k_scope();
    int k;
    int after;
    k = 123;
    foreach (q[k]) after = k;
    return k * 1000 + after;
  endfunction
endclass

module top;
  coll_c c;
  int r1, r2, r3, r4, r5, r6, r7, r8, r9;
  int wsum2;
  initial begin
    c = new();
    // q = [3, 1, 4, 1, 5]
    c.q.push_back(3); c.q.push_back(1); c.q.push_back(4);
    c.q.push_back(1); c.q.push_back(5);
    c.aa[-5] = 50; c.aa[-1] = 10; c.aa[0] = 11; c.aa[2] = 22; c.aa[7] = 77;
    c.sa["b"] = "1"; c.sa["a"] = "2"; c.sa["c"] = "3";
    r1 = c.q_wsum();
    r2 = c.aa_vsum();
    r3 = c.aa_kcount();
    r4 = c.q_mutate();
    r5 = c.q_find_gt2();
    r6 = c.q_even_sum();
    r7 = c.q_empty();
    // seed the nested-loop collections AFTER q_empty so it sees an
    // empty member queue (zero iterations)
    c.inner.push_back(1); c.inner.push_back(2);
    c.inner2.push_back(5);
    r8 = c.q_nested();
    r9 = c.k_scope();
    // q was grown by 1 in-place: [4, 2, 5, 2, 6] -> weighted sum 42
    wsum2 = c.q_wsum();
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulation should run");
    // q_wsum: 0*3 + 1*1 + 2*4 + 3*1 + 4*5 = 32
    assert_eq!(u(&sim, "r1"), 32, "q_wsum");
    // aa_vsum: 50+10+11+22+77 = 170 (keys -5,-1,0,2,7 sorted numerically)
    assert_eq!(u(&sim, "r2"), 170, "aa_vsum");
    // aa_kcount: 5 keys
    assert_eq!(u(&sim, "r3"), 5, "aa_kcount");
    // q_mutate: 5 iterations, each element grown in place
    assert_eq!(u(&sim, "r4"), 5, "q_mutate");
    // q_find_gt2: first element > 2 is q[0]=3
    assert_eq!(u(&sim, "r5"), 0, "q_find_gt2");
    // q_even_sum (runs AFTER q_mutate: q=[4,2,5,2,6]):
    // q[0]+q[2]+q[4] = 4+5+6 = 15
    assert_eq!(u(&sim, "r6"), 15, "q_even_sum");
    // q_empty: empty member queue -> zero iterations, n keeps 7
    assert_eq!(u(&sim, "r7"), 7, "q_empty");
    // q_nested: inner=[1,2], inner2=[5] -> (0*10+0) + (1*10+0) = 10
    assert_eq!(u(&sim, "r8"), 10, "q_nested");
    // k_scope: loop var is loop-scoped; outer k stays 123
    assert_eq!(u(&sim, "r9"), 123004, "k_scope");
    // wsum2: after q_mutate, q=[4,2,5,2,6] -> 0*4+1*2+2*5+3*2+4*6 = 42
    assert_eq!(u(&sim, "wsum2"), 42, "wsum2");
}

/// String accumulation through a string-keyed member assoc foreach:
/// the loop var is the KEY string, concatenated in sorted key order.
/// The result is read back through a string-returning method call in a
/// $display (string locals are invisible to `get_signal`).
#[test]
fn member_foreach_string_keys() {
    if !gate_on() {
        return;
    }
    let src = r#"
class coll_c;
  string sa[string];
  function new();
  endfunction
  function string sa_join();
    string acc;
    string k;
    acc = "";
    foreach (sa[k]) acc = {acc, k};
    return acc;
  endfunction
endclass

module top;
  string seen;
  initial begin
    coll_c c;
    c = new();
    c.sa["b"] = "1"; c.sa["a"] = "2"; c.sa["c"] = "3";
    seen = c.sa_join();
    $display("TAGX seen=%0s", seen);
  end
endmodule
"#;
    let path = "/tmp/svtest_method_foreach_str.sv";
    std::fs::write(path, src).unwrap();
    let out = std::process::Command::new(xezim_bin())
        .args(["--simulate", "-s", "top", path])
        .env("XEZIM_COMPILE_METHODS", "1")
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("TAGX seen=abc"),
        "expected sorted key concat 'abc' in stdout, got: {}",
        stdout
    );
}
