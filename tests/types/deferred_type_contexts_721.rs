//! §6.20.3 / §20.7: shared contexts preserve types, bounds, empty scopes and reloads.

use std::process::Command;

const SOURCE: &str = include_str!("deferred_type_contexts_721.sv");

#[test]
fn shared_type_contexts_keep_instance_bounds_and_clear_empty_scopes() {
    for _ in 0..3 {
        let sim = xezim::simulate(SOURCE, 100).expect("simulate type contexts");
        assert_eq!(
            sim.output
                .iter()
                .filter(|o| o.message == "PASS type contexts")
                .count(),
            1,
        );
    }
}

fn run(command: &mut Command, expect_pass: bool) {
    let output = command.output().expect("run type contexts");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{text}");
    if expect_pass {
        assert!(
            text.lines().any(|line| line == "PASS type contexts"),
            "{text}"
        );
    }
}

#[test]
fn shared_type_contexts_survive_cold_warm_cache_and_artifact() {
    let dir = std::env::temp_dir().join(format!("xezim_type_contexts_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.sv");
    let cache = dir.join("cache");
    let artifact = dir.join("source.xez");
    std::fs::write(&source, SOURCE).unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    for _ in 0..2 {
        run(
            Command::new(bin)
                .args(["--simulate", "-s", "top", "--cache-dir"])
                .arg(&cache)
                .arg(&source),
            true,
        );
    }
    run(
        Command::new(bin)
            .args(["--compile", "--no-cache", "-s", "top"])
            .arg(&source)
            .arg("-o")
            .arg(&artifact),
        false,
    );
    run(Command::new(bin).arg("--simulate").arg(&artifact), true);
    std::fs::remove_dir_all(dir).unwrap();
}
