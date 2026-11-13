//! `vpiType` of explicitly typed ports.
//!
//! A port with an explicit variable type reports that variable kind
//! (`vpiIntVar`, `vpiBitVar`, `vpiByteVar`, ...), and an ANSI `input logic`
//! is a net (§23.2.2.3). An `input integer` reports `vpiIntegerNet`, as the
//! reference simulator does.
//! Expected kinds are the reference simulator's.

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

const SV: &str = r#"
module s (input int i, input bit b, input byte y, input shortint si, input longint li,
          input integer g, input real r, input logic [3:0] il, input bit [3:0] bv,
          output int oi, output bit ob, output logic ol, output reg orr);
  assign oi = i;
  assign ob = b;
  assign ol = b;
  initial orr = 0;
endmodule
module top;
  int i = 3; bit b = 1; byte y = 2; shortint si = 4; longint li = 5; integer g = 6;
  real r = 1.5; logic [3:0] il = 1; bit [3:0] bv = 2;
  int oi; bit ob; logic ol; logic orr;
  s u (.i(i), .b(b), .y(y), .si(si), .li(li), .g(g), .r(r), .il(il), .bv(bv),
       .oi(oi), .ob(ob), .ol(ol), .orr(orr));
  import "DPI-C" context function void probe();
  initial #1 probe();
endmodule
"#;

const C: &str = r#"
#include "vpi_user.h"
void probe(void) {
    const char *names[] = {"top.u.i", "top.u.b", "top.u.y", "top.u.si", "top.u.li", "top.u.g",
                           "top.u.r", "top.u.il", "top.u.bv", "top.u.oi", "top.u.ob", "top.u.ol",
                           "top.u.orr", 0};
    for (int k = 0; names[k]; k++) {
        vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)names[k], NULL);
        vpi_printf("T|%s=%d\n", names[k], h ? vpi_get(vpiType, h) : -1);
    }
}
"#;

#[test]
fn typed_ports_report_their_variable_kinds() {
    let d = scratch("vpi_typed_port_kinds");
    let lib = shared_lib(&d, "probe", C);
    std::fs::write(d.join("top.sv"), SV).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(&lib)
        .args(["--no-cache", "-s", "top", "top.sv"])
        .current_dir(&d)
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
    assert_eq!(
        got,
        [
            "T|top.u.i=612",
            "T|top.u.b=620",
            "T|top.u.y=614",
            "T|top.u.si=611",
            "T|top.u.li=610",
            "T|top.u.g=681",
            "T|top.u.r=47",
            "T|top.u.il=36",
            "T|top.u.bv=620",
            "T|top.u.oi=612",
            "T|top.u.ob=620",
            "T|top.u.ol=48",
            "T|top.u.orr=48"
        ],
        "{text}"
    );
}
