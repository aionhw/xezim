//! IEEE 1800-2023 §7.2.1, §11.8.3: a signed packed-structure member read is
//! signed in compiled code too, and (§11.6.1) the operands of an operator in
//! a concatenation are extended by their own sign, in every execution tier.
//! The expected values are the reference simulator's.
use std::process::Command;

fn run(env: &[(&str, &str)], tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_1159_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("signed_packed_member_reads_1159.sv");
    std::fs::write(&file, include_str!("signed_packed_member_reads_1159.sv")).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--no-cache", file.to_str().unwrap()]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn signed_members_and_concatenation_operands() {
    let expected = [
        "T|members ffffffffffffffff fffffffffffffffe fffffffffffffffc fffffffffffffffd",
        "T|ops fffffffffffffffc ffffffffffffffff fffffffffffffffe fffffffffffffffc 1 -3",
        "T|concat 0000000000007ffe 00000000000000e3",
    ];
    for (tag, env) in [("default", vec![]), ("jit", vec![("XEZIM_JIT", "1")])] {
        assert_eq!(run(&env, tag), expected, "{tag}");
    }
}
