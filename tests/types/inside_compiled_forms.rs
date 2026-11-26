//! IEEE 1800-2023 §11.4.13, §11.4.5, §12.5.4: set membership compiled to
//! bytecode — continuous assignments, `always_comb`, `always @*`,
//! `always @(v)` and class methods — matches like the interpreter. A real
//! operand compares with `==`, converting a signed member by its sign
//! (`r inside {NEG}` with `NEG = -1`); a `case inside` item is matched by
//! `inside`, so `4'b10??` is a wildcard, a range is a range and a real item
//! or selector compares numerically (a NaN matches nothing). Run with
//! compiled class methods and without (#1281). Expected values are the
//! reference simulator's.

use std::process::Command;

fn run(compile: &str) -> Vec<String> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_inside_compiled_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, include_str!("inside_compiled_forms.sv")).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "simulation failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("T|"))
        .map(String::from)
        .collect()
}

#[test]
fn compiled_inside_and_case_inside_match_like_inside() {
    for compile in ["1", "0"] {
        assert_eq!(
            run(compile),
            [
                "T|assign neg=1 lit=1 real_item=1 wild=1 fill=0",
                "T|case wild=1 range=1 real=1 int_real=1 nan=0 neg=1 wide=1 multi=2 star=1 at=1 init=1",
                "T|again wild=0 range=0 multi=1 at=0 fill=1",
                "T|class 1 1",
                "T|class 0 2 2",
                "T|class 1 3 1",
            ],
            "XEZIM_COMPILE_METHODS={compile}"
        );
    }
}
