//! IEEE 1800-2023 §6.7.1, §10.3.2-3, §28.5, §28.16: cache modes must
//! preserve xezim's uncached value before a delayed driver's first update.
//! §6.7.1 does not settle the value within that first delay; early x is an
//! uncached compatibility check, not an LRM-mandated value.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, args: &[&str]) -> (Vec<String>, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("XEZIM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run simulator");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{args:?}: {stdout}\n{stderr}");
    let lines = stdout
        .lines()
        .filter(|line| line.starts_with("T|"))
        .map(str::to_owned)
        .collect();
    (lines, stderr)
}

fn lifecycle(source: &str, expected: &[&str]) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "xezim_prepared_init_{}_{}_{}",
        std::process::id(),
        stamp,
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).expect("create unique scratch directory");
    let scratch = Scratch(dir);
    std::fs::write(scratch.0.join("top.sv"), source).expect("write source");
    let uncached = run(&scratch.0, &["--no-cache", "top.sv"]).0;
    assert_eq!(uncached, expected, "uncached behavior");
    let args = ["--cache-dir", "cache", "top.sv"];
    let (cold, cold_log) = run(&scratch.0, &args);
    assert!(cold_log.contains("prepared-comb stored"), "{cold_log}");
    let (warm, warm_log) = run(&scratch.0, &args);
    assert!(warm_log.contains("prepared-comb hit"), "{warm_log}");
    let mut removed = 0;
    for entry in std::fs::read_dir(scratch.0.join("cache")).expect("read cache") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "xezcomb") {
            std::fs::remove_file(path).expect("delete prepared table");
            removed += 1;
        }
    }
    assert!(removed > 0, "no prepared tables to delete");
    let (rebuilt, rebuilt_log) = run(&scratch.0, &args);
    assert!(
        rebuilt_log.contains("prepared-comb stored"),
        "{rebuilt_log}"
    );
    assert!(!rebuilt_log.contains("prepared-comb hit"), "{rebuilt_log}");
    run(
        &scratch.0,
        &["--compile", "--no-cache", "-o", "design.xezbc", "top.sv"],
    );
    let artifact = run(&scratch.0, &["--no-cache", "design.xezbc"]).0;
    for (mode, lines) in [
        ("cold", cold),
        ("warm", warm),
        ("prepared tables deleted", rebuilt),
        ("artifact", artifact),
    ] {
        assert_eq!(lines, uncached, "{mode} differs from uncached run");
    }
}

#[test]
fn delayed_driven_nets_match_uncached_in_every_cache_mode() {
    lifecycle(
        include_str!("prepared_cache_driven_init.sv"),
        &[
            "T|t1 wire=x tri0=x tri1=x vector=xxxx gate=x",
            "T|t3 wire=1 tri0=1 tri1=1 vector=1010 gate=1",
        ],
    );
}

#[test]
fn driven_net_initialization_preserves_exceptions_in_every_cache_mode() {
    lifecycle(
        include_str!("prepared_cache_init_exceptions.sv"),
        &[
            "T|t1 undriven=z tri0=0 tri1=1 supply=01 real=0.00 immediate=1",
            "T|t3 undriven=z tri0=0 tri1=1 supply=01 real=1.25 immediate=1",
        ],
    );
}
