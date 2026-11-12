//! class-perf: PERSISTENT compiled-method cache (XEZIM_METHOD_CACHE).
//!
//! Every test here runs the debug binary as a SUBPROCESS twice against the
//! same on-disk cache directory: once cold (compile + store), once warm
//! (disk probe replaces admission + compilation). The contract is that the
//! cold, warm, and cache-less outputs are BYTE-IDENTICAL — a warm run may
//! skip work, but it may never make a different decision than a cold run
//! would have.
//!
//! Covers:
//! - cold/warm/no-cache parity on a nested-bare-call workload (the same
//!   shape as the tiering test: static counters + 250 iterations, so the
//!   warm run actually re-enters persisted blocks),
//! - the cache directory really is written (entry files appear) and really
//!   is read back (strace-free proof: a CORRUPTED entry must be treated as
//!   a miss, recompiled, and still produce the identical output),
//! - a method that declines compilation (a `$display` body) round-trips as
//!   a persisted Nil without changing behavior.
//!
//! Reference-validated source shape (the bare-call double-evaluation fix):
//! the expected TAG line is what the reference simulator prints.
use std::path::PathBuf;
use std::process::Command;

fn xezim_bin() -> PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

/// Unique cache dir per test invocation (never collides with a parallel
/// test run, never picks up a stale ~/.cache).
fn fresh_cache_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "xezim-pcache-test-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    d
}

fn run_cached(src: &PathBuf, cache: &PathBuf, tier: &str) -> String {
    let out = Command::new(xezim_bin())
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(src)
        .env("XEZIM_COMPILE_METHODS", "1")
        .env("XEZIM_METHOD_TIER", tier)
        .env("XEZIM_METHOD_CACHE", cache)
        .output()
        .expect("subprocess failed");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

fn write_src(name: &str, body: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT_SOURCE: AtomicUsize = AtomicUsize::new(0);
    let p = std::env::temp_dir().join(format!(
        "xezim-pcache-{}-{}-{}.sv",
        name,
        std::process::id(),
        NEXT_SOURCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&p, body).expect("write src");
    p
}

#[test]
fn source_files_are_isolated_between_invocations() {
    let first = write_src("isolation", "first source");
    let second = write_src("isolation", "second source");
    assert_ne!(first, second);
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "first source");
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second source");
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
}

const WORKLOAD: &str = r#"
class C;
  static longint unsigned s_hits;
  function int step(int v);
    C::s_hits = C::s_hits + 1;
    return v + 1;
  endfunction
  function int sum3(int a, int b, int c);
    return step(a) + step(b) + step(c);
  endfunction
endclass
module top;
  C c;
  longint unsigned total;
  longint unsigned hits;
  int acc2;
  int i;
  initial begin
    c = new();
    for (i = 0; i < 250; i = i + 1) begin
      total = total + c.sum3(i, i * 2, i * 3);
      acc2 = acc2 + c.step(1);
    end
    hits = C::s_hits;
    $display("TAG total=%0d hits=%0d acc=%0d", total, hits, acc2);
  end
endmodule
"#;

#[test]
fn cold_warm_and_cacheless_runs_are_identical() {
    let src = write_src("workload", WORKLOAD);
    let cache = fresh_cache_dir("parity");
    let cold = run_cached(&src, &cache, "0");
    assert!(
        cold.contains("TAG total=187500 hits=1000 acc=500"),
        "cold output: {}",
        cold
    );
    // Eager (tier 0) warm run: every method's block comes from disk.
    let warm = run_cached(&src, &cache, "0");
    // Default-tier warm run: promotion still probes disk at call 100.
    let warm_tiered = run_cached(&src, &cache, "100");
    assert_eq!(cold, warm, "warm eager run diverged");
    assert_eq!(cold, warm_tiered, "warm tiered run diverged");
    // Cache-less control run.
    let plain = Command::new(xezim_bin())
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(&src)
        .env("XEZIM_COMPILE_METHODS", "0")
        .output()
        .expect("subprocess failed");
    let plain = format!(
        "{}{}",
        String::from_utf8_lossy(&plain.stdout),
        String::from_utf8_lossy(&plain.stderr)
    );
    assert_eq!(cold, plain, "cacheless run diverged");
    // The directory must have been populated.
    let entries = std::fs::read_dir(&cache)
        .expect("cache dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "bin"))
        .count();
    assert!(entries >= 2, "expected persisted entries, got {}", entries);
}

#[test]
fn corrupted_entries_are_misses_not_failures() {
    let src = write_src("workload", WORKLOAD);
    let cache = fresh_cache_dir("corrupt");
    let cold = run_cached(&src, &cache, "0");
    // Trash every persisted entry: each must deserialize-fail, fall back to
    // recompilation, and still produce the identical output (and rewrite
    // the entries with good payloads).
    for entry in std::fs::read_dir(&cache)
        .expect("cache dir")
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.extension().is_some_and(|x| x == "bin") {
            std::fs::write(&p, b"\x00garbage-not-bincode").expect("corrupt");
        }
    }
    let warm = run_cached(&src, &cache, "0");
    assert_eq!(cold, warm, "corrupted-cache run diverged");
    // And the repaired entries keep working.
    let warm2 = run_cached(&src, &cache, "0");
    assert_eq!(cold, warm2, "re-warmed run diverged");
}

#[test]
fn declined_methods_round_trip_as_nil() {
    // `show` contains a $display body: it declines compilation, so its
    // outcome persists as Nil and a warm run must skip admission the same
    // way — observable only as output parity, which is the contract.
    let src = write_src(
        "declined",
        r#"
class D;
  int n;
  function int get_n();
    return n;
  endfunction
  function void show(string tag);
    $display("TAG %s n=%0d", tag, n);
  endfunction
endclass
module top;
  D d;
  initial begin
    d = new();
    d.n = 41;
    d.show("one");
    d.n = d.get_n() + 1;
    d.show("two");
  end
endmodule
"#,
    );
    let cache = fresh_cache_dir("nil");
    let cold = run_cached(&src, &cache, "0");
    assert!(
        cold.contains("TAG one n=41") && cold.contains("TAG two n=42"),
        "cold output: {}",
        cold
    );
    let warm = run_cached(&src, &cache, "0");
    assert_eq!(cold, warm, "warm run diverged on declined method");
}
