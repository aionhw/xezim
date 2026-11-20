//! IEEE 1800-2023 §38.11, §38.3: an element of an unpacked array has the
//! left and right range of its own type, and `vpi_chk_error` fills every
//! string of the error record. cocotb, writing a Python list to an
//! unpacked-array port (`dut.priv_ids_i.value = [...]`, as the I3C core's
//! tests do), makes a handle for each element and reads its range through
//! `vpi_handle(vpiLeftRange, element)`. xezim returned NULL there, cocotb read
//! a value through the NULL handle, and on the error that raised it logged
//! the record's `file`, which xezim had left NULL: the run crashed in the
//! C library's `strlen`. Verilator runs the same test to completion.
//!
//! tests/dpi/vpi_array_elem_range.c reads the ranges of whole arrays, of
//! elements (`[31:0]`, an ascending `[0:7]`), of a sub-array and of an
//! element of it, checks that a bit has none, writes every element and reads
//! the sum the design computes, and checks the error record of a failed
//! call.

use std::path::Path;
use std::process::Command;

#[test]
fn vpi_array_element_ranges_and_error_record() {
    let stem = "vpi_array_elem_range";
    let dir = std::env::temp_dir().join(format!("xezim_{}_{}", stem, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let so = dir.join(format!("{stem}.so"));
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-Wall", "-Werror", "-I"])
        .arg(root.join("include"))
        .arg(root.join(format!("tests/dpi/{stem}.c")))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {stem}.c");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(&so)
        .args(["-s", "top", "--no-cache"])
        .arg(root.join(format!("tests/dpi/{stem}.sv")))
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "xezim failed:\n{text}");
    assert!(!text.contains("FAIL:"), "VPI check failed:\n{text}");
    for line in [
        "CHK|array handles",
        "CHK|ids [0:3]",
        "CHK|down [3:0]",
        "CHK|grid [0:1]",
        "CHK|ids[0] [31:0]",
        "CHK|ids[3] [31:0]",
        "CHK|down[2] [0:7]",
        "CHK|grid[1] [2:0]",
        "CHK|grid[1][0] [15:8]",
        "CHK|ids[0][3] has no left range",
        "CHK|vpi_get_value(NULL) is an error",
        "CHK|error strings are set",
        "CHK|error strings read",
        "CHK|sum of the written elements",
    ] {
        assert!(text.contains(line), "missing `{line}`:\n{text}");
    }
}
