//! IEEE 1800-2023 §11.8.1, §11.8.2: an unsigned operand of any kind makes
//! a context-determined expression unsigned, in every execution tier.
use std::process::Command;

fn run(env: &[(&str, &str)], tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_1158_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("unsigned_operand_kinds_1158.sv");
    std::fs::write(&file, include_str!("unsigned_operand_kinds_1158.sv")).unwrap();
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
fn unsigned_operands_make_the_expression_unsigned() {
    let row = "000000000000017f 000000000000017f 000000000000017f 000000000000017f \
               000000000000017f 00000000000001fe 000000000000017f 000000000000017f";
    let expected = [
        format!("T|assign {row}"),
        format!("T|comb {row}"),
        format!("T|proc {row}"),
    ];
    for (tag, env) in [("default", vec![]), ("jit", vec![("XEZIM_JIT", "1")])] {
        assert_eq!(run(&env, tag), expected, "{tag}");
    }
}
