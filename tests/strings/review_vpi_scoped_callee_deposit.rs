//! IEEE 1800-2023 §38.34: a deposit on a net holds until one of its drivers
//! changes. §10.3.2, §13.4.1, §26.3: the name of a called function is not an
//! operand of the continuous assignment, also when the call names it through
//! a package (`p::f(d)`) or the compilation unit (`$unit::uf(d)`), so a
//! write to a module variable of the same name does not drive the net again.
use std::path::Path;
use std::process::Command;

const DEPOSIT_C: &str = r#"
#include "vpi_user.h"

void deposit_int(const char *path) {
    vpiHandle n = vpi_handle_by_name((PLI_BYTE8 *)path, NULL);
    s_vpi_value value = { .format = vpiIntVal };
    value.value.integer = 99;
    vpi_put_value(n, &value, NULL, vpiNoDelay);
}
"#;

const SCOPED_CALLEES: &str = r#"
`timescale 1ns/1ps
package p;
  int bias = 3;
  function automatic int f(input int x);
    f = x + bias;
  endfunction
  function automatic int g(input int x);
    return x * 2;
  endfunction
endpackage
function automatic int uf(input int x);
  uf = x + 7;
endfunction
module top;
  import "DPI-C" context function void deposit_int(input string path);
  int d;
  wire [31:0] w1, w2, w3, w4;
  int f, g, uf_other;
  assign w1 = p::f(d);
  assign w2 = p::g(d);
  assign w3 = $unit::uf(d);
  assign w4 = uf(d);
  initial begin
    d = 4;
    #1 $display("T|initial %0d %0d %0d %0d", w1, w2, w3, w4);
    #1 begin
      deposit_int("top.w1");
      deposit_int("top.w2");
      deposit_int("top.w3");
      deposit_int("top.w4");
    end
    #1 $display("T|held %0d %0d %0d %0d", w1, w2, w3, w4);
    #1 begin
      f = 1;
      g = 2;
      uf_other = 3;
    end
    #1 $display("T|after unrelated %0d %0d %0d %0d", w1, w2, w3, w4);
    #1 d = 5;
    #1 $display("T|changed %0d %0d %0d %0d", w1, w2, w3, w4);
    p::bias = 10;
    #1 $display("T|package variable %0d %0d %0d %0d", w1, w2, w3, w4);
    $finish;
  end
endmodule
"#;

#[test]
fn deposit_holds_on_a_wire_driven_by_a_scoped_function_call() {
    let dir = std::env::temp_dir().join(format!("xezim_scoped_callee_{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("deposit.c"), DEPOSIT_C).unwrap();
    std::fs::write(dir.join("top.sv"), SCOPED_CALLEES).unwrap();
    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()));
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let built = cc
        .arg("-I")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("include"))
        .args(["deposit.c", "-o", "deposit.so"])
        .current_dir(&dir)
        .output()
        .expect("failed to launch C compiler");
    assert!(built.status.success(), "{built:?}");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--no-cache",
            "--dpi-lib",
            "./deposit.so",
            "-s",
            "top",
            "top.sv",
        ])
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    std::fs::remove_dir_all(dir).unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    let got: Vec<_> = text.lines().filter_map(|l| l.strip_prefix("T|")).collect();
    assert_eq!(
        got,
        [
            "initial 7 8 11 11",
            "held 99 99 99 99",
            "after unrelated 99 99 99 99",
            "changed 8 10 12 12",
            "package variable 15 10 12 12",
        ],
        "{text}"
    );
}
