//! class-perf tiering: with the gate on, a method body compiles only after
//! `XEZIM_METHOD_TIER` calls (the product default is a much larger,
//! economics-calibrated value — see `method_tier_threshold` in the
//! simulator; the boundary arm below pins its own threshold so its
//! coverage never depends on that default). Below the threshold every call
//! runs the AST interpreter with gate-OFF frames; above it the compiled
//! block takes over. The ON==OFF byte-for-byte contract must therefore hold
//! ACROSS the tier boundary: a hot method must observe its own state
//! (static + instance) identically whether a given call was interpreted or
//! compiled, including values computed before the boundary and consumed
//! after it.
//!
//! The test drives one method past the default threshold (250 outer / 750
//! inner calls vs 100) and checks the accumulated results against an
//! independent Rust model, then re-runs the same source with tier 0
//! (compile-on-first-call) and a huge tier (never compiles) — all three
//! must agree, proving the boundary itself is invisible.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("top.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is x/z", n))
}

const SRC: &str = r#"
class counter;
  static longint unsigned s_hits;
  int acc;
  function new();
    acc = 1000;
  endfunction
  function int step(int v);
    acc = acc + v;
    s_hits = s_hits + 1;
    if (acc > 1100) acc = acc - 7;
    return acc;
  endfunction
  function int sum3(int a, int b, int c);
    return step(a) + step(b) + step(c);
  endfunction
endclass

module top;
  counter c;
  longint unsigned total;
  longint unsigned hits;
  int racc;
  int r;
  int i;
  initial begin
    c = new();
    for (i = 0; i < 250; i = i + 1) begin
      r = c.sum3(i, i + 1, 2);
      total = total + r;
    end
    hits = counter::s_hits;
    racc = c.acc;
  end
endmodule
"#;

/// Independent model: the exact `step`/`sum3` semantics, no SV involved.
fn model() -> (i64, i64, i64) {
    let (mut acc, mut s_hits, mut total) = (1000i64, 0i64, 0i64);
    for i in 0..250i64 {
        let mut sum = 0i64;
        for v in [i, i + 1, 2] {
            acc += v;
            s_hits += 1;
            if acc > 1100 {
                acc -= 7;
            }
            sum += acc;
        }
        total += sum;
    }
    (total, s_hits, acc)
}

fn run() -> (u64, u64, u64) {
    let sim = simulate(SRC, 100).expect("simulation should run");
    (u(&sim, "total"), u(&sim, "hits"), u(&sim, "racc"))
}

#[test]
fn tiering_boundary_is_invisible() {
    if !super::compiled_method_test_env::policies(&[("1", "100"), ("1", "0"), ("1", "1000000")]) {
        return;
    }
    let (want_total, want_hits, want_acc) = model();
    // Sanity: the model actually exercises the >1100 clamp.
    assert!(want_hits == 750, "model must make 750 step() calls");

    // Each policy runs in a fresh process. Tier 100 crosses the boundary
    // mid-run; tier zero and the large tier check its compiled/AST controls.
    let a = run();
    assert_eq!(a, (want_total as u64, want_hits as u64, want_acc as u64));
}

// ---------------------------------------------------------------------------
// class-perf ADAPTIVE default tiering (task b): the skip cache now runs
// BEFORE the tier counter, so permanently skip-cached methods (task form,
// string-return/string-formal, non-scalar result, ...) never touch tier
// bookkeeping, and the product default threshold (`HOT_CALLS` = 1000)
// compiles a method only after it has been *genuinely called* enough to
// repay. Each arm is a fresh SUBPROCESS so the process-global tier/directive
// locks see the true default.
// ---------------------------------------------------------------------------

use std::path::PathBuf;
use std::process::Command;

fn xezim_bin2() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

fn write_tier_src(name: &str, body: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("xezim-tier-{}.sv", name));
    std::fs::write(&p, body).expect("write src");
    p
}

