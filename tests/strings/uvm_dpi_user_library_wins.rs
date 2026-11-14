//! A `--dpi-lib` library that defines one of the UVM DPI-C helpers serves
//! every call to it; the built-in port of those helpers only covers symbols
//! no loaded library resolves (IEEE 1800-2023 §35.5: the import binds to the
//! user's C function). The rule holds for direct calls and for calls made
//! from class methods and package functions that run as bytecode.

use std::process::Command;

const USER_C: &str = r#"
#include <string.h>
const char* uvm_dpi_get_tool_name_c(void) { return "userlib"; }
unsigned char uvm_re_compexecfree(const char* re, const char* str, unsigned char deglob,
                                  int* exec_ret) {
  (void)re; (void)str; (void)deglob;
  *exec_ret = 42;
  return 0;
}
"#;

const SV: &str = r#"
package p;
  import "DPI-C" function string uvm_dpi_get_tool_name_c();
  import "DPI-C" function byte unsigned uvm_re_compexecfree(string re, string str,
                                                            byte unsigned deglob,
                                                            output int exec_ret);
  function automatic int pkg_match(string re, string s);
    int r;
    byte unsigned ok;
    ok = uvm_re_compexecfree(re, s, 0, r);
    return r + ok;
  endfunction
endpackage
module top;
  import p::*;
  class K;
    function int cls_match(string re, string s);
      int r;
      byte unsigned ok;
      ok = uvm_re_compexecfree(re, s, 0, r);
      return r + ok;
    endfunction
  endclass
  initial begin
    K k = new;
    int r, s1, s2;
    byte unsigned ok;
    ok = uvm_re_compexecfree("ab", "ab", 0, r);
    $display("T|tool=%s ok=%0d exec=%0d", uvm_dpi_get_tool_name_c(), ok, r);
    for (int i = 0; i < 1200; i++) begin
      s1 += k.cls_match("ab", "ab");
      s2 += pkg_match("ab", "ab");
    end
    $display("T|cls=%0d pkg=%0d", s1, s2);
  end
endmodule
"#;

#[test]
fn user_dpi_library_overrides_builtin_uvm_helpers() {
    let d = std::env::temp_dir().join(format!("xezim_dpi_user_wins_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("user.c"), USER_C).unwrap();
    std::fs::write(d.join("top.sv"), SV).unwrap();
    let so = d.join("user.so");
    let build = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC"])
        .arg(d.join("user.c"))
        .arg("-o")
        .arg(&so)
        .output()
        .expect("failed to launch cc");
    assert!(
        build.status.success(),
        "cc failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(&so)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(&d)
        .env_remove("XEZIM_COMPILE_METHODS")
        .env_remove("XEZIM_METHOD_TIER")
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_dir_all(&d);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    let got: Vec<&str> = text.lines().filter(|l| l.starts_with("T|")).collect();
    // The reference simulator with the same library prints these lines.
    assert_eq!(
        got,
        ["T|tool=userlib ok=0 exec=42", "T|cls=50400 pkg=50400"],
        "{text}"
    );
}
