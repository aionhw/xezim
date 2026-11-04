//! A compiled artifact named next to a source file, or under `--compile`, is
//! refused with an error instead of being parsed as SystemVerilog text,
//! which panicked on its binary payload (#738, reported by git-hevangel).

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
}

#[test]
fn artifact_must_be_the_only_file() {
    let bin = xezim_bin();
    if !bin.exists() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("xezim_art738_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("base.sv"),
        "module top; initial $display(\"BASE\"); endmodule\n",
    )
    .expect("write");
    std::fs::write(
        dir.join("extra_pkg.sv"),
        "package extra_pkg; localparam int K = 1; endpackage\n",
    )
    .expect("write");
    let art = dir.join("base.art");
    let o = Command::new(&bin)
        .arg("--compile")
        .arg(dir.join("base.sv"))
        .args(["-s", "top", "-o"])
        .arg(&art)
        .output()
        .expect("compile");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    let refused = |args: &[&std::path::Path], compile: bool| {
        let mut c = Command::new(&bin);
        if compile {
            c.arg("--compile");
        }
        let o = c.args(args).output().expect("run");
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{err}");
        assert!(err.contains("is a compiled artifact"), "{err}");
        assert!(!err.contains("panicked"), "{err}");
    };
    refused(&[&art, &dir.join("extra_pkg.sv")], false);
    refused(&[&dir.join("extra_pkg.sv"), &art], false);
    refused(&[&art], true);

    let o = Command::new(&bin).arg(&art).output().expect("run");
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("BASE"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
