//! IEEE 1800-2023 §7.4.1: an element of a packed array of a named signed
//! type is signed, in procedural and compiled code and in every execution
//! tier; §13.4.1: a function's typedef'd signed result is signed; §11.4.14:
//! a compiled left stream of a part-select of such an array reverses its
//! elements. The expected values are the reference simulator's.
use std::process::Command;

fn run(env: &[(&str, &str)], tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_1107_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("signed_named_packed_elements_1107.sv");
    std::fs::write(&file, include_str!("signed_named_packed_elements_1107.sv")).unwrap();
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
fn named_signed_packed_elements() {
    let expected = [
        "T|proc -362 -362 -362 -724 1686 -181 1 1 -362",
        "T|comb -362 -181 -362",
        "T|call -362",
        "T|stream 33 22 11 33 22 11",
    ];
    for (tag, env) in [("default", vec![]), ("jit", vec![("XEZIM_JIT", "1")])] {
        assert_eq!(run(&env, tag), expected, "{tag}");
    }
}
