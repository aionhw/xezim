//! An interpreted for-init / foreach loop variable writes its index under
//! the BARE name into the runtime signals map, but every read of the same
//! name goes through hierarchical name resolution, whose single-segment
//! leaf-name fallback binds the bare name to an inlined child instance's
//! same-named signal (u.i): write and read then address two different
//! storage cells, so the loop reads x (4-state child) or silently steps
//! the child's counter (2-state child) instead of its own. IEEE 1800-2023
//! 23.6: an unqualified name must never resolve downward into an instance.
//!
//! The SV source merges eleven probes into one self-checking matrix: seven
//! collision probes (4-state child x-drop, silent 2-state hijack, foreach
//! index, resolve-hint poisoning of an array name, aliased storage after a
//! hierarchical write, an ambiguous two-child pick, a child that actively
//! uses its own variable) and four controls that must stay green (compiled
//! loop fast path, block-local, automatic-task local, static-task local).

use std::process::Command;

fn run(name: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xezim_lvcc_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "tb_top", path.to_str().unwrap(), "--no-cache"])
        .output()
        .expect("run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

#[test]
fn loopvar_collision_child_leaf_all_probes() {
    let text = run("merged", include_str!("loopvar_collision_child_leaf.sv"));
    assert!(text.contains("TEST_PASS"), "{text}");
    assert!(!text.contains("FAIL:"), "{text}");
}