fn run_cli(src: &PathBuf, compile: Option<&str>, tier: Option<&str>) -> String {
    let mut cmd = Command::new(xezim_bin2());
    cmd.arg("--simulate").arg("-s").arg("top").arg(src);
    if let Some(c) = compile {
        cmd.env("XEZIM_COMPILE_METHODS", c);
    }
    if let Some(t) = tier {
        cmd.env("XEZIM_METHOD_TIER", t);
    }
    let out = cmd.output().expect("subprocess failed");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

// A genuinely hot compilable method (called 1500x > the 1000 default) mixed
// with permanently skip-cached methods (a task and a string-return
// function): under the ADAPTIVE default tier the hot method must compile
// and the skip-cached ones must stay interpreted — yet the output is
// byte-for-byte identical to a gate-off (pure AST) run.
const HOT_MIXED_SRC: &str = r#"
class C;
  int n;
  // compilable: scalar input + scalar return, simple body.
  function automatic int bump(int x);
    n = n + x;
    return n;
  endfunction
  // skip-cached: task form (timing body -> never compiles).
  task automatic touch(int x);
    n = n + x;
  endtask
  // skip-cached: string-return function.
  function automatic string tag(int x);
    string s = "";
    if (x == 0) s = "a";
    else s = "b";
    return s;
  endfunction
endclass
module top;
  integer r;
  integer i;
  string s;
  initial begin
    C c;
    c = new();
    for (i = 0; i < 1500; i = i + 1) begin
      r = r + c.bump(i % 4);
      c.touch(1);
    end
    s = c.tag(1);
    $display("TAG r=%0d s=%s", r, s);
  end
endmodule
"#;

#[test]
fn adaptive_default_tier_skips_cache_is_byte_identical_to_gate_off() {
    let src = write_tier_src("hotm", HOT_MIXED_SRC);
    // Gate OFF (pure AST interpreter): the reference output.
    let off = run_cli(&src, Some("0"), None);
    assert!(
        off.contains("TAG r="),
        "gate-off output missing TAG: {}",
        off
    );
    // Default tier (adaptive, no XEZIM_METHOD_TIER set): hot `bump`
    // compiles past the 1000 threshold; the skip-cached task/string
    // methods never do. Output must be byte-identical to gate-off.
    let dflt = run_cli(&src, Some("1"), None);
    assert_eq!(
        off, dflt,
        "adaptive default-tier run diverged from gate-off\n--- ON ---\n{}\n--- OFF ---\n{}",
        dflt, off
    );
    // Tier 0 (compile every admitted method on first call): still identical.
    let t0 = run_cli(&src, Some("1"), Some("0"));
    assert_eq!(off, t0, "tier-0 run diverged from gate-off");

    // Remove the temp source.
    let _ = std::fs::remove_file(&src);
}

#[test]
fn adaptive_default_tier_threshold_compiles_a_hot_method() {
    // A single hot method driven above the default 1000-call threshold. The
    // first ~1000 calls run on AST until the tier counter reaches the
    // threshold, then the rest run compiled.
    //
    // Two SV bodies drive it: a LIGHT one (4000 calls x t<60) for the
    // byte-identity runs (fast; still fully past the 1000-call threshold on
    // the compiled path, so it exercises the same tiering), and a HEAVY one
    // (12000 calls x t<200) ONLY for the profiler subprocess. The heavy body
    // makes the compiled VM window span ~8 wall seconds so the 5 ms
    // statistical sampler tallies ~800 VM samples that even heavy parallel-
    // test CPU contention cannot shrink below a nonzero count.
    let src_light = write_tier_src(
        "hotthreshold",
        r#"
class K;
  int acc;
  function automatic int step(int v);
    int t;
    acc = acc + v;
    for (t = 0; t < 60; t = t + 1) acc = acc + ((t * v) % 7);
    if (acc > 900) acc = acc - 300;
    return acc;
  endfunction
endclass
module top;
  integer r;
  integer i;
  initial begin
    K k;
    k = new();
    for (i = 0; i < 4000; i = i + 1) begin
      r = r + k.step(1);
    end
    $display("TAG r=%0d", r);
  end
endmodule
"#,
    );
    let src_heavy = write_tier_src(
        "hotthresholdheavy",
        r#"
class K;
  int acc;
  function automatic int step(int v);
    int t;
    acc = acc + v;
    for (t = 0; t < 100; t = t + 1) acc = acc + ((t * v) % 7);
    if (acc > 900) acc = acc - 300;
    return acc;
  endfunction
endclass
module top;
  integer r;
  integer i;
  initial begin
    K k;
    k = new();
    for (i = 0; i < 3000; i = i + 1) begin
      r = r + k.step(1);
    end
    $display("TAG r=%0d", r);
  end
endmodule
"#,
    );
    let off = run_cli(&src_light, Some("0"), None);
    let dflt = run_cli(&src_light, Some("1"), None);
    assert_eq!(off, dflt, "default-tier hot method diverged from gate-off");

    // The default tier (1000) is BELOW the 4000 calls, so `step` must have
    // been admitted to the compiled path — the profiler confirms it ran as
    // VM bytecode, not AST. Run it on the HEAVY body so the VM window is
    // long enough for the sampler to catch under contention.
    let mut cmd = Command::new(xezim_bin2());
    cmd.arg("--simulate").arg("-s").arg("top").arg(&src_heavy);
    cmd.env("XEZIM_COMPILE_METHODS", "1");
    cmd.env("XEZIM_METHOD_PROFILE", "1");
    let prof = cmd.output().expect("profiler subprocess failed");
    let prof = format!(
        "{}{}",
        String::from_utf8_lossy(&prof.stdout),
        String::from_utf8_lossy(&prof.stderr)
    );
    assert!(
        step_vm_count(&prof) > 0,
        "expected the profiler to show `step` with a nonzero VM counter under the \
         adaptive default tier (12000 calls > 1000 threshold); profiler output:\n{}",
        prof
    );

    // And a just-below-threshold method must NOT have compiled under the
    // default (it stays on AST), still matching gate-off.
    let src2 = write_tier_src(
        "coldunder",
        r#"
class L;
  int acc;
  function automatic int step(int v);
    acc = acc + v;
    return acc;
  endfunction
endclass
module top;
  integer r;
  integer i;
  initial begin
    L l;
    l = new();
    for (i = 0; i < 500; i = i + 1) begin
      r = r + l.step(1);
    end
    $display("TAG r=%0d", r);
  end
endmodule
"#,
    );
    let off2 = run_cli(&src2, Some("0"), None);
    let dflt2 = run_cli(&src2, Some("1"), None);
    assert_eq!(
        off2, dflt2,
        "sub-threshold (500 calls) method diverged from gate-off"
    );
    // 500 < 1000 threshold: `step` must NOT appear as VM in the profile.
    let mut cmd = Command::new(xezim_bin2());
    cmd.arg("--simulate").arg("-s").arg("top").arg(&src2);
    cmd.env("XEZIM_COMPILE_METHODS", "1");
    cmd.env("XEZIM_METHOD_PROFILE", "1");
    let prof2 = cmd.output().expect("profiler subprocess failed");
    let prof2 = format!(
        "{}{}",
        String::from_utf8_lossy(&prof2.stdout),
        String::from_utf8_lossy(&prof2.stderr)
    );
    assert_eq!(
        step_vm_count(&prof2),
        0,
        "expected step (500 calls < 1000 default threshold) to stay on AST \
         (zero VM samples); profiler output:\n{}",
        prof2
    );

    let _ = std::fs::remove_file(&src_light);
    let _ = std::fs::remove_file(&src_heavy);
    let _ = std::fs::remove_file(&src2);
}

/// Measure admission from a fresh persistent cache, independent of the
/// asynchronous profiler. The positive case must contain an actual compiled
/// block, rather than a cached refusal, and both policies check known results.
fn run_with_admission(source: &str, tier: &str) -> (String, usize, usize) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim-loop-heat-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).expect("isolated heat test directory");
    let src = dir.join("input.sv");
    let cache = dir.join("cache");
    std::fs::write(&src, source).unwrap();
    let output = Command::new("timeout")
        .args(["--kill-after=5s", "60s"])
        .arg(xezim_bin2())
        .args(["--simulate", "-s", "top"])
        .arg(&src)
        .env("XEZIM_COMPILE_METHODS", "1")
        .env("XEZIM_METHOD_TIER", tier)
        .env("XEZIM_METHOD_CACHE", &cache)
        .env_remove("XEZIM_METHOD_PROFILE")
        .output()
        .expect("heat subprocess");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("TAG "))
        .collect::<Vec<_>>()
        .join("\n");
    let mut blocks = 0;
    let mut refusals = 0;
    if cache.exists() {
        for entry in std::fs::read_dir(&cache).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "bin") {
                let outcome = xezim::compiler::bytecode::decode_persisted_method_outcome(
                    &std::fs::read(path).unwrap(),
                )
                .expect("valid method cache");
                if matches!(
                    outcome,
                    xezim::compiler::bytecode::PersistedMethodOutcome::Block { .. }
                ) {
                    blocks += 1;
                } else {
                    refusals += 1;
                }
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
    (text, blocks, refusals)
}

