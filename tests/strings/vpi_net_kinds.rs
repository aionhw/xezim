//! VPI object kinds of implicit nets and array reads.
//!
//! Two elaboration bugs showed through VPI:
//!
//! * A continuous assignment READING an unpacked-array element
//!   (`assign w2 = m1[2];`) made the §6.10 implicit-net pass register a 1-bit
//!   placeholder net named after the array, so `vpi_iterate(vpiNet)` listed a
//!   phantom `top.m1` and `vpi_handle_by_name("top.m1")` found it.
//! * §23.2.2.3: a port with no net type and no data type (`input a`,
//!   `output [3:0] d`, a non-ANSI `input a;` never redeclared) is a net of the
//!   default net type, and so is an ANSI `input logic a`. Only ports spelled
//!   with an explicit `wire` were recorded as nets, so the rest reported
//!   `vpiReg`, in sub-modules and at the top alike. A non-ANSI port completed
//!   by `reg d;` stays a variable, as does an explicitly typed output and a
//!   non-ANSI `input logic e;`.
//!
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
module s1 (a, y, d, e, f);
  input a;
  output y;
  output d;
  reg d;
  input logic e;
  output [1:0] f;
  assign y = ~a;
  assign f = {a, a};
  initial d = 0;
endmodule
module s2 (input a, input wire b, input logic [3:0] c, output y, output logic z,
           output var v);
  assign y = a & b;
  assign z = c[0];
  assign v = a;
endmodule
module top (input tin);
  logic [7:0] m1 [3:0];
  wire [7:0] w2;
  assign w2 = m1[2];
  reg r;
  wire y1, d1, y2, z2, v2;
  wire [1:0] f1;
  s1 u1 (.a(r), .y(y1), .d(d1), .e(r), .f(f1));
  s2 u2 (.a(r), .b(r), .c(4'h1), .y(y2), .z(z2), .v(v2));
  import "DPI-C" context function void probe();
  initial begin
    m1[2] = 8'h5a;
    r = 0;
    #1 probe();
    $finish;
  end
endmodule
"#;

const C: &str = r#"
#include <stdio.h>
#include "vpi_user.h"
static const char *kind(int t) {
    return t == vpiNet ? "net" : t == vpiReg ? "reg" : "other";
}
static void list_nets(const char *scope) {
    vpiHandle it = vpi_iterate(vpiNet, vpi_handle_by_name((PLI_BYTE8 *)scope, NULL));
    vpiHandle h;
    while (it && (h = vpi_scan(it))) vpi_printf("NET|%s\n", vpi_get_str(vpiFullName, h));
}
void probe(void) {
    list_nets("top");
    vpiHandle m = vpi_handle_by_name("top.m1", NULL);
    vpi_printf("M1|%s\n", m && vpi_get(vpiType, m) == vpiNet ? "net" : "not-net");
    const char *names[] = {"top.tin", "top.u1.a", "top.u1.y", "top.u1.d", "top.u1.e",
                           "top.u1.f", "top.u2.a", "top.u2.b", "top.u2.c", "top.u2.y",
                           "top.u2.z", "top.u2.v", 0};
    for (int i = 0; names[i]; i++) {
        vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)names[i], NULL);
        vpi_printf("KIND|%s=%s\n", names[i], h ? kind(vpi_get(vpiType, h)) : "null");
    }
    vpiHandle w2 = vpi_handle_by_name("top.w2", NULL);
    s_vpi_value v;
    v.format = vpiHexStrVal;
    vpi_get_value(w2, &v);
    vpi_printf("W2|%s\n", v.value.str);
}
"#;

