//! #284 / #282: DPI-C prototypes whose return type is named by a typedef,
//! and an exported SV function reached from C through the export trampoline
//! (built with the host compiler, no x86-only flags). IEEE 1800-2023
//! §35.5.5: a typedef of a small value (`int`, `byte`, `real`, a chain of
//! typedefs) maps to the same C type as the type it names. Expected values
//! come from the reference simulator.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shared_lib(dir: &Path, stem: &str, src: &str) -> PathBuf {
    let c = dir.join(format!("{stem}.c"));
    let so = dir.join(format!("{stem}.so"));
    std::fs::write(&c, src).unwrap();
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC"])
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

#[test]
fn typedef_named_dpi_returns_and_export_trampoline() {
    let dir = scratch("dpi_typedef_return");
    let lib = shared_lib(
        &dir,
        "dpitd",
        r#"
extern int sv_inc(int x);
int fi(int x) { return x * 10; }
signed char fb(int x) { return (signed char)x; }
double fr(int x) { return x / 4.0; }
int fi2(int x) { return x - 1; }
int c_call(int x) { return sv_inc(x); }
"#,
    );
    std::fs::write(
        dir.join("top.sv"),
        r#"
module top;
  typedef int I;
  typedef byte B;
  typedef real R;
  typedef I I2;
  import "DPI-C" function I fi(int x);
  import "DPI-C" function B fb(int x);
  import "DPI-C" function R fr(int x);
  import "DPI-C" function I2 fi2(int x);
  import "DPI-C" context function int c_call(int x);
  export "DPI-C" function sv_inc;
  function I sv_inc(int x);
    return x + 1;
  endfunction
  initial begin
    $display("T|fi=%0d fb=%0d fr=%0.2f fi2=%0d", fi(5), fb(300), fr(3), fi2(-7));
    $display("T|export=%0d", c_call(41));
  end
endmodule
"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(&lib)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(
        got,
        ["T|fi=50 fb=44 fr=0.75 fi2=-8", "T|export=42"],
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