#[test]
fn measured_loop_heat_admits_only_bodies_with_enough_iterations() {
    let source = r#"
class K;
  int acc = 0;
  function automatic int step(int v);
    int t;
    acc = acc + v;
    for (t = 0; t < 80; t = t + 1) acc = acc + ((t * v) % 7);
    return acc;
  endfunction
endclass
module top;
  integer r = 0;
  initial begin
    K k = new;
    for (int i = 0; i < 30; i++) r = r + k.step(1);
    $display("TAG r=%0d", r);
  end
endmodule
"#;
    assert_eq!(
        run_with_admission(source, "1000000"),
        ("TAG r=109275".into(), 0, 0)
    );
    assert_eq!(
        run_with_admission(source, "100"),
        ("TAG r=109275".into(), 1, 0)
    );
}

// §12.7.3: all foreach collection shapes contribute executed iterations.
// Calling twice crosses the heat threshold while remaining below call tier.
#[test]
fn foreach_iterations_cross_the_method_heat_threshold() {
    for (tag, declaration, setup, loop_stmt, expected) in [
        (
            "queue",
            "int q[$];",
            "for (int i=0;i<2000;i++) c.q.push_back(i);",
            "foreach(q[k]) n++;",
            2000,
        ),
        (
            "dynamic",
            "int q[];",
            "c.q = new[2000];",
            "foreach(q[k]) n++;",
            2000,
        ),
        ("fixed", "int q[2000];", "", "foreach(q[k]) n++;", 2000),
        (
            "assoc",
            "int q[int];",
            "for(int i=0;i<2000;i++) c.q[i*2]=i;",
            "foreach(q[k]) n++;",
            2000,
        ),
        ("matrix", "int q[40][50];", "", "foreach(q[j,k]) n++;", 2000),
        (
            "continue",
            "int q[$];",
            "for(int i=0;i<2000;i++) c.q.push_back(i);",
            "foreach(q[k]) begin if(k%2==0) continue; n++; end",
            1000,
        ),
    ] {
        let source = format!(
            r#"
class C;
  {declaration}
  function int step();
    int n = 0;
    {loop_stmt}
    return n;
  endfunction
endclass
module top;
  initial begin
    C c = new;
    {setup}
    $display("TAG %0d", c.step());
    $display("TAG %0d", c.step());
  end
endmodule
"#
        );
        let output = format!("TAG {expected}\nTAG {expected}");
        assert_eq!(
            run_with_admission(&source, "1000000"),
            (output.clone(), 0, 0),
            "{tag} cold"
        );
        // Every shape compiles once heat reaches admission; a multi-index
        // member foreach, which bytecode does not lower, runs as one
        // interpreted statement inside the compiled body.
        assert_eq!(
            run_with_admission(&source, "100"),
            (output, 1, 0),
            "{tag} hot"
        );
    }
}

