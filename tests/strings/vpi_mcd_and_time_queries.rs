//! VPI multichannel descriptors and output (#205), and the time unit /
//! precision queries from VPI and DPI (#206).
//!
//! `vpi_mcd_open`/`close`/`flush`/`name`, `vpi_flush`, `vpi_compare_objects`
//! and `vpi_get64` were missing. `vpi_get(vpiTimeUnit/vpiTimePrecision)` had
//! no defines in `vpi_user.h` and answered with the simulation's tick even for
//! a module; `svGetTime`, `svGetTimeUnit` and `svGetTimePrecision` did not
//! exist.

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
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", c.display());
    so
}

fn run(dir: &Path, lib: &Path, sv: &str) -> String {
    std::fs::write(dir.join("top.sv"), sv).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(lib)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    text
}

const SV: &str = r#"
`timescale 1us/1ns
module sub; endmodule
`timescale 1ns/1ps
module top;
  logic [11:0] sig;
  logic other;
  sub u_sub();
  import "DPI-C" context function void probe();
  initial begin #5; probe(); $finish; end
endmodule
"#;

const C: &str = r#"
#include <stdint.h>
#include "vpi_user.h"
#include "svdpi.h"
void probe(void) {
    PLI_UINT32 fd = vpi_mcd_open("mcd_out.txt");
    vpi_printf("MCD|open=%s\n", fd > 1 ? "ok" : "fail");
    vpi_mcd_printf(fd | 1, "MCD|both %d\n", 7);
    vpi_printf("MCD|name=%s\n", vpi_mcd_name(fd) ? vpi_mcd_name(fd) : "(null)");
    vpi_printf("MCD|stdout=%s\n", vpi_mcd_name(1));
    vpi_printf("MCD|flush=%d\n", vpi_mcd_flush(fd));
    vpi_printf("MCD|close=%u\n", vpi_mcd_close(fd));
    vpi_printf("MCD|after close=%s\n", vpi_mcd_name(fd) ? "open" : "null");
    vpi_printf("MCD|vpi_flush=%d\n", vpi_flush());
    vpiHandle a = vpi_handle_by_name("top.sig", NULL);
    vpiHandle b = vpi_handle_by_name("top.sig", NULL);
    vpiHandle c = vpi_handle_by_name("top.other", NULL);
    vpi_printf("CMP|same=%d diff=%d\n", vpi_compare_objects(a, b), vpi_compare_objects(a, c));
    vpi_printf("G64|size=%lld\n", (long long)vpi_get64(vpiSize, a));
    vpiHandle sub = vpi_handle_by_name("top.u_sub", NULL);
    vpi_printf("TIME|sim unit=%d prec=%d\n", vpi_get(vpiTimeUnit, NULL), vpi_get(vpiTimePrecision, NULL));
    vpi_printf("TIME|sub unit=%d prec=%d\n", vpi_get(vpiTimeUnit, sub), vpi_get(vpiTimePrecision, sub));
    int32_t u = 0, p = 0;
    svScope s = svGetScopeFromName("top.u_sub");
    svGetTimeUnit(s, &u);
    svGetTimePrecision(s, &p);
    vpi_printf("SV|sub unit=%d prec=%d\n", u, p);
    svGetTimeUnit(NULL, &u);
    svGetTimePrecision(NULL, &p);
    vpi_printf("SV|sim unit=%d prec=%d\n", u, p);
    svTimeVal t;
    t.type = vpiSimTime;
    svGetTime(NULL, &t);
    vpi_printf("SV|ticks=%u\n", t.low);
    t.type = vpiScaledRealTime;
    svGetTime(s, &t);
    vpi_printf("SV|sub time=%.3f\n", t.real);
}
"#;

#[test]
fn mcd_output_compare_get64_and_time_queries() {
    let d = scratch("vpi_mcd_time");
    let lib = shared_lib(&d, "probe", C);
    let text = run(&d, &lib, SV);
    let file = std::fs::read_to_string(d.join("mcd_out.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&d);
    for line in [
        "MCD|open=ok",
        "MCD|both 7",
        "MCD|name=mcd_out.txt",
        "MCD|stdout=stdout",
        "MCD|flush=0",
        "MCD|close=0",
        "MCD|after close=null",
        "MCD|vpi_flush=0",
        "CMP|same=1 diff=0",
        "G64|size=12",
        "TIME|sim unit=-12 prec=-12",
        "TIME|sub unit=-6 prec=-9",
        "SV|sub unit=-6 prec=-9",
        "SV|sim unit=-12 prec=-12",
        "SV|ticks=5000",
        "SV|sub time=0.005",
    ] {
        assert!(text.contains(line), "missing `{line}`:\n{text}");
    }
    assert!(file.contains("MCD|both 7"), "the file channel got {file:?}");
}
