//! An import that no `--dpi-lib` library defines binds to the function of
//! that name in the libraries the process already has loaded, as a
//! Verilator model links any C function (IEEE 1800-2023 §35.5.1: the import
//! names a C function). `import "DPI-C" function string getenv(input string
//! name);` used to need a library of its own; without one the first call
//! stopped the run with "has no implementation". An import found nowhere is
//! still fatal (`dpi_library_loading.rs`).

#![cfg(unix)]

use std::process::Command;

const SV: &str = r#"
module top;
  import "DPI-C" function string getenv(input string name);
  import "DPI-C" function int abs(input int x);
  import "DPI-C" function int xezim_no_such_c_function(input int x);
  initial begin
    string v;
    v = getenv("DPI_LIBC_IMPORT_PROBE");
    $display("T|getenv=%s", v);
    $display("T|unset=[%s]", getenv("DPI_LIBC_IMPORT_UNSET_VARIABLE"));
    $display("T|abs=%0d", abs(-7));
    $display("T|missing=%0d", xezim_no_such_c_function(1));
    $display("T|not reached");
  end
endmodule
"#;

#[test]
fn libc_import_needs_no_dpi_lib() {
    let d = std::env::temp_dir().join(format!("xezim_dpi_libc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("top.sv"), SV).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "top.sv"])
        .env("DPI_LIBC_IMPORT_PROBE", "from the environment")
        .env_remove("DPI_LIBC_IMPORT_UNSET_VARIABLE")
        .current_dir(&d)
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&d);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(
        lines,
        [
            "T|getenv=from the environment",
            "T|unset=[]",
            "T|abs=7",
            // The call that has no implementation returns 0 and ends the run
            // after its statement.
            "T|missing=0",
        ],
        "{text}"
    );
    assert!(
        text.contains(
            "DPI import 'xezim_no_such_c_function' has no implementation: \
             C function 'xezim_no_such_c_function' is not in any --dpi-lib library"
        ),
        "{text}"
    );
    // The unimplemented import is fatal, so the run fails.
    assert_eq!(out.status.code(), Some(1), "{text}");
}
