//! IEEE 1800-2023 §23.6, §23.8, §27.6: a child generate label resolves
//! within the declaring instance, including after cache and artifact reloads.

use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = include_str!("generate_child_label_refs.sv");
const EXPECTED: &str = "T|u0=2 3 4 11 u1=22 23 24 31";

#[test]
fn nested_child_labels_keep_instance_and_parameter_bindings() {
    let sim = xezim::simulate(SOURCE, 10).expect("simulate");
    assert!(sim.output.iter().any(|line| line.message == EXPECTED));
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run simulator");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{args:?}: {text}");
    text
}

#[test]
fn nested_child_labels_survive_cache_and_artifact_roundtrips() {
    let dir = Scratch(
        std::env::temp_dir().join(format!("xezim_child_label_cache_{}", std::process::id())),
    );
    std::fs::create_dir_all(&dir.0).expect("create scratch");
    std::fs::write(dir.0.join("top.sv"), SOURCE).expect("write source");
    for args in [
        vec!["--no-cache", "-s", "top", "top.sv"],
        vec!["--cache-dir", "cache", "-s", "top", "top.sv"],
        vec!["--cache-dir", "cache", "-s", "top", "top.sv"],
    ] {
        assert!(run(&dir.0, &args).contains(EXPECTED));
    }
    for compression in ["none", "3"] {
        run(
            &dir.0,
            &[
                "--compile",
                "--no-cache",
                "--artifact-compression",
                compression,
                "-s",
                "top",
                "-o",
                "design.xezbc",
                "top.sv",
            ],
        );
        assert!(run(&dir.0, &["design.xezbc"]).contains(EXPECTED));
    }
}
