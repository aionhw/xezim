//! IEEE 1800-2023 §27.4: a negative genvar value inside a generate-for
//! iteration reads the same from source, from a cold and a warm design cache
//! and from a compiled artifact. The value is synthesized as a numeric
//! literal whose text is re-read once the cached form is loaded; a leading
//! minus sign read as x.
use std::path::PathBuf;
use std::process::Command;

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(dir: &Dir, args: &[&str]) -> Vec<String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    command.args(args).current_dir(&dir.0);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("XEZIM_") {
            command.env_remove(name);
        }
    }
    let out = command.output().expect("run xezim");
    assert!(
        out.status.success(),
        "{args:?}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.starts_with("T|"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn negative_genvar_values_survive_the_design_cache_and_artifacts() {
    let dir =
        Dir(std::env::temp_dir().join(format!("xezim-negative-genvar-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).expect("mkdir");
    std::fs::write(
        dir.0.join("input.sv"),
        include_str!("negative_genvar_cached.sv"),
    )
    .expect("write source");
    let expected = [
        "T|o 8 19 30 41",
        "T|k=-2 twice=-4 sel=10",
        "T|k=-1 twice=-2 sel=20",
        "T|k=0 twice=0 sel=30",
        "T|k=1 twice=2 sel=40",
    ];
    assert_eq!(run(&dir, &["--no-cache", "input.sv"]), expected);
    for _ in 0..2 {
        assert_eq!(run(&dir, &["--cache-dir", "cache", "input.sv"]), expected);
    }
    assert!(
        run(
            &dir,
            &["--compile", "--no-cache", "-o", "design.xez", "input.sv"]
        )
        .is_empty()
    );
    assert_eq!(run(&dir, &["--simulate", "design.xez"]), expected);
}
