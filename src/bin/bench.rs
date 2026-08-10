//! `xezim-bench` — host-CPU benchmark harness.
//!
//! Runs the self-checking workloads from `xezim::benchw` and reports
//! per-workload simulation rates TAGGED with the host CPU (model, arch,
//! thread count, SIMD features), so numbers collected on different machine
//! types are directly comparable. Ends with one machine-readable JSON line
//! (`BENCH_JSON {...}`) for scripted collection across hosts.
//!
//! Usage:
//!   xezim-bench [--cycles N] [--repeats K] [--filter substr] [--json-only]
//!
//! Timing method: the public API compiles and runs in one call, so each
//! workload is timed twice — a 1-cycle run (compile + startup) and the full
//! run — and the steady-state rate is derived from the difference. Every run
//! re-checks the design's accumulators against a Rust mirror, so a wrong
//! result aborts instead of reporting a meaningless rate.

use std::time::Instant;
use xezim::benchw;

struct HostInfo {
    arch: &'static str,
    model: String,
    threads: usize,
    features: Vec<&'static str>,
}

fn host_info() -> HostInfo {
    let model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name") || l.starts_with("Hardware"))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let mut features: Vec<&'static str> = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse4.2") {
            features.push("sse4.2");
        }
        if std::arch::is_x86_feature_detected!("popcnt") {
            features.push("popcnt");
        }
        if std::arch::is_x86_feature_detected!("bmi2") {
            features.push("bmi2");
        }
        if std::arch::is_x86_feature_detected!("avx2") {
            features.push("avx2");
        }
        if std::arch::is_x86_feature_detected!("avx512f") {
            features.push("avx512f");
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON is baseline on aarch64.
        features.push("neon");
        if std::arch::is_aarch64_feature_detected!("sve") {
            features.push("sve");
        }
    }
    HostInfo {
        arch: std::env::consts::ARCH,
        model,
        threads,
        features,
    }
}

struct BenchResult {
    name: &'static str,
    stresses: &'static str,
    cycles: u64,
    compile_ms: f64,
    run_ms: f64,
    cycles_per_sec: f64,
}

fn run_workload(w: &benchw::Workload, cycles: u64, repeats: usize) -> BenchResult {
    // Compile + startup cost: a 1-cycle run.
    let tiny_src = (w.source)(1);
    let t0 = Instant::now();
    let sim = xezim::simulate(&tiny_src, (w.sim_time)(1)).expect("bench compile failed");
    let compile_ms = t0.elapsed().as_secs_f64() * 1e3;
    drop(sim);

    // Full runs: keep the BEST (min) wall time of `repeats` runs — the
    // usual defense against scheduler noise on a shared host.
    let src = (w.source)(cycles);
    let sim_time = (w.sim_time)(cycles);
    let mut best_total_ms = f64::INFINITY;
    for _ in 0..repeats {
        let t = Instant::now();
        let sim = xezim::simulate(&src, sim_time).expect("bench run failed");
        let total_ms = t.elapsed().as_secs_f64() * 1e3;
        (w.check)(&sim); // outside the timed span cost-wise irrelevant; abort on wrong result
        best_total_ms = best_total_ms.min(total_ms);
    }
    let run_ms = (best_total_ms - compile_ms).max(0.001);
    BenchResult {
        name: w.name,
        stresses: w.stresses,
        cycles,
        compile_ms,
        run_ms,
        cycles_per_sec: cycles as f64 / (run_ms / 1e3),
    }
}

fn main() {
    let mut cycles: u64 = 200_000;
    let mut repeats: usize = 3;
    let mut filter: Option<String> = None;
    let mut json_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--cycles" => {
                cycles = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--cycles N");
            }
            "--repeats" => {
                repeats = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--repeats K");
            }
            "--filter" => filter = args.next(),
            "--json-only" => json_only = true,
            other => {
                eprintln!("unknown arg: {other}");
                eprintln!("usage: xezim-bench [--cycles N] [--repeats K] [--filter substr] [--json-only]");
                std::process::exit(2);
            }
        }
    }

    let host = host_info();
    if !json_only {
        println!("xezim host-cpu benchmark");
        println!(
            "host: {} | arch {} | {} threads | features: {}",
            host.model,
            host.arch,
            host.threads,
            if host.features.is_empty() {
                "-".to_string()
            } else {
                host.features.join(" ")
            }
        );
        println!("cycles/workload: {cycles}   repeats: {repeats} (min taken)");
        println!();
        println!(
            "{:<12} {:<30} {:>12} {:>11} {:>10} {:>13}",
            "workload", "stresses", "compile_ms", "run_ms", "ns/cycle", "cycles/s"
        );
    }

    let mut results: Vec<BenchResult> = Vec::new();
    for w in benchw::workloads() {
        if let Some(f) = &filter {
            if !w.name.contains(f.as_str()) {
                continue;
            }
        }
        let r = run_workload(&w, cycles, repeats);
        if !json_only {
            println!(
                "{:<12} {:<30} {:>12.1} {:>11.1} {:>10.1} {:>13.0}",
                r.name,
                r.stresses,
                r.compile_ms,
                r.run_ms,
                (r.run_ms * 1e6) / r.cycles as f64,
                r.cycles_per_sec
            );
        }
        results.push(r);
    }

    // Machine-readable line for cross-host collection.
    let feats = host
        .features
        .iter()
        .map(|f| format!("\"{f}\""))
        .collect::<Vec<_>>()
        .join(",");
    let body = results
        .iter()
        .map(|r| {
            format!(
                "{{\"name\":\"{}\",\"cycles\":{},\"compile_ms\":{:.2},\"run_ms\":{:.2},\"cycles_per_sec\":{:.0}}}",
                r.name, r.cycles, r.compile_ms, r.run_ms, r.cycles_per_sec
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "BENCH_JSON {{\"model\":\"{}\",\"arch\":\"{}\",\"threads\":{},\"features\":[{}],\"version\":\"{}\",\"results\":[{}]}}",
        host.model.replace('"', "'"),
        host.arch,
        host.threads,
        feats,
        env!("CARGO_PKG_VERSION"),
        body
    );
}