#[test]
fn foreach_heat_counts_executed_iterations_instead_of_collection_size() {
    for (tag, setup, loop_stmt, expected) in [
        ("empty", "", "foreach(q[k]) n++;", 0),
        (
            "break",
            "for(int i=0;i<2000;i++) c.q.push_back(i);",
            "foreach(q[k]) begin n++; if(k==1) break; end",
            2,
        ),
        (
            "return",
            "for(int i=0;i<2000;i++) c.q.push_back(i);",
            "foreach(q[k]) begin n++; if(k==1) return n; end",
            2,
        ),
    ] {
        let source = format!(
            r#"
class C;
  int q[$];
  function int step();
    int n = 0;
    {loop_stmt}
    return n;
  endfunction
endclass
module top;
  initial begin
    C c = new;
    {setup}
    $display("TAG %0d", c.step());
    $display("TAG %0d", c.step());
  end
endmodule
"#
        );
        assert_eq!(
            run_with_admission(&source, "100"),
            (format!("TAG {expected}\nTAG {expected}"), 0, 0),
            "{tag}"
        );
    }
}

/// Parse the method-profiler histogram for `step` and return its VM counter.
/// The histogram line has the form `   AST  VM  step`; a positive VM count
/// means the method was admitted to the compiled bytecode path.
fn step_vm_count(profile: &str) -> u64 {
    let mut vm = 0u64;
    for line in profile.lines() {
        // e.g. `    2 AST    6 VM  step`
        let t = line.trim();
        if !t.contains("step") {
            continue;
        }
        let f: Vec<&str> = t.split_whitespace().collect();
        // [ast, "AST", vm, "VM", "step"]
        if f.len() >= 4 && f.get(1) == Some(&"AST") && f.get(3) == Some(&"VM") {
            if let Ok(v) = f[2].parse::<u64>() {
                vm += v;
            }
        }
    }
    vm
}