/// §37.11: simulator-created edge aliases are not public VPI objects, while
/// the original nets remain enumerable, readable and callback-capable.
#[test]
fn edge_aliases_are_hidden_from_vpi_scope_members() {
    let sv = r#"
`timescale 1ns/1ns
module observer(inout [3:0] link);
  int rising = 0;
  always @(posedge link[2]) rising++;
endmodule
module top;
  logic [7:0] drive = 0;
  wire [7:0] bus;
  int local_edges = 0, computed_edges = 0;
  assign bus = drive;
  observer u(.link(bus[7:4]));
  always @(posedge drive[1]) local_edges++;
  always @(posedge (drive[1] && drive[6])) computed_edges++;
  import "DPI-C" context function void inspect(input int phase);
  initial begin
    #2 inspect(0);
    #2 drive = 8'h02;
    #2 drive = 8'h42;
    #2 inspect(1);
    $display("T| edges port=%0d local=%0d computed=%0d", u.rising, local_edges, computed_edges);
    $finish;
  end
endmodule
"#;
    let c = r#"
#include <string.h>
#include "vpi_user.h"
static int callbacks;
static s_vpi_value callback_value;
static s_vpi_time callback_time;
static PLI_INT32 changed(p_cb_data data) {
    (void)data;
    callbacks++;
    return 0;
}
static void list_members(const char *scope, int kind) {
    vpiHandle owner = vpi_handle_by_name((PLI_BYTE8 *)scope, NULL);
    vpiHandle it = vpi_iterate(kind, owner), item;
    while (it && (item = vpi_scan(it))) {
        const char *name = vpi_get_str(vpiFullName, item);
        if (kind == vpiNet) vpi_printf("NET|%s\n", name);
        if (strstr(name, "__xz_")) vpi_printf("INTERNAL|%s\n", name);
    }
}
static int read_integer(const char *name) {
    vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)name, NULL);
    if (!h) return -1;
    s_vpi_value value;
    value.format = vpiIntVal;
    vpi_get_value(h, &value);
    return value.value.integer;
}
void inspect(int phase) {
    if (!phase) {
        list_members("top", vpiNet);
        list_members("top", vpiReg);
        list_members("top.u", vpiNet);
        list_members("top.u", vpiReg);
        s_cb_data cb;
        memset(&cb, 0, sizeof(cb));
        callback_value.format = vpiIntVal;
        callback_time.type = vpiSimTime;
        cb.reason = cbValueChange;
        cb.cb_rtn = changed;
        cb.obj = vpi_handle_by_name("top.bus", NULL);
        cb.value = &callback_value;
        cb.time = &callback_time;
        vpi_register_cb(&cb);
        vpi_printf("T| initial bus=%d link=%d\n", read_integer("top.bus"), read_integer("top.u.link"));
    } else {
        vpi_printf("T| final bus=%d link=%d callbacks=%d\n", read_integer("top.bus"), read_integer("top.u.link"), callbacks);
    }
}
"#;
    let d = scratch("vpi_edge_alias_visibility");
    let lib = shared_lib(&d, "edge_probe", c);
    let text = run(&d, &lib, sv);
    let _ = std::fs::remove_dir_all(&d);
    let mut nets: Vec<&str> = text.lines().filter(|l| l.starts_with("NET|")).collect();
    nets.sort_unstable();
    assert_eq!(nets, ["NET|top.bus", "NET|top.u.link"], "{text}");
    assert!(!text.contains("INTERNAL|"), "{text}");
    for expected in [
        "T| initial bus=0 link=0",
        "T| final bus=66 link=4 callbacks=2",
        "T| edges port=1 local=1 computed=1",
    ] {
        assert!(text.contains(expected), "missing `{expected}`:\n{text}");
    }
}

#[test]
fn array_reads_add_no_net_and_implicit_ports_are_nets() {
    let d = scratch("vpi_net_kinds");
    let lib = shared_lib(&d, "probe", C);
    let text = run(&d, &lib, SV);
    let _ = std::fs::remove_dir_all(&d);
    // The object model iterates in declaration order; the set is what counts.
    let mut nets: Vec<&str> = text.lines().filter(|l| l.starts_with("NET|")).collect();
    nets.sort_unstable();
    assert_eq!(
        nets,
        [
            "NET|top.d1",
            "NET|top.f1",
            "NET|top.tin",
            "NET|top.v2",
            "NET|top.w2",
            "NET|top.y1",
            "NET|top.y2",
            "NET|top.z2"
        ],
        "{text}"
    );
    for line in [
        "M1|not-net",
        "KIND|top.tin=net",
        "KIND|top.u1.a=net",
        "KIND|top.u1.y=net",
        "KIND|top.u1.d=reg",
        "KIND|top.u1.e=reg",
        "KIND|top.u1.f=net",
        "KIND|top.u2.a=net",
        "KIND|top.u2.b=net",
        "KIND|top.u2.c=net",
        "KIND|top.u2.y=net",
        "KIND|top.u2.z=reg",
        "KIND|top.u2.v=reg",
        "W2|5a",
    ] {
        assert!(text.contains(line), "missing `{line}`:\n{text}");
    }
}
