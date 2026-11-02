//! §23.10, #721: an instance's prepared body is cached under the parameters
//! its own scope can read. Its scope also holds the parameters of every
//! instance above it, and keying on all of them gave nearly every instance a
//! body and a key of its own, kept for the whole elaboration: a deep design
//! grew without bound. Here 64 `wrap` instances differ in `I`, which the
//! parameterless `leaf` below them cannot read, so the 256 leaves share one
//! body, and the elaboration statistics (`XEZIM_COMPILE_PHASES`) count fewer
//! than 128 cache misses where each `leaf` used to add one per `wrap`.
use std::path::PathBuf;
use std::process::Command;

#[test]
fn a_body_is_shared_by_instances_whose_ancestors_differ() {
    let sv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/hierarchy/prepared_body_shared_across_ancestors.sv");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_COMPILE_PHASES", "1")
        .args(["--no-cache", sv.to_str().unwrap()])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "run failed:\n{stdout}{stderr}");
    let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(lines, ["T|wrap I=63 bits=4", "T|leaf W=4"], "{stdout}");
    let misses = stderr
        .lines()
        .filter_map(|l| l.strip_prefix("[elab-stats] "))
        .flat_map(|l| l.split_whitespace())
        .filter_map(|kv| kv.strip_prefix("prep_misses=")?.parse::<u64>().ok())
        .max()
        .expect("no prepared-body statistics");
    assert!(misses < 128, "{misses} prepared bodies:\n{stderr}");
}
