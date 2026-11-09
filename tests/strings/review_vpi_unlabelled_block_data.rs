//! §27.6, §37: the VPI object model keys an unlabelled generate block's
//! declarations under its implicit name (`top.genblk1.x`), as elaboration
//! does, so a handle reads the block's variable: not nothing, and not a
//! same-named variable of the enclosing module.

use std::path::Path;
use std::process::Command;

const READER_C: &str = r###"#include <string.h>
#include "vpi_user.h"
static void show(const char *n) {
    vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)n, NULL);
    if (!h) { vpi_printf("VB|%s none\n", n); return; }
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(h, &v);
    vpi_printf("VB|%s %d\n", n, v.value.integer);
}
static void vars(const char *scope) {
    vpiHandle s = vpi_handle_by_name((PLI_BYTE8 *)scope, NULL), it, c;
    if (!s) { vpi_printf("VB|scope %s none\n", scope); return; }
    it = vpi_iterate(vpiVariables, s);
    while (it && (c = vpi_scan(it))) {
        s_vpi_value v;
        v.format = vpiIntVal;
        vpi_get_value(c, &v);
        vpi_printf("VB|in %s %s %d\n", scope, vpi_get_str(vpiFullName, c), v.value.integer);
    }
}
static PLI_INT32 rd(PLI_BYTE8 *u) {
    (void)u;
    show("top.x");
    show("top.genblk1.x");
    show("top.genblk2.y");
    show("top.u.genblk1.x");
    vars("top.genblk1");
    vars("top.u.genblk1");
    return 0;
}
static void reg(void) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = vpiSysTask;
    d.tfname = (PLI_BYTE8 *)"$rd";
    d.calltf = rd;
    vpi_register_systf(&d);
}
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;

/// The `VB|` lines of `sv` run with the reader above.
fn read(tag: &str, sv: &str) -> Vec<String> {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("rd.c"), READER_C).unwrap();
    std::fs::write(d.join("top.sv"), sv).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let so = d.join("rd.so");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg(d.join("rd.c"))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {tag}");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--vpi-lib")
        .arg(&so)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(&d)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&d);
    assert!(out.status.success(), "{text}");
    text.lines()
        .filter(|l| l.starts_with("VB|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_handle_reads_an_unlabelled_blocks_variable() {
    let got = read(
        "vpi_unlabelled_block",
        "module child; if (1) begin int x = 40; end endmodule
module top;
  if (1) begin int x = 2; end
  if (1) begin int y = 3; end
  child u();
  initial #1 $rd;
endmodule
",
    );
    assert_eq!(
        got,
        [
            "VB|top.x none",
            "VB|top.genblk1.x 2",
            "VB|top.genblk2.y 3",
            "VB|top.u.genblk1.x 40",
            "VB|in top.genblk1 top.genblk1.x 2",
            "VB|in top.u.genblk1 top.u.genblk1.x 40",
        ]
    );
}

#[test]
fn a_block_variable_that_hides_a_module_variable_is_the_blocks() {
    let got = read(
        "vpi_unlabelled_block_hiding",
        "module child; int x = 30; if (1) begin int x = 40; end endmodule
module top;
  int x = 1;
  if (1) begin int x = 2; end
  if (1) begin int y = 3; end
  child u();
  initial #1 $rd;
endmodule
",
    );
    assert_eq!(
        got,
        [
            "VB|top.x 1",
            "VB|top.genblk1.x 2",
            "VB|top.genblk2.y 3",
            "VB|top.u.genblk1.x 40",
            "VB|in top.genblk1 top.genblk1.x 2",
            "VB|in top.u.genblk1 top.u.genblk1.x 40",
        ]
    );
}
