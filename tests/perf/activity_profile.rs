//! Diagnostic collection must preserve IEEE 1800 §4 event regions, §6.3
//! four-state values and §10.6 force/release. It supplies no pruning proof.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "xezim_activity_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn run(&self, profile: Option<&Path>, cache: bool, extra: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("XEZIM_") {
                cmd.env_remove(key);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            cmd.env(
                "XEZIM_UNREGISTERED_NONUNICODE",
                std::ffi::OsString::from_vec(vec![0xff]),
            );
        }
        cmd.current_dir(&self.0).args([
            "--simulate",
            "--error-exit",
            "-s",
            "top",
            "design.sv",
            "+iterations=3",
        ]);
        if cache {
            cmd.arg("--cache-dir")
                .arg(self.0.join("cache"))
                .arg("--verbose");
        } else {
            cmd.arg("--no-cache");
        }
        if let Some(path) = profile {
            cmd.env("XEZIM_ACTIVITY_PROFILE", path);
        }
        for (key, value) in extra {
            cmd.env(key, value);
        }
        let out = cmd.output().expect("run activity fixture");
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn write(&self, src: &str) {
        std::fs::write(self.0.join("design.sv"), src).unwrap();
    }

    fn read(&self, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.0.join(name)).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn payload(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.starts_with("CHECK") || line.starts_with("DONE"))
        .map(str::to_string)
        .collect()
}

const SCHEDULING: &str = r#"
module top;
  reg [7:0] a = 8'h3c;
  reg [7:0] q = 0;
  wire [7:0] y = a ^ 8'h55;
  initial begin
    q <= 0;
    q = 1;
    #1;
    if (q !== 0) $fatal(1, "NBA intent lost");
    $display("CHECK q=%h y=%h", q, y);
    a = 8'hxx;
    #0;
    if (y !== 8'hxx) $fatal(1, "X propagation lost");
    force y = 8'h5a;
    #0;
    if (y !== 8'h5a) $fatal(1, "force lost");
    release y;
    #0;
    if (y !== 8'hxx) $fatal(1, "release lost");
    a = 8'hzz;
    #0;
    if (y !== 8'hxx) $fatal(1, "Z propagation lost");
    a = 8'h0f;
    #1;
    if (y !== 8'h5a) $fatal(1, "final value wrong");
    $display("DONE q=%h y=%h", q, y);
    $finish;
  end
endmodule
"#;

#[test]
fn collecting_without_timing_preserves_regions_and_four_state_values() {
    let f = Fixture::new();
    f.write(SCHEDULING);
    let plain = f.run(None, false, &[]);
    let observed = f.run(
        Some(&f.0.join("activity.json")),
        false,
        &[
            ("XEZIM_ACTIVITY_STRIDE", "1"),
            ("XEZIM_UNREGISTERED_SECRET", "not-exported"),
        ],
    );
    assert_eq!(payload(&plain), vec!["CHECK q=00 y=69", "DONE q=00 y=5a"]);
    assert_eq!(payload(&plain), payload(&observed));
    let report = f.read("activity.json");
    assert_eq!(report["schema"], 1);
    assert_eq!(report["plusargs"], serde_json::json!(["+iterations=3"]));
    assert!(report["epochs"].as_u64().unwrap() > 1);
    assert_eq!(report["sampled_epochs"], report["epochs"]);
    assert_eq!(report["gate_lane_disabled"], true);
    assert!(
        report["runtime_options"]
            .get("XEZIM_PROFILE_TIMING")
            .is_none()
    );
    assert!(
        report["runtime_options"]
            .get("XEZIM_UNREGISTERED_SECRET")
            .is_none()
    );
    assert!(
        report["runtime_options"]
            .get("XEZIM_UNREGISTERED_NONUNICODE")
            .is_none()
    );
    let entries = report["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["evaluations"].as_u64().unwrap() > 0)
    );
    for entry in entries {
        let active = entry["active_epochs"].as_u64().unwrap();
        assert!(entry["evaluations"].as_u64().unwrap() >= active);
        assert_eq!(entry["sampled_active_epochs"].as_u64().unwrap(), active);
        assert_eq!(entry["minhash"].is_null(), active == 0);
    }
}

const SMALL: &str = r#"
module top;
  reg [7:0] a = 1;
  wire [7:0] y = a ^ 8'h55;
  initial begin
    #1; a = 2; #1;
    $display("DONE %h", y);
    $finish;
  end
endmodule
"#;

#[test]
fn separate_processes_and_prepared_cache_preserve_profile_identity() {
    let f = Fixture::new();
    f.write(SMALL);
    let cold = f.run(Some(&f.0.join("cold.json")), true, &[]);
    let warm = f.run(Some(&f.0.join("warm.json")), true, &[]);
    assert_eq!(payload(&cold), vec!["DONE 57"]);
    assert_eq!(payload(&cold), payload(&warm));
    assert!(String::from_utf8_lossy(&warm.stderr).contains("prepared-comb hit"));
    let a = f.read("cold.json");
    let b = f.read("warm.json");
    for key in [
        "design_fingerprint",
        "layout_fingerprint",
        "execution_fingerprint",
    ] {
        assert_eq!(a[key], b[key], "unstable {key}");
        assert_eq!(a[key].as_str().unwrap().len(), 32);
    }
    assert_eq!(a["entries"], b["entries"]);
    f.write(&SMALL.replace("8'h55", "8'haa"));
    f.run(Some(&f.0.join("changed.json")), true, &[]);
    let changed = f.read("changed.json");
    assert_eq!(
        a["entries"].as_array().unwrap().len(),
        changed["entries"].as_array().unwrap().len()
    );
    assert_ne!(a["design_fingerprint"], changed["design_fingerprint"]);
    assert_ne!(a["layout_fingerprint"], changed["layout_fingerprint"]);
}

#[test]
fn invalid_sampling_and_unsupported_backend_do_not_emit_partial_profiles() {
    let f = Fixture::new();
    f.write(SMALL);
    for extra in [
        vec![("XEZIM_ACTIVITY_STRIDE", "0")],
        vec![("XEZIM_BSP_SETTLE", "1")],
    ] {
        let out = f.run(Some(&f.0.join("rejected.json")), false, &extra);
        assert_eq!(payload(&out), vec!["DONE 57"]);
        assert!(!f.0.join("rejected.json").exists());
        assert!(String::from_utf8_lossy(&out.stderr).contains("profile disabled"));
    }
}
