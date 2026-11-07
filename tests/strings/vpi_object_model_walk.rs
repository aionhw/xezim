//! The VPI object model (IEEE 1800-2017 chapter 37), walked from a VPI
//! module: scopes and instances (modules, interfaces, programs, packages,
//! generate scopes and arrays, named blocks, tasks and functions), declared
//! objects with their declared types, ranges and typespecs, processes,
//! continuous assignments, primitives, specify paths and timing checks,
//! ports and their connections, and name lookup, each with its source
//! location.
//!
//! xezim flattens the design at elaboration; the object model is rebuilt
//! from the sources on the first VPI call that needs it
//! (`src/compiler/simulator/vpi_model.rs`). Before it, only instances and
//! signal-table names were visible: generate scopes, processes, tasks,
//! primitives and packages did not exist, `vpiType` came from a flat
//! type table (a `q` in one module took the type of a `q` in another), and
//! placeholder signals for generate block names showed up as variables.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Build `src` as a VPI module and run `sv` with it; return the `OM|` lines.
fn walk(tag: &str, src: &str, sv: &str, top: Option<&str>) -> Vec<String> {
    walk_sources(tag, src, &[("top.sv", sv)], top)
}

fn walk_sources(tag: &str, src: &str, sources: &[(&str, &str)], top: Option<&str>) -> Vec<String> {
    let d = scratch(tag);
    std::fs::write(d.join("common.h"), PRELUDE).unwrap();
    std::fs::write(d.join("walk.c"), src).unwrap();
    for (name, source) in sources {
        std::fs::write(d.join(name), source).unwrap();
    }
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let so = d.join("walk.so");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg("-I")
        .arg(&d)
        .arg(d.join("walk.c"))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc")
        .success();
    assert!(ok, "cc failed for {}", tag);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.arg("--vpi-lib").arg(&so).arg("--no-cache");
    if let Some(t) = top {
        cmd.args(["-s", t]);
    }
    let out = cmd
        .args(sources.iter().map(|(name, _)| name))
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
        .filter(|l| l.starts_with("OM|"))
        .map(str::to_string)
        .collect()
}

fn check(got: &[String], want: &[&str]) {
    let missing: Vec<&&str> = want
        .iter()
        .filter(|w| !got.iter().any(|g| g == *w))
        .collect();
    assert!(
        missing.is_empty(),
        "missing lines:\n{}\n--- got:\n{}",
        missing
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        got.join("\n")
    );
    assert_eq!(
        got.len(),
        want.len(),
        "line count; got:\n{}",
        got.join("\n")
    );
}

const PRELUDE: &str = r###"#include <stdio.h>
#include <string.h>
#include "sv_vpi_user.h"
static const char *S(const char *s) { return s ? s : "-"; }
static const char *base(const char *f) {
    if (!f) return "-";
    const char *b = strrchr(f, '/');
    return b ? b + 1 : f;
}
static int ival(vpiHandle h) {
    s_vpi_value v;
    v.format = vpiIntVal;
    vpi_get_value(h, &v);
    return v.format == vpiIntVal ? v.value.integer : -999;
}
static const char *tname(vpiHandle h) { return h ? S(vpi_get_str(vpiType, h)) : "NULL"; }
static const char *fname(vpiHandle h) { return h ? S(vpi_get_str(vpiFullName, h)) : "NULL"; }
static vpiHandle H(const char *n) { return vpi_handle_by_name((PLI_BYTE8 *)n, NULL); }
static void list(const char *tag, PLI_INT32 rel, vpiHandle ref) {
    char buf[4096];
    buf[0] = 0;
    vpiHandle it = vpi_iterate(rel, ref), o;
    while (it && (o = vpi_scan(it))) {
        char one[256];
        snprintf(one, sizeof one, "%s%s:%s", buf[0] ? " " : "", S(vpi_get_str(vpiName, o)), tname(o));
        strncat(buf, one, sizeof buf - strlen(buf) - 1);
        vpi_free_object(o);
    }
    vpi_printf("OM|%s=%s\n", tag, buf);
}
static void reg_task(const char *name, PLI_INT32 (*fn)(PLI_BYTE8 *)) {
    s_vpi_systf_data d;
    memset(&d, 0, sizeof d);
    d.type = vpiSysTask;
    d.tfname = (PLI_BYTE8 *)name;
    d.calltf = fn;
    vpi_register_systf(&d);
}
"###;

/// The design the first three walks share. Line numbers are asserted, so
/// keep its text stable. The program's initial procedure waits past the
/// walk: the run ends when it does (§24.3).
const DESIGN: &str = r###"`timescale 1ns/1ps
package cfg_pkg;
  parameter int WIDTH = 8;
  localparam logic [3:0] MAGIC = 4'hA;
  typedef enum logic [1:0] {IDLE = 0, BUSY = 1, DONE = 3} state_e;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  int pkg_count;
  function automatic int twice(input int x); return 2 * x; endfunction
endpackage
interface bus_if #(parameter int W = 8) (input logic clk);
  logic [W-1:0] data;
  logic valid;
  modport mst (output data, output valid, input clk);
endinterface
primitive udp_and (out, a, b);
  output out; input a, b;
  table
    0 ? : 0;
    ? 0 : 0;
    1 1 : 1;
  endtable
endprimitive
`celldefine
module cell_buf (output y, input a);
  buf b0 (y, a);
  specify
    (a => y) = 1;
  endspecify
endmodule
`endcelldefine
module leaf #(parameter int W = 4) (input logic clk, input logic [W-1:0] d, output logic [W-1:0] q);
  always_ff @(posedge clk) q <= d;
endmodule
module dff_chk (input clk, input d, output reg q);
  reg notifier;
  always @(posedge clk) q <= d;
  specify
    $setup(d, posedge clk, 1, notifier);
    $hold(posedge clk, d, 1, notifier);
  endspecify
endmodule
program prog (input logic clk);
  int pcount;
  initial begin pcount = 7; #100; end
endprogram
module top;
  import cfg_pkg::*;
  parameter int N = 2;
  localparam int M = N + 1;
  logic clk = 0;
  wire [3:0] w;
  logic [7:0] lv = 8'hA5;
  bit [0:3] b4;
  integer i32 = -5;
  int iv;
  byte by;
  shortint si;
  longint li;
  real rv = 2.5;
  time tv;
  string s = "hello";
  state_e st = BUSY;
  pair_t pr = 8'h3C;
  struct { int a; logic [3:0] b; } us;
  union packed { logic [7:0] x; logic [7:0] y; } un;
  logic [7:0] mem [0:3]; logic [3:0] m2 [2][3];
  wire [1:0] nets [2];
  int dyn [];
  int qu [$];
  int aa [string];
  const int CI = 9;
  event ev;
  wire a_out, u_out, cb_y, sw_out;
  wand wa;
  tri1 t1;
  bus_if #(.W(8)) bif (.clk(clk));
  leaf #(.W(4)) u_leaf (.clk(clk), .d(w), .q());
  cell_buf u_cell (.y(cb_y), .a(clk));
  dff_chk u_dff (.clk(clk), .d(lv[0]), .q());
  prog p_inst (.clk(clk));
  for (genvar g = 0; g < N; g++) begin : gl
    logic [3:0] gsig; assign gsig = g + 1;
    leaf #(.W(4)) u_l (.clk(clk), .d(gsig), .q());
  end
  for (genvar k = 0; k < 2; k++) begin
    logic ksig;
  end
  if (N > 1) begin : gif
    logic gi;
  end else begin : gif
    logic gother;
  end
  case (N)
    2: begin : gc2 logic c2; end
    default: begin : gcd logic cd; end
  endcase
  assign #2 w = lv[3:0];
  and g_and (a_out, lv[0], clk);
  udp_and u_udp (u_out, lv[1], lv[2]);
  nmos n1 (sw_out, lv[3], clk);
  function int add(input int a, input int b); return a + b; endfunction
  task automatic tick(input int n, output int m); m = n; endtask
  initial begin : init_blk
    int local_i;
    local_i = 3;
    pkg_count = 11;
    mem[1] = 8'h42; dyn = new[3]; dyn[2] = 7; qu = '{4, 5}; aa["k"] = 1; m2[1][2] = 4'h9;
    fork : fk
      #1 li = 1;
    join_none
    #3 $walk;
    $finish;
  end
  always @(posedge clk) begin : ablk
    si = si + 1;
  end
  always_comb iv = add(1, 2);
  always_latch if (clk) by = 1;
  always #1 clk = ~clk;
  final $display("done");
endmodule
bind dff_chk cell_buf u_bound (.y(), .a(d));
"###;

/// Scopes: instances of every kind (a `bind`-added one included), generate
/// scopes and arrays, named blocks, tasks and functions, packages, modports;
/// their file, line, definition and relations; and names resolved
/// absolutely, relatively and in packages.
const SCOPES_C: &str = r###"#include "common.h"
static void scope_line(vpiHandle s) {
    vpi_printf("OM|scope %s type=%s name=%s def=%s file=%s line=%d deffile=%s defline=%d top=%d cell=%d\n",
               fname(s), tname(s), S(vpi_get_str(vpiName, s)), S(vpi_get_str(vpiDefName, s)),
               base(vpi_get_str(vpiFile, s)), vpi_get(vpiLineNo, s), base(vpi_get_str(vpiDefFile, s)),
               vpi_get(vpiDefLineNo, s), vpi_get(vpiTopModule, s), vpi_get(vpiCellInstance, s));
}
static void tree(vpiHandle s) {
    scope_line(s);
    vpiHandle it = vpi_iterate(vpiInternalScope, s), c;
    while (it && (c = vpi_scan(it))) tree(c);
}
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    list("instances", vpiInstance, NULL);
    list("modules", vpiModule, NULL);
    list("packages", vpiPackage, NULL);
    vpiHandle top = H("top");
    list("top.internal", vpiInternalScope, top);
    list("top.modules", vpiModule, top);
    list("top.interfaces", vpiInterface, top);
    list("top.programs", vpiProgram, top);
    list("top.instances", vpiInstance, top);
    list("top.genarrays", vpiGenScopeArray, top);
    list("top.taskfuncs", vpiTaskFunc, top);
    tree(top);
    tree(H("cfg_pkg"));
    list("bif.modports", vpiModport, H("top.bif"));
    vpiHandle mst = H("top.bif.mst");
    vpiHandle it = vpi_iterate(vpiIODecl, mst), io;
    while (it && (io = vpi_scan(it)))
        vpi_printf("OM|modport io %s dir=%d expr=%s:%s parent=%s line=%d\n", fname(io), vpi_get(vpiDirection, io),
                   fname(vpi_handle(vpiExpr, io)), tname(vpi_handle(vpiExpr, io)), fname(vpi_handle(vpiParent, io)),
                   vpi_get(vpiLineNo, io));
    vpiHandle ub = H("top.u_dff.u_bound");
    vpi_printf("OM|bound %s def=%s line=%d scope=%s\n", fname(ub), S(vpi_get_str(vpiDefName, ub)), vpi_get(vpiLineNo, ub),
               fname(vpi_handle(vpiScope, ub)));
    it = vpi_iterate(vpiPort, ub);
    while (it && (io = vpi_scan(it)))
        vpi_printf("OM|bound port %s high=%s\n", fname(io), fname(vpi_handle(vpiHighConn, io)));
    vpiHandle ga = H("top.gl");
    vpi_printf("OM|genarray %s type=%s size=%d line=%d scope=%s\n", fname(ga), tname(ga), vpi_get(vpiSize, ga),
               vpi_get(vpiLineNo, ga), fname(vpi_handle(vpiScope, ga)));
    list("gl.elements", vpiGenScope, ga);
    vpiHandle g1 = vpi_handle_by_index(ga, 1);
    vpi_printf("OM|gl[1] byindex=%s index=%d parent=%s scope=%s arraymember=%d implicit=%d\n", fname(g1),
               ival(vpi_handle(vpiIndex, g1)), fname(vpi_handle(vpiParent, g1)), fname(vpi_handle(vpiScope, g1)),
               vpi_get(vpiArrayMember, g1), vpi_get(vpiImplicitDecl, g1));
    vpi_printf("OM|gl[7] byindex=%s\n", fname(vpi_handle_by_index(ga, 7)));
    vpiHandle gb = H("top.genblk2[0]");
    vpi_printf("OM|genblk2[0] implicit=%d parent=%s\n", vpi_get(vpiImplicitDecl, gb), fname(vpi_handle(vpiParent, gb)));
    vpiHandle gif = H("top.gif");
    vpi_printf("OM|gif arraymember=%d parent=%s\n", vpi_get(vpiArrayMember, gif), fname(vpi_handle(vpiParent, gif)));
    vpiHandle ul = H("top.gl[1].u_l");
    vpi_printf("OM|u_l scope=%s parent=%s module=%s instance=%s\n", fname(vpi_handle(vpiScope, ul)),
               fname(vpi_handle(vpiParent, ul)), fname(vpi_handle(vpiModule, ul)), fname(vpi_handle(vpiInstance, ul)));
    vpiHandle gs = H("top.gl[1].gsig");
    vpi_printf("OM|gsig scope=%s module=%s\n", fname(vpi_handle(vpiScope, gs)), fname(vpi_handle(vpiModule, gs)));
    vpiHandle fk = H("top.init_blk.fk");
    vpi_printf("OM|fk type=%s join=%d scope=%s line=%d\n", tname(fk), vpi_get(vpiJoinType, fk),
               fname(vpi_handle(vpiScope, fk)), vpi_get(vpiLineNo, fk));
    vpiHandle pc = H("cfg_pkg::pkg_count");
    vpi_printf("OM|pkg_count instance=%s scope=%s module=%s\n", fname(vpi_handle(vpiInstance, pc)),
               fname(vpi_handle(vpiScope, pc)), fname(vpi_handle(vpiModule, pc)));
    vpiHandle bd = H("top.bif.data");
    vpi_printf("OM|bif.data scope=%s module=%s instance=%s\n", fname(vpi_handle(vpiScope, bd)),
               fname(vpi_handle(vpiModule, bd)), fname(vpi_handle(vpiInstance, bd)));
    vpi_printf("OM|scope-of-top=%s scope-of-null=%s\n", fname(vpi_handle(vpiScope, top)), fname(vpi_handle(vpiScope, NULL)));
    vpi_printf("OM|rel gsig@gl[0]=%s\n", fname(vpi_handle_by_name("gsig", H("top.gl[0]"))));
    vpi_printf("OM|rel u_l.d@gl[1]=%s\n", fname(vpi_handle_by_name("u_l.d", H("top.gl[1]"))));
    vpi_printf("OM|rel MAGIC@cfg_pkg=%s\n", fname(vpi_handle_by_name("MAGIC", H("cfg_pkg"))));
    vpi_printf("OM|rel local_i@init_blk=%s\n", fname(vpi_handle_by_name("local_i", H("top.init_blk"))));
    const char *names[] = {"top.genblk2[1].ksig", "top.gc2.c2", "top.gcd", "top.gif.gother", "top.nosuch",
                           "cfg_pkg::twice", "cfg_pkg::twice.x", "top.u_cell.b0", "top.g_and", "top.p_inst.pcount", 0};
    for (int i = 0; names[i]; i++) {
        vpiHandle h = H(names[i]);
        vpi_printf("OM|name %s -> %s %s\n", names[i], fname(h), tname(h));
    }
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;

#[test]
fn design_walk_scopes_and_names() {
    let got = walk("vpi_om_scopes", SCOPES_C, DESIGN, Some("top"));
    check(
        &got,
        &[
            "OM|instances=top:vpiModule cfg_pkg:vpiPackage",
            "OM|modules=top:vpiModule",
            "OM|packages=cfg_pkg:vpiPackage",
            "OM|top.internal=bif:vpiInterface u_leaf:vpiModule u_cell:vpiModule u_dff:vpiModule p_inst:vpiProgram gl[0]:vpiGenScope gl[1]:vpiGenScope genblk2[0]:vpiGenScope genblk2[1]:vpiGenScope gif:vpiGenScope gc2:vpiGenScope add:vpiFunction tick:vpiTask init_blk:vpiNamedBegin ablk:vpiNamedBegin",
            "OM|top.modules=u_leaf:vpiModule u_cell:vpiModule u_dff:vpiModule",
            "OM|top.interfaces=bif:vpiInterface",
            "OM|top.programs=p_inst:vpiProgram",
            "OM|top.instances=bif:vpiInterface u_leaf:vpiModule u_cell:vpiModule u_dff:vpiModule p_inst:vpiProgram",
            "OM|top.genarrays=gl:vpiGenScopeArray genblk2:vpiGenScopeArray",
            "OM|top.taskfuncs=add:vpiFunction tick:vpiTask",
            "OM|scope top type=vpiModule name=top def=top file=top.sv line=46 deffile=top.sv defline=46 top=1 cell=0",
            "OM|scope top.bif type=vpiInterface name=bif def=bus_if file=top.sv line=76 deffile=top.sv defline=10 top=-1 cell=0",
            "OM|scope top.u_leaf type=vpiModule name=u_leaf def=leaf file=top.sv line=77 deffile=top.sv defline=31 top=0 cell=0",
            "OM|scope top.u_cell type=vpiModule name=u_cell def=cell_buf file=top.sv line=78 deffile=top.sv defline=24 top=0 cell=1",
            "OM|scope top.u_dff type=vpiModule name=u_dff def=dff_chk file=top.sv line=79 deffile=top.sv defline=34 top=0 cell=0",
            "OM|scope top.u_dff.u_bound type=vpiModule name=u_bound def=cell_buf file=top.sv line=122 deffile=top.sv defline=24 top=0 cell=1",
            "OM|scope top.p_inst type=vpiProgram name=p_inst def=prog file=top.sv line=80 deffile=top.sv defline=42 top=-1 cell=0",
            "OM|scope top.gl[0] type=vpiGenScope name=gl[0] def=- file=top.sv line=81 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.gl[0].u_l type=vpiModule name=u_l def=leaf file=top.sv line=83 deffile=top.sv defline=31 top=0 cell=0",
            "OM|scope top.gl[1] type=vpiGenScope name=gl[1] def=- file=top.sv line=81 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.gl[1].u_l type=vpiModule name=u_l def=leaf file=top.sv line=83 deffile=top.sv defline=31 top=0 cell=0",
            "OM|scope top.genblk2[0] type=vpiGenScope name=genblk2[0] def=- file=top.sv line=85 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.genblk2[1] type=vpiGenScope name=genblk2[1] def=- file=top.sv line=85 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.gif type=vpiGenScope name=gif def=- file=top.sv line=88 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.gc2 type=vpiGenScope name=gc2 def=- file=top.sv line=93 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.add type=vpiFunction name=add def=- file=top.sv line=101 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.tick type=vpiTask name=tick def=- file=top.sv line=102 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.init_blk type=vpiNamedBegin name=init_blk def=- file=top.sv line=103 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.init_blk.fk type=vpiNamedFork name=fk def=- file=top.sv line=108 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope top.ablk type=vpiNamedBegin name=ablk def=- file=top.sv line=114 deffile=- defline=-1 top=-1 cell=-1",
            "OM|scope cfg_pkg:: type=vpiPackage name=cfg_pkg def=cfg_pkg file=top.sv line=2 deffile=top.sv defline=2 top=-1 cell=-1",
            "OM|scope cfg_pkg::twice type=vpiFunction name=twice def=- file=top.sv line=8 deffile=- defline=-1 top=-1 cell=-1",
            "OM|bif.modports=mst:vpiModport",
            "OM|modport io top.bif.mst.data dir=2 expr=top.bif.data:vpiReg parent=top.bif.mst line=13",
            "OM|modport io top.bif.mst.valid dir=2 expr=top.bif.valid:vpiReg parent=top.bif.mst line=13",
            "OM|modport io top.bif.mst.clk dir=1 expr=top.bif.clk:vpiNet parent=top.bif.mst line=13",
            "OM|bound top.u_dff.u_bound def=cell_buf line=122 scope=top.u_dff",
            "OM|bound port top.u_dff.u_bound.y high=NULL",
            "OM|bound port top.u_dff.u_bound.a high=top.u_dff.d",
            "OM|genarray top.gl type=vpiGenScopeArray size=2 line=81 scope=top",
            "OM|gl.elements=gl[0]:vpiGenScope gl[1]:vpiGenScope",
            "OM|gl[1] byindex=top.gl[1] index=1 parent=top.gl scope=top arraymember=1 implicit=0",
            "OM|gl[7] byindex=NULL",
            "OM|genblk2[0] implicit=1 parent=top.genblk2",
            "OM|gif arraymember=0 parent=top",
            "OM|u_l scope=top.gl[1] parent=top.gl[1] module=top instance=top",
            "OM|gsig scope=top.gl[1] module=top",
            "OM|fk type=vpiNamedFork join=1 scope=top.init_blk line=108",
            "OM|pkg_count instance=cfg_pkg:: scope=cfg_pkg:: module=NULL",
            "OM|bif.data scope=top.bif module=NULL instance=top.bif",
            "OM|scope-of-top=NULL scope-of-null=top",
            "OM|rel gsig@gl[0]=top.gl[0].gsig",
            "OM|rel u_l.d@gl[1]=top.gl[1].u_l.d",
            "OM|rel MAGIC@cfg_pkg=cfg_pkg::MAGIC",
            "OM|rel local_i@init_blk=top.init_blk.local_i",
            "OM|name top.genblk2[1].ksig -> top.genblk2[1].ksig vpiReg",
            "OM|name top.gc2.c2 -> top.gc2.c2 vpiReg",
            "OM|name top.gcd -> NULL NULL",
            "OM|name top.gif.gother -> NULL NULL",
            "OM|name top.nosuch -> NULL NULL",
            "OM|name cfg_pkg::twice -> cfg_pkg::twice vpiFunction",
            "OM|name cfg_pkg::twice.x -> cfg_pkg::twice.x vpiIODecl",
            "OM|name top.u_cell.b0 -> top.u_cell.b0 vpiGate",
            "OM|name top.g_and -> top.g_and vpiGate",
            "OM|name top.p_inst.pcount -> top.p_inst.pcount vpiIntVar",
        ],
    );
}

/// Declared objects: the declared vpiType of every kind of variable, net,
/// parameter and named event, their sizes, ranges, typespecs (enum
/// constants, struct members, array element types), struct members, bits,
/// elements of fixed, multi-dimensional, dynamic and queue arrays, values
/// and the properties of each.
const DECLS_C: &str = r###"#include "common.h"
static void var_line(const char *n) {
    vpiHandle h = H(n);
    if (!h) {
        vpi_printf("OM|var %s NULL\n", n);
        return;
    }
    vpiHandle l = vpi_handle(vpiLeftRange, h), r = vpi_handle(vpiRightRange, h), ts = vpi_handle(vpiTypespec, h);
    char rng[64] = "-";
    if (l && r) snprintf(rng, sizeof rng, "[%d:%d]", ival(l), ival(r));
    vpi_printf("OM|var %s type=%s size=%d signed=%d vector=%d scalar=%d array=%d range=%s ts=%s:%s line=%d file=%s\n", n,
               tname(h), vpi_get(vpiSize, h), vpi_get(vpiSigned, h), vpi_get(vpiVector, h), vpi_get(vpiScalar, h),
               vpi_get(vpiArray, h), rng, tname(ts), S(ts ? vpi_get_str(vpiName, ts) : NULL), vpi_get(vpiLineNo, h),
               base(vpi_get_str(vpiFile, h)));
}
static void ranges(const char *tag, vpiHandle h) {
    char buf[256];
    buf[0] = 0;
    vpiHandle it = vpi_iterate(vpiRange, h), r;
    while (it && (r = vpi_scan(it))) {
        char one[64];
        snprintf(one, sizeof one, "%s[%d:%d]/%d", buf[0] ? " " : "", ival(vpi_handle(vpiLeftRange, r)),
                 ival(vpi_handle(vpiRightRange, r)), vpi_get(vpiSize, r));
        strncat(buf, one, sizeof buf - strlen(buf) - 1);
    }
    vpi_printf("OM|ranges %s=%s\n", tag, buf);
}
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    const char *vars[] = {"top.clk", "top.w", "top.lv", "top.b4", "top.i32", "top.iv", "top.by", "top.si",
                          "top.li", "top.rv", "top.tv", "top.s", "top.st", "top.pr", "top.us", "top.un",
                          "top.mem", "top.nets", "top.dyn", "top.qu", "top.aa", "top.CI", "top.ev", "top.wa",
                          "top.t1", "top.N", "top.M", "cfg_pkg::WIDTH", "cfg_pkg::MAGIC", "cfg_pkg::pkg_count",
                          "top.init_blk.local_i", "top.gl[1].gsig", "top.genblk2[1].ksig", "top.u_leaf.d",
                          "top.u_leaf.q", "top.u_leaf.W", 0};
    for (int i = 0; vars[i]; i++) var_line(vars[i]);
    vpiHandle top = H("top");
    list("top.nets", vpiNet, top);
    list("top.netarrays", vpiNetArray, top);
    list("top.regs", vpiReg, top);
    list("top.regarrays", vpiRegArray, top);
    list("top.memories", vpiMemory, top);
    list("top.variables", vpiVariables, top);
    list("top.intvars", vpiIntVar, top);
    list("top.params", vpiParameter, top);
    list("top.events", vpiNamedEvent, top);
    list("cfg_pkg.variables", vpiVariables, H("cfg_pkg"));
    list("cfg_pkg.params", vpiParameter, H("cfg_pkg"));
    list("init_blk.variables", vpiVariables, H("top.init_blk"));
    vpi_printf("OM|nettype w=%d wa=%d t1=%d nets=%d clk=%d resolved(wa)=%d\n", vpi_get(vpiNetType, H("top.w")),
               vpi_get(vpiNetType, H("top.wa")), vpi_get(vpiNetType, H("top.t1")), vpi_get(vpiNetType, H("top.nets")),
               vpi_get(vpiNetType, H("top.clk")), vpi_get(vpiResolvedNetType, H("top.wa")));
    vpi_printf("OM|arraytype mem=%d dyn=%d qu=%d aa=%d nets=%d lv=%d\n", vpi_get(vpiArrayType, H("top.mem")),
               vpi_get(vpiArrayType, H("top.dyn")), vpi_get(vpiArrayType, H("top.qu")), vpi_get(vpiArrayType, H("top.aa")),
               vpi_get(vpiArrayType, H("top.nets")), vpi_get(vpiArrayType, H("top.lv")));
    vpi_printf("OM|ismemory mem=%d nets=%d dyn=%d\n", vpi_get(vpiIsMemory, H("top.mem")), vpi_get(vpiIsMemory, H("top.nets")),
               vpi_get(vpiIsMemory, H("top.dyn")));
    vpi_printf("OM|params N local=%d ctype=%d val=%d; M local=%d val=%d; WIDTH local=%d ctype=%d val=%d; MAGIC local=%d ctype=%d val=%d\n",
               vpi_get(vpiLocalParam, H("top.N")), vpi_get(vpiConstType, H("top.N")), ival(H("top.N")),
               vpi_get(vpiLocalParam, H("top.M")), ival(H("top.M")), vpi_get(vpiLocalParam, H("cfg_pkg::WIDTH")),
               vpi_get(vpiConstType, H("cfg_pkg::WIDTH")), ival(H("cfg_pkg::WIDTH")),
               vpi_get(vpiLocalParam, H("cfg_pkg::MAGIC")), vpi_get(vpiConstType, H("cfg_pkg::MAGIC")),
               ival(H("cfg_pkg::MAGIC")));
    vpi_printf("OM|const CI=%d constvar=%d; automatic local_i=%d li=%d; visibility iv=%d\n", ival(H("top.CI")),
               vpi_get(vpiConstantVariable, H("top.CI")), vpi_get(vpiAutomatic, H("top.init_blk.local_i")),
               vpi_get(vpiAutomatic, H("top.li")), vpi_get(vpiVisibility, H("top.iv")));
    ranges("lv", H("top.lv"));
    ranges("mem", H("top.mem"));
    ranges("nets", H("top.nets"));
    ranges("m2", H("top.m2"));
    vpiHandle m21 = vpi_handle_by_index(H("top.m2"), 1);
    vpiHandle m212 = vpi_handle_by_index(m21, 2);
    vpiHandle m2b = H("top.m2[1][2][3]");
    vpi_printf("OM|m2 size=%d m2[1]=%s:%s:%d m2[1][2]=%s:%s:%d parent=%s bit=%s:%s:%d parent=%s\n", vpi_get(vpiSize, H("top.m2")),
               fname(m21), tname(m21), vpi_get(vpiSize, m21), fname(m212), tname(m212), ival(m212),
               fname(vpi_handle(vpiParent, m212)), fname(m2b), tname(m2b), ival(m2b), fname(vpi_handle(vpiParent, m2b)));
    ranges("dyn", H("top.dyn"));
    ranges("qu", H("top.qu"));
    list("dyn.elements", vpiReg, H("top.dyn"));
    vpi_printf("OM|dyn size=%d dyn[2]=%d qu size=%d qu[1]=%d qu[2]=%s aa size=%d aa.left=%s\n", vpi_get(vpiSize, H("top.dyn")),
               ival(vpi_handle_by_index(H("top.dyn"), 2)), vpi_get(vpiSize, H("top.qu")),
               ival(vpi_handle_by_index(H("top.qu"), 1)), fname(vpi_handle_by_index(H("top.qu"), 2)),
               vpi_get(vpiSize, H("top.aa")), fname(vpi_handle(vpiLeftRange, H("top.aa"))));
    vpiHandle mts = vpi_handle(vpiTypespec, H("top.mem"));
    vpiHandle ets = vpi_handle(vpiElemTypespec, mts);
    vpi_printf("OM|mem typespec=%s size=%d arraytype=%d elem=%s elemsize=%d\n", tname(mts), vpi_get(vpiSize, mts),
               vpi_get(vpiArrayType, mts), tname(ets), vpi_get(vpiSize, ets));
    ranges("mem.ts", mts);
    ranges("mem.elem", ets);
    vpiHandle sts = vpi_handle(vpiTypespec, H("top.st"));
    vpi_printf("OM|st typespec=%s name=%s size=%d base=%s value=%d\n", tname(sts), S(vpi_get_str(vpiName, sts)),
               vpi_get(vpiSize, sts), tname(vpi_handle(vpiBaseTypespec, sts)), ival(H("top.st")));
    vpiHandle it = vpi_iterate(vpiEnumConst, sts), o;
    while (it && (o = vpi_scan(it)))
        vpi_printf("OM|enumconst %s=%d type=%s size=%d parent=%s\n", S(vpi_get_str(vpiName, o)), ival(o), tname(o),
                   vpi_get(vpiSize, o), tname(vpi_handle(vpiParent, o)));
    vpiHandle pts = vpi_handle(vpiTypespec, H("top.pr"));
    vpi_printf("OM|pr typespec=%s name=%s packed=%d size=%d\n", tname(pts), S(vpi_get_str(vpiName, pts)),
               vpi_get(vpiPacked, pts), vpi_get(vpiSize, pts));
    it = vpi_iterate(vpiTypespecMember, pts);
    while (it && (o = vpi_scan(it))) {
        vpiHandle mt = vpi_handle(vpiTypespec, o);
        vpi_printf("OM|tsmember %s type=%s ts=%s size=%d line=%d\n", S(vpi_get_str(vpiName, o)), tname(o), tname(mt),
                   vpi_get(vpiSize, mt), vpi_get(vpiLineNo, o));
    }
    const char *structs[] = {"top.pr", "top.us", "top.un", 0};
    for (int i = 0; structs[i]; i++) {
        it = vpi_iterate(vpiMember, H(structs[i]));
        while (it && (o = vpi_scan(it)))
            vpi_printf("OM|member %s type=%s size=%d value=%d su=%d parent=%s\n", fname(o), tname(o), vpi_get(vpiSize, o),
                       ival(o), vpi_get(vpiStructUnionMember, o), fname(vpi_handle(vpiParent, o)));
    }
    vpi_printf("OM|packed pr=%d us=%d\n", vpi_get(vpiPacked, H("top.pr")), vpi_get(vpiPacked, H("top.us")));
    list("w.bits", vpiBit, H("top.w"));
    vpiHandle w2 = H("top.w[2]");
    vpi_printf("OM|w[2] type=%s value=%d size=%d parent=%s index=%d\n", tname(w2), ival(w2), vpi_get(vpiSize, w2),
               fname(vpi_handle(vpiParent, w2)), ival(vpi_handle(vpiIndex, w2)));
    vpiHandle b4_0 = H("top.b4[0]");
    vpi_printf("OM|b4[0] type=%s\n", tname(b4_0));
    list("mem.elements", vpiReg, H("top.mem"));
    vpiHandle m1 = vpi_handle_by_index(H("top.mem"), 1);
    vpi_printf("OM|mem[1] %s type=%s value=%d arraymember=%d parent=%s\n", fname(m1), tname(m1), ival(m1),
               vpi_get(vpiArrayMember, m1), fname(vpi_handle(vpiParent, m1)));
    list("nets.elements", vpiNet, H("top.nets"));
    vpi_printf("OM|values pr=%d i32=%d st=%d pkg_count=%d pcount=%d w=%d\n", ival(H("top.pr")), ival(H("top.i32")),
               ival(H("top.st")), ival(H("cfg_pkg::pkg_count")), ival(H("top.p_inst.pcount")), ival(H("top.w")));
    s_vpi_value v;
    v.format = vpiRealVal;
    vpi_get_value(H("top.rv"), &v);
    vpi_printf("OM|rv=%g\n", v.value.real);
    v.format = vpiStringVal;
    vpi_get_value(H("top.s"), &v);
    vpi_printf("OM|s=%s size=%d\n", v.value.str, vpi_get(vpiSize, H("top.s")));
    vpiHandle ks = H("top.genblk2[1].ksig");
    v.format = vpiIntVal;
    v.value.integer = 1;
    vpi_put_value(ks, &v, NULL, vpiNoDelay);
    vpi_printf("OM|ksig after put=%d\n", ival(ks));
    vpi_chk_error(NULL);
    int lv = ival(H("top.init_blk.local_i"));
    int e1 = vpi_chk_error(NULL);
    vpi_printf("OM|local_i value=%d chk=%d\n", lv, e1);
    int ev = ival(H("top.ev"));
    int e2 = vpi_chk_error(NULL);
    vpi_printf("OM|ev value=%d chk=%d\n", ev, e2);
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;

#[test]
fn design_walk_declarations_types_values() {
    let got = walk("vpi_om_decls", DECLS_C, DESIGN, Some("top"));
    check(
        &got,
        &[
            "OM|var top.clk type=vpiReg size=1 signed=0 vector=0 scalar=1 array=0 range=- ts=vpiLogicTypespec:- line=50 file=top.sv",
            "OM|var top.w type=vpiNet size=4 signed=0 vector=1 scalar=0 array=0 range=[3:0] ts=vpiLogicTypespec:- line=51 file=top.sv",
            "OM|var top.lv type=vpiReg size=8 signed=0 vector=1 scalar=0 array=0 range=[7:0] ts=vpiLogicTypespec:- line=52 file=top.sv",
            "OM|var top.b4 type=vpiBitVar size=4 signed=0 vector=1 scalar=0 array=0 range=[0:3] ts=vpiBitTypespec:- line=53 file=top.sv",
            "OM|var top.i32 type=vpiIntegerVar size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntegerTypespec:- line=54 file=top.sv",
            "OM|var top.iv type=vpiIntVar size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=55 file=top.sv",
            "OM|var top.by type=vpiByteVar size=8 signed=1 vector=1 scalar=0 array=0 range=[7:0] ts=vpiByteTypespec:- line=56 file=top.sv",
            "OM|var top.si type=vpiShortIntVar size=16 signed=1 vector=1 scalar=0 array=0 range=[15:0] ts=vpiShortIntTypespec:- line=57 file=top.sv",
            "OM|var top.li type=vpiLongIntVar size=64 signed=1 vector=1 scalar=0 array=0 range=[63:0] ts=vpiLongIntTypespec:- line=58 file=top.sv",
            "OM|var top.rv type=vpiRealVar size=64 signed=0 vector=0 scalar=0 array=0 range=- ts=vpiRealTypespec:- line=59 file=top.sv",
            "OM|var top.tv type=vpiTimeVar size=64 signed=0 vector=1 scalar=0 array=0 range=[63:0] ts=vpiTimeTypespec:- line=60 file=top.sv",
            "OM|var top.s type=vpiStringVar size=5 signed=0 vector=0 scalar=0 array=0 range=- ts=vpiStringTypespec:- line=61 file=top.sv",
            "OM|var top.st type=vpiEnumVar size=2 signed=0 vector=1 scalar=0 array=0 range=[1:0] ts=vpiEnumTypespec:state_e line=62 file=top.sv",
            "OM|var top.pr type=vpiStructVar size=8 signed=0 vector=1 scalar=0 array=0 range=[7:0] ts=vpiStructTypespec:pair_t line=63 file=top.sv",
            "OM|var top.us type=vpiStructVar size=36 signed=0 vector=0 scalar=0 array=0 range=- ts=vpiStructTypespec:- line=64 file=top.sv",
            "OM|var top.un type=vpiUnionVar size=8 signed=0 vector=1 scalar=0 array=0 range=[7:0] ts=vpiUnionTypespec:- line=65 file=top.sv",
            "OM|var top.mem type=vpiRegArray size=4 signed=0 vector=0 scalar=0 array=1 range=[0:3] ts=vpiArrayTypespec:- line=66 file=top.sv",
            "OM|var top.nets type=vpiNetArray size=2 signed=0 vector=0 scalar=0 array=1 range=[0:1] ts=vpiArrayTypespec:- line=67 file=top.sv",
            "OM|var top.dyn type=vpiRegArray size=3 signed=1 vector=0 scalar=0 array=1 range=[0:2] ts=vpiArrayTypespec:- line=68 file=top.sv",
            "OM|var top.qu type=vpiRegArray size=2 signed=1 vector=0 scalar=0 array=1 range=[0:1] ts=vpiArrayTypespec:- line=69 file=top.sv",
            "OM|var top.aa type=vpiRegArray size=1 signed=1 vector=0 scalar=0 array=1 range=- ts=vpiArrayTypespec:- line=70 file=top.sv",
            "OM|var top.CI type=vpiIntVar size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=71 file=top.sv",
            "OM|var top.ev type=vpiNamedEvent size=-1 signed=0 vector=0 scalar=0 array=0 range=- ts=vpiEventTypespec:- line=72 file=top.sv",
            "OM|var top.wa type=vpiNet size=1 signed=0 vector=0 scalar=1 array=0 range=- ts=vpiLogicTypespec:- line=74 file=top.sv",
            "OM|var top.t1 type=vpiNet size=1 signed=0 vector=0 scalar=1 array=0 range=- ts=vpiLogicTypespec:- line=75 file=top.sv",
            "OM|var top.N type=vpiParameter size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=48 file=top.sv",
            "OM|var top.M type=vpiParameter size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=49 file=top.sv",
            "OM|var cfg_pkg::WIDTH type=vpiParameter size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=3 file=top.sv",
            "OM|var cfg_pkg::MAGIC type=vpiParameter size=4 signed=0 vector=1 scalar=0 array=0 range=[3:0] ts=vpiLogicTypespec:- line=4 file=top.sv",
            "OM|var cfg_pkg::pkg_count type=vpiIntVar size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=7 file=top.sv",
            "OM|var top.init_blk.local_i type=vpiIntVar size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=104 file=top.sv",
            "OM|var top.gl[1].gsig type=vpiReg size=4 signed=0 vector=1 scalar=0 array=0 range=[3:0] ts=vpiLogicTypespec:- line=82 file=top.sv",
            "OM|var top.genblk2[1].ksig type=vpiReg size=1 signed=0 vector=0 scalar=1 array=0 range=- ts=vpiLogicTypespec:- line=86 file=top.sv",
            "OM|var top.u_leaf.d type=vpiNet size=4 signed=0 vector=1 scalar=0 array=0 range=[3:0] ts=vpiLogicTypespec:- line=31 file=top.sv",
            "OM|var top.u_leaf.q type=vpiReg size=4 signed=0 vector=1 scalar=0 array=0 range=[3:0] ts=vpiLogicTypespec:- line=31 file=top.sv",
            "OM|var top.u_leaf.W type=vpiParameter size=32 signed=1 vector=1 scalar=0 array=0 range=[31:0] ts=vpiIntTypespec:- line=31 file=top.sv",
            "OM|top.nets=w:vpiNet a_out:vpiNet u_out:vpiNet cb_y:vpiNet sw_out:vpiNet wa:vpiNet t1:vpiNet",
            "OM|top.netarrays=nets:vpiNetArray",
            "OM|top.regs=clk:vpiReg lv:vpiReg",
            "OM|top.regarrays=mem:vpiRegArray m2:vpiRegArray dyn:vpiRegArray qu:vpiRegArray aa:vpiRegArray",
            "OM|top.memories=mem:vpiRegArray",
            "OM|top.variables=clk:vpiReg lv:vpiReg b4:vpiBitVar i32:vpiIntegerVar iv:vpiIntVar by:vpiByteVar si:vpiShortIntVar li:vpiLongIntVar rv:vpiRealVar tv:vpiTimeVar s:vpiStringVar st:vpiEnumVar pr:vpiStructVar us:vpiStructVar un:vpiUnionVar mem:vpiRegArray m2:vpiRegArray dyn:vpiRegArray qu:vpiRegArray aa:vpiRegArray CI:vpiIntVar",
            "OM|top.intvars=iv:vpiIntVar CI:vpiIntVar",
            "OM|top.params=N:vpiParameter M:vpiParameter",
            "OM|top.events=ev:vpiNamedEvent",
            "OM|cfg_pkg.variables=pkg_count:vpiIntVar",
            "OM|cfg_pkg.params=WIDTH:vpiParameter MAGIC:vpiParameter",
            "OM|init_blk.variables=local_i:vpiIntVar",
            "OM|nettype w=1 wa=2 t1=6 nets=1 clk=-1 resolved(wa)=2",
            "OM|arraytype mem=1 dyn=2 qu=4 aa=3 nets=1 lv=-1",
            "OM|ismemory mem=1 nets=0 dyn=0",
            "OM|params N local=0 ctype=7 val=2; M local=1 val=3; WIDTH local=0 ctype=7 val=8; MAGIC local=1 ctype=5 val=10",
            "OM|const CI=9 constvar=1; automatic local_i=0 li=0; visibility iv=1",
            "OM|ranges lv=[7:0]/8",
            "OM|ranges mem=[0:3]/4",
            "OM|ranges nets=[0:1]/2",
            "OM|ranges m2=[0:1]/2 [0:2]/3",
            "OM|m2 size=6 m2[1]=top.m2[1]:vpiRegArray:3 m2[1][2]=top.m2[1][2]:vpiReg:9 parent=top.m2[1] bit=top.m2[1][2][3]:vpiRegBit:1 parent=top.m2[1][2]",
            "OM|ranges dyn=[0:2]/3",
            "OM|ranges qu=[0:1]/2",
            "OM|dyn.elements=dyn[0]:vpiIntVar dyn[1]:vpiIntVar dyn[2]:vpiIntVar",
            "OM|dyn size=3 dyn[2]=7 qu size=2 qu[1]=5 qu[2]=NULL aa size=1 aa.left=NULL",
            "OM|mem typespec=vpiArrayTypespec size=4 arraytype=1 elem=vpiLogicTypespec elemsize=8",
            "OM|ranges mem.ts=[0:3]/4",
            "OM|ranges mem.elem=[7:0]/8",
            "OM|st typespec=vpiEnumTypespec name=state_e size=2 base=vpiLogicTypespec value=1",
            "OM|enumconst IDLE=0 type=vpiEnumConst size=2 parent=vpiEnumTypespec",
            "OM|enumconst BUSY=1 type=vpiEnumConst size=2 parent=vpiEnumTypespec",
            "OM|enumconst DONE=3 type=vpiEnumConst size=2 parent=vpiEnumTypespec",
            "OM|pr typespec=vpiStructTypespec name=pair_t packed=1 size=8",
            "OM|tsmember hi type=vpiTypespecMember ts=vpiLogicTypespec size=4 line=6",
            "OM|tsmember lo type=vpiTypespecMember ts=vpiLogicTypespec size=4 line=6",
            "OM|member top.pr.hi type=vpiReg size=4 value=3 su=1 parent=top.pr",
            "OM|member top.pr.lo type=vpiReg size=4 value=12 su=1 parent=top.pr",
            "OM|member top.us.a type=vpiIntVar size=32 value=0 su=1 parent=top.us",
            "OM|member top.us.b type=vpiReg size=4 value=0 su=1 parent=top.us",
            "OM|member top.un.x type=vpiReg size=8 value=0 su=1 parent=top.un",
            "OM|member top.un.y type=vpiReg size=8 value=0 su=1 parent=top.un",
            "OM|packed pr=1 us=0",
            "OM|w.bits=w[3]:vpiNetBit w[2]:vpiNetBit w[1]:vpiNetBit w[0]:vpiNetBit",
            "OM|w[2] type=vpiNetBit value=1 size=1 parent=top.w index=2",
            "OM|b4[0] type=vpiRegBit",
            "OM|mem.elements=mem[0]:vpiReg mem[1]:vpiReg mem[2]:vpiReg mem[3]:vpiReg",
            "OM|mem[1] top.mem[1] type=vpiReg value=66 arraymember=1 parent=top.mem",
            "OM|nets.elements=nets[0]:vpiNet nets[1]:vpiNet",
            "OM|values pr=60 i32=-5 st=1 pkg_count=11 pcount=7 w=5",
            "OM|rv=2.5",
            "OM|s=hello size=5",
            "OM|ksig after put=1",
            "OM|local_i value=-999 chk=3",
            "OM|ev value=-999 chk=3",
        ],
    );
}

/// Tasks and functions with their argument declarations, processes,
/// continuous assignments (lhs, rhs, delay), gate, UDP and switch primitives
/// with their terminals, specify paths, timing checks, and ports with their
/// high and low connections, bits and connected ports.
const BEHAVIOR_C: &str = r###"#include "common.h"
static void port_line(vpiHandle p) {
    vpiHandle hi = vpi_handle(vpiHighConn, p), lo = vpi_handle(vpiLowConn, p);
    vpi_printf("OM|port %s index=%d dir=%d size=%d type=%s high=%s:%s low=%s:%s byname=%d line=%d\n", fname(p),
               vpi_get(vpiPortIndex, p), vpi_get(vpiDirection, p), vpi_get(vpiSize, p), tname(p), fname(hi), tname(hi),
               fname(lo), tname(lo), vpi_get(vpiConnByName, p), vpi_get(vpiLineNo, p));
}
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    vpiHandle top = H("top");
    vpiHandle it, o;
    /* tasks and functions */
    const char *tfs[] = {"top.add", "top.tick", "cfg_pkg::twice", 0};
    for (int i = 0; tfs[i]; i++) {
        vpiHandle tf = H(tfs[i]);
        vpi_printf("OM|tf %s type=%s functype=%d size=%d signed=%d automatic=%d visibility=%d line=%d scope=%s\n",
                   fname(tf), tname(tf), vpi_get(vpiFuncType, tf), vpi_get(vpiSize, tf), vpi_get(vpiSigned, tf),
                   vpi_get(vpiAutomatic, tf), vpi_get(vpiVisibility, tf), vpi_get(vpiLineNo, tf),
                   fname(vpi_handle(vpiScope, tf)));
        it = vpi_iterate(vpiIODecl, tf);
        while (it && (o = vpi_scan(it)))
            vpi_printf("OM|iodecl %s type=%s dir=%d size=%d signed=%d line=%d scope=%s\n", fname(o), tname(o),
                       vpi_get(vpiDirection, o), vpi_get(vpiSize, o), vpi_get(vpiSigned, o), vpi_get(vpiLineNo, o),
                       fname(vpi_handle(vpiScope, o)));
    }
    /* processes */
    it = vpi_iterate(vpiProcess, top);
    while (it && (o = vpi_scan(it)))
        vpi_printf("OM|process %s line=%d alwaystype=%d stmt=%s scope=%s module=%s\n", tname(o), vpi_get(vpiLineNo, o),
                   vpi_get(vpiAlwaysType, o), fname(vpi_handle(vpiStmt, o)), fname(vpi_handle(vpiScope, o)),
                   fname(vpi_handle(vpiModule, o)));
    list("u_leaf.process", vpiProcess, H("top.u_leaf"));
    list("p_inst.process", vpiProcess, H("top.p_inst"));
    /* continuous assignment */
    it = vpi_iterate(vpiContAssign, top);
    while (it && (o = vpi_scan(it))) {
        vpiHandle l = vpi_handle(vpiLhs, o), r = vpi_handle(vpiRhs, o), d = vpi_handle(vpiDelay, o);
        vpi_printf("OM|contassign line=%d size=%d netdecl=%d lhs=%s:%s rhs=%s value=%d parent=%s left=%d right=%d delay=%s:%d:%d\n",
                   vpi_get(vpiLineNo, o), vpi_get(vpiSize, o), vpi_get(vpiNetDeclAssign, o), fname(l), tname(l),
                   tname(r), ival(r), fname(vpi_handle(vpiParent, r)), ival(vpi_handle(vpiLeftRange, r)),
                   ival(vpi_handle(vpiRightRange, r)), tname(d), vpi_get(vpiConstType, d), ival(d));
    }
    it = vpi_iterate(vpiContAssign, H("top.gl[1]"));
    while (it && (o = vpi_scan(it))) {
        vpiHandle l = vpi_handle(vpiLhs, o), r = vpi_handle(vpiRhs, o);
        vpi_printf("OM|gl[1] contassign line=%d lhs=%s:%d rhs=%s value=%d operands:", vpi_get(vpiLineNo, o), fname(l), ival(l),
                   tname(r), ival(r));
        vpiHandle oi = vpi_iterate(vpiOperand, r), op;
        while (oi && (op = vpi_scan(oi))) vpi_printf(" %s:%s=%d", tname(op), S(vpi_get_str(vpiFullName, op)), ival(op));
        vpi_printf(" optype=%d\n", vpi_get(vpiOpType, r));
    }
    /* primitives */
    it = vpi_iterate(vpiPrimitive, top);
    while (it && (o = vpi_scan(it))) {
        vpi_printf("OM|prim %s type=%s def=%s primtype=%d size=%d line=%d\n", fname(o), tname(o),
                   S(vpi_get_str(vpiDefName, o)), vpi_get(vpiPrimType, o), vpi_get(vpiSize, o), vpi_get(vpiLineNo, o));
        vpiHandle ti = vpi_iterate(vpiPrimTerm, o), t;
        while (ti && (t = vpi_scan(ti))) {
            vpiHandle e = vpi_handle(vpiExpr, t);
            vpi_printf("OM|  term %d dir=%d expr=%s:%s parent=%s\n", vpi_get(vpiTermIndex, t), vpi_get(vpiDirection, t),
                       tname(e), e && vpi_get(vpiType, e) == vpiBitSelect ? fname(vpi_handle(vpiParent, e)) : fname(e),
                       fname(vpi_handle(vpiParent, t)));
        }
    }
    list("top.gates", vpiGate, top);
    list("top.udps", vpiUdp, top);
    list("top.switches", vpiSwitch, top);
    vpi_printf("OM|u_udp out=%d\n", ival(vpi_scan(vpi_iterate(vpiPrimTerm, H("top.u_udp")))));
    list("u_cell.prims", vpiPrimitive, H("top.u_cell"));
    /* specify */
    it = vpi_iterate(vpiModPath, H("top.u_cell"));
    while (it && (o = vpi_scan(it))) {
        vpi_printf("OM|modpath line=%d ifnone=%d\n", vpi_get(vpiLineNo, o), vpi_get(vpiModPathHasIfNone, o));
        vpiHandle pi = vpi_iterate(vpiModPathIn, o), t;
        while (pi && (t = vpi_scan(pi)))
            vpi_printf("OM|  in %s dir=%d\n", fname(vpi_handle(vpiExpr, t)), vpi_get(vpiDirection, t));
        pi = vpi_iterate(vpiModPathOut, o);
        while (pi && (t = vpi_scan(pi)))
            vpi_printf("OM|  out %s dir=%d\n", fname(vpi_handle(vpiExpr, t)), vpi_get(vpiDirection, t));
    }
    it = vpi_iterate(vpiTchk, H("top.u_dff"));
    while (it && (o = vpi_scan(it))) {
        vpiHandle r = vpi_handle(vpiTchkRefTerm, o), d = vpi_handle(vpiTchkDataTerm, o);
        vpi_printf("OM|tchk %s type=%d line=%d ref=%s edge=%d data=%s edge=%d notifier=%s\n", S(vpi_get_str(vpiName, o)),
                   vpi_get(vpiTchkType, o), vpi_get(vpiLineNo, o), fname(vpi_handle(vpiExpr, r)), vpi_get(vpiEdge, r),
                   fname(vpi_handle(vpiExpr, d)), vpi_get(vpiEdge, d), fname(vpi_handle(vpiTchkNotifier, o)));
    }
    /* ports */
    const char *insts[] = {"top.u_leaf", "top.bif", "top.gl[0].u_l", "top.u_dff", 0};
    for (int i = 0; insts[i]; i++) {
        it = vpi_iterate(vpiPort, H(insts[i]));
        while (it && (o = vpi_scan(it))) port_line(o);
    }
    it = vpi_iterate(vpiPort, H("top.u_leaf"));
    vpi_scan(it);
    vpiHandle pd = vpi_scan(it);
    vpi_free_object(it);
    it = vpi_iterate(vpiBit, pd);
    while (it && (o = vpi_scan(it)))
        vpi_printf("OM|portbit %s type=%s index=%d dir=%d value=%d\n", fname(o), tname(o), vpi_get(vpiPortIndex, o),
                   vpi_get(vpiDirection, o), ival(o));
    list("w.portinst", vpiPortInst, H("top.w"));
    list("clk.portinst", vpiPortInst, H("top.clk"));
    list("u_leaf.d.ports", vpiPorts, H("top.u_leaf.d"));
    list("top.ports", vpiPort, top);
    /* named event and values that cannot be written */
    vpiHandle proc = vpi_scan(vpi_iterate(vpiProcess, top));
    s_vpi_value v;
    v.format = vpiIntVal;
    v.value.integer = 1;
    vpi_chk_error(NULL);
    vpi_put_value(proc, &v, NULL, vpiNoDelay);
    int e1 = vpi_chk_error(NULL);
    vpi_put_value(H("top.ev"), &v, NULL, vpiNoDelay);
    int e2 = vpi_chk_error(NULL);
    vpi_printf("OM|put process chk=%d put event chk=%d\n", e1, e2);
    vpi_printf("OM|compare %d %d\n", vpi_compare_objects(H("top.gl[1]"), vpi_handle_by_index(H("top.gl"), 1)),
               vpi_compare_objects(H("top.gl[1]"), H("top.gl[0]")));
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;

#[test]
fn design_walk_subroutines_processes_primitives_ports() {
    let got = walk("vpi_om_behavior", BEHAVIOR_C, DESIGN, Some("top"));
    check(
        &got,
        &[
            "OM|tf top.add type=vpiFunction functype=1 size=32 signed=1 automatic=0 visibility=1 line=101 scope=top",
            "OM|iodecl top.add.a type=vpiIODecl dir=1 size=32 signed=1 line=101 scope=top.add",
            "OM|iodecl top.add.b type=vpiIODecl dir=1 size=32 signed=1 line=101 scope=top.add",
            "OM|tf top.tick type=vpiTask functype=-1 size=-1 signed=-1 automatic=1 visibility=1 line=102 scope=top",
            "OM|iodecl top.tick.n type=vpiIODecl dir=1 size=32 signed=1 line=102 scope=top.tick",
            "OM|iodecl top.tick.m type=vpiIODecl dir=2 size=32 signed=1 line=102 scope=top.tick",
            "OM|tf cfg_pkg::twice type=vpiFunction functype=1 size=32 signed=1 automatic=1 visibility=1 line=8 scope=cfg_pkg::",
            "OM|iodecl cfg_pkg::twice.x type=vpiIODecl dir=1 size=32 signed=1 line=8 scope=cfg_pkg::twice",
            "OM|process vpiInitial line=103 alwaystype=-1 stmt=top.init_blk scope=top module=top",
            "OM|process vpiAlways line=114 alwaystype=1 stmt=NULL scope=top module=top",
            "OM|process vpiAlways line=117 alwaystype=2 stmt=NULL scope=top module=top",
            "OM|process vpiAlways line=118 alwaystype=4 stmt=NULL scope=top module=top",
            "OM|process vpiAlways line=119 alwaystype=1 stmt=NULL scope=top module=top",
            "OM|process vpiFinal line=120 alwaystype=-1 stmt=NULL scope=top module=top",
            "OM|u_leaf.process=-:vpiAlways",
            "OM|p_inst.process=-:vpiInitial",
            "OM|contassign line=97 size=4 netdecl=0 lhs=top.w:vpiNet rhs=vpiPartSelect value=5 parent=top.lv left=3 right=0 delay=vpiConstant:1:2",
            "OM|gl[1] contassign line=82 lhs=top.gl[1].gsig:2 rhs=vpiOperation value=2 operands: vpiConstant:-=1 vpiConstant:-=1 optype=24",
            "OM|prim top.g_and type=vpiGate def=and primtype=1 size=2 line=98",
            "OM|  term 0 dir=2 expr=vpiNet:top.a_out parent=top.g_and",
            "OM|  term 1 dir=1 expr=vpiBitSelect:top.lv parent=top.g_and",
            "OM|  term 2 dir=1 expr=vpiReg:top.clk parent=top.g_and",
            "OM|prim top.u_udp type=vpiUdp def=udp_and primtype=28 size=2 line=99",
            "OM|  term 0 dir=2 expr=vpiNet:top.u_out parent=top.u_udp",
            "OM|  term 1 dir=1 expr=vpiBitSelect:top.lv parent=top.u_udp",
            "OM|  term 2 dir=1 expr=vpiBitSelect:top.lv parent=top.u_udp",
            "OM|prim top.n1 type=vpiSwitch def=nmos primtype=13 size=2 line=100",
            "OM|  term 0 dir=2 expr=vpiNet:top.sw_out parent=top.n1",
            "OM|  term 1 dir=1 expr=vpiBitSelect:top.lv parent=top.n1",
            "OM|  term 2 dir=1 expr=vpiReg:top.clk parent=top.n1",
            "OM|top.gates=g_and:vpiGate",
            "OM|top.udps=u_udp:vpiUdp",
            "OM|top.switches=n1:vpiSwitch",
            "OM|u_udp out=0",
            "OM|u_cell.prims=b0:vpiGate",
            "OM|modpath line=27 ifnone=0",
            "OM|  in top.u_cell.a dir=1",
            "OM|  out top.u_cell.y dir=2",
            "OM|tchk $setup type=1 line=38 ref=top.u_dff.clk edge=13 data=top.u_dff.d edge=0 notifier=top.u_dff.notifier",
            "OM|tchk $hold type=2 line=39 ref=top.u_dff.clk edge=13 data=top.u_dff.d edge=0 notifier=top.u_dff.notifier",
            "OM|port top.u_leaf.clk index=0 dir=1 size=1 type=vpiPort high=top.clk:vpiReg low=top.u_leaf.clk:vpiNet byname=1 line=31",
            "OM|port top.u_leaf.d index=1 dir=1 size=4 type=vpiPort high=top.w:vpiNet low=top.u_leaf.d:vpiNet byname=1 line=31",
            "OM|port top.u_leaf.q index=2 dir=2 size=4 type=vpiPort high=NULL:NULL low=top.u_leaf.q:vpiReg byname=1 line=31",
            "OM|port top.bif.clk index=0 dir=1 size=1 type=vpiPort high=top.clk:vpiReg low=top.bif.clk:vpiNet byname=1 line=10",
            "OM|port top.gl[0].u_l.clk index=0 dir=1 size=1 type=vpiPort high=top.clk:vpiReg low=top.gl[0].u_l.clk:vpiNet byname=1 line=31",
            "OM|port top.gl[0].u_l.d index=1 dir=1 size=4 type=vpiPort high=top.gl[0].gsig:vpiReg low=top.gl[0].u_l.d:vpiNet byname=1 line=31",
            "OM|port top.gl[0].u_l.q index=2 dir=2 size=4 type=vpiPort high=NULL:NULL low=top.gl[0].u_l.q:vpiReg byname=1 line=31",
            "OM|port top.u_dff.clk index=0 dir=1 size=1 type=vpiPort high=top.clk:vpiReg low=top.u_dff.clk:vpiNet byname=1 line=34",
            "OM|port top.u_dff.d index=1 dir=1 size=1 type=vpiPort high=-:vpiBitSelect low=top.u_dff.d:vpiNet byname=1 line=34",
            "OM|port top.u_dff.q index=2 dir=2 size=1 type=vpiPort high=NULL:NULL low=top.u_dff.q:vpiReg byname=1 line=34",
            "OM|portbit top.u_leaf.d[3] type=vpiPortBit index=1 dir=1 value=0",
            "OM|portbit top.u_leaf.d[2] type=vpiPortBit index=1 dir=1 value=1",
            "OM|portbit top.u_leaf.d[1] type=vpiPortBit index=1 dir=1 value=0",
            "OM|portbit top.u_leaf.d[0] type=vpiPortBit index=1 dir=1 value=1",
            "OM|w.portinst=d:vpiPort",
            "OM|clk.portinst=clk:vpiPort clk:vpiPort a:vpiPort clk:vpiPort clk:vpiPort clk:vpiPort clk:vpiPort",
            "OM|u_leaf.d.ports=d:vpiPort",
            "OM|top.ports=",
            "OM|put process chk=3 put event chk=3",
            "OM|compare 1 0",
        ],
    );
}

/// Two tops (a module and a program) under the multi-top root, non-ANSI
/// ports completed by a later declaration, ANSI ports inheriting direction
/// and type, an interface (modport) port, an instance array and an
/// implicit net.
const DESIGN2: &str = r###"interface simple_if;
  logic [3:0] v;
  modport m (input v);
endinterface
module nonansi (a, b, y);
  input [3:0] a;
  input b;
  output y;
  reg y;
  wire [3:0] a;
  always @* y = |a & b;
endmodule
module ansi_inh (input [1:0] p1, p2, output logic o1, o2);
  assign o1 = p1[0];
  assign o2 = p2[1];
endmodule
module ifport (simple_if.m sif, input logic en);
  logic r;
  assign r = en & sif.v[0];
endmodule
module top1;
  logic [3:0] a4 = 4'b1010;
  logic b1 = 1, y1, o1, o2, en = 1;
  simple_if sif();
  nonansi u_na (.a(a4), .b(b1), .y(y1));
  ansi_inh u_ai (a4[1:0], a4[3:2], o1, o2);
  ifport u_ip (.sif(sif), .en(en));
  nonansi arr [1:0] (.a(a4), .b(b1), .y());
  assign implicit_w = b1;
  initial #1 $walk;
endmodule
program prog2;
  initial #2 $finish;
endprogram
"###;

const MULTI_C: &str = r###"#include "common.h"
static void ports(const char *inst) {
    vpiHandle it = vpi_iterate(vpiPort, H(inst)), p;
    while (it && (p = vpi_scan(it))) {
        vpiHandle hi = vpi_handle(vpiHighConn, p), lo = vpi_handle(vpiLowConn, p);
        vpi_printf("OM|port %s index=%d dir=%d size=%d porttype=%d high=%s:%s low=%s:%s byname=%d\n", fname(p),
                   vpi_get(vpiPortIndex, p), vpi_get(vpiDirection, p), vpi_get(vpiSize, p), vpi_get(vpiPortType, p),
                   fname(hi), tname(hi), fname(lo), tname(lo), vpi_get(vpiConnByName, p));
    }
}
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    list("instances", vpiInstance, NULL);
    list("modules", vpiModule, NULL);
    list("programs", vpiProgram, NULL);
    list("packages", vpiPackage, NULL);
    vpiHandle t1 = H("top1"), p2 = H("prog2");
    vpi_printf("OM|top1 type=%s topmodule=%d line=%d scope=%s; prog2 type=%s line=%d scope=%s\n", tname(t1),
               vpi_get(vpiTopModule, t1), vpi_get(vpiLineNo, t1), fname(vpi_handle(vpiScope, t1)), tname(p2),
               vpi_get(vpiLineNo, p2), fname(vpi_handle(vpiScope, p2)));
    list("top1.internal", vpiInternalScope, t1);
    list("top1.nets", vpiNet, t1);
    vpiHandle iw = H("top1.implicit_w");
    vpi_printf("OM|implicit_w type=%s implicit=%d nettype=%d value=%d line=%d\n", tname(iw), vpi_get(vpiImplicitDecl, iw),
               vpi_get(vpiNetType, iw), ival(iw), vpi_get(vpiLineNo, iw));
    list("u_na.nets", vpiNet, H("top1.u_na"));
    list("u_na.regs", vpiReg, H("top1.u_na"));
    ports("top1.u_na");
    ports("top1.u_ai");
    ports("top1.u_ip");
    ports("top1.arr[1]");
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;

#[test]
fn design_walk_multi_top_program_and_port_kinds() {
    let got = walk("vpi_om_multi", MULTI_C, DESIGN2, None);
    check(
        &got,
        &[
            "OM|instances=top1:vpiModule prog2:vpiProgram",
            "OM|modules=top1:vpiModule",
            "OM|programs=prog2:vpiProgram",
            "OM|packages=",
            "OM|top1 type=vpiModule topmodule=1 line=21 scope=NULL; prog2 type=vpiProgram line=32 scope=NULL",
            "OM|top1.internal=sif:vpiInterface u_na:vpiModule u_ai:vpiModule u_ip:vpiModule arr[0]:vpiModule arr[1]:vpiModule",
            "OM|top1.nets=implicit_w:vpiNet",
            "OM|implicit_w type=vpiNet implicit=1 nettype=1 value=1 line=0",
            "OM|u_na.nets=a:vpiNet b:vpiNet",
            "OM|u_na.regs=y:vpiReg",
            "OM|port top1.u_na.a index=0 dir=1 size=4 porttype=44 high=top1.a4:vpiReg low=top1.u_na.a:vpiNet byname=1",
            "OM|port top1.u_na.b index=1 dir=1 size=1 porttype=44 high=top1.b1:vpiReg low=top1.u_na.b:vpiNet byname=1",
            "OM|port top1.u_na.y index=2 dir=2 size=1 porttype=44 high=top1.y1:vpiReg low=top1.u_na.y:vpiReg byname=1",
            "OM|port top1.u_ai.p1 index=0 dir=1 size=2 porttype=44 high=-:vpiPartSelect low=top1.u_ai.p1:vpiNet byname=0",
            "OM|port top1.u_ai.p2 index=1 dir=1 size=2 porttype=44 high=-:vpiPartSelect low=top1.u_ai.p2:vpiNet byname=0",
            "OM|port top1.u_ai.o1 index=2 dir=2 size=1 porttype=44 high=top1.o1:vpiReg low=top1.u_ai.o1:vpiReg byname=0",
            "OM|port top1.u_ai.o2 index=3 dir=2 size=1 porttype=44 high=top1.o2:vpiReg low=top1.u_ai.o2:vpiReg byname=0",
            "OM|port top1.u_ip.sif index=0 dir=5 size=-1 porttype=2 high=top1.sif:vpiInterface low=NULL:NULL byname=1",
            "OM|port top1.u_ip.en index=1 dir=1 size=1 porttype=44 high=top1.en:vpiReg low=top1.u_ip.en:vpiNet byname=1",
            "OM|port top1.arr[1].a index=0 dir=1 size=4 porttype=44 high=top1.a4:vpiReg low=top1.arr[1].a:vpiNet byname=1",
            "OM|port top1.arr[1].b index=1 dir=1 size=1 porttype=44 high=top1.b1:vpiReg low=top1.arr[1].b:vpiNet byname=1",
            "OM|port top1.arr[1].y index=2 dir=2 size=1 porttype=44 high=NULL:NULL low=top1.arr[1].y:vpiReg byname=1",
        ],
    );
}

/// §23.11 / §37: indexed binds retain port metadata in the selected scope,
/// even when a child shadows the root constant used in the target path.
#[test]
fn design_walk_indexed_bind_root_parameter() {
    let sv = r###"module probe(input logic [3:0] s); initial #2 $display("T|bound=%0d",s); endmodule
module mem;
    localparam int SEL=0;
    logic [3:0] data=7;
endmodule
module top;
    localparam int SEL=1;
    for(genvar g=0;g<2;g++) begin: bank mem m(); end
    bind top.bank[SEL].m probe bound(.s(data));
    initial begin #1 $walk; $finish; end
endmodule
"###;
    let c = r###"#include "common.h"
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    vpiHandle b = H("top.bank[1].m.bound");
    char name[256], def[256], host[256];
    snprintf(name, sizeof name, "%s", fname(b));
    snprintf(def, sizeof def, "%s", S(vpi_get_str(vpiDefName, b)));
    snprintf(host, sizeof host, "%s", S(vpi_get_str(vpiDefName, H("top.bank[1].m"))));
    vpi_printf("OM|selected %s def=%s decoy=%s line=%d\n", name, def,
               fname(H("top.bank[0].m.bound")), vpi_get(vpiLineNo, b));
    vpiHandle it = vpi_iterate(vpiPort, b);
    vpiHandle port = it ? vpi_scan(it) : NULL;
    vpi_printf("OM|host def=%s port=%s\n", host, S(vpi_get_str(vpiName, port)));
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;
    let got = walk("vpi_om_selected_bind", c, sv, Some("top"));
    assert_eq!(
        got,
        [
            "OM|selected top.bank[1].m.bound def=probe decoy=NULL line=9",
            "OM|host def=mem port=s",
        ]
    );
}

/// §23.11/§37: the source model uses the same resolved bind targets as
/// simulation for lexical constants, effective overrides and interface hosts.
#[test]
fn design_walk_bind_sibling_contexts() {
    let c = r###"#include "common.h"
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    vpiHandle b = H("top.h.bank[1].m.bound");
    char name[256], def[256], host[256];
    snprintf(name, sizeof name, "%s", fname(b));
    snprintf(def, sizeof def, "%s", S(vpi_get_str(vpiDefName, b)));
    snprintf(host, sizeof host, "%s", S(vpi_get_str(vpiDefName, H("top.h.bank[1].m"))));
    vpiHandle it = vpi_iterate(vpiPort, b);
    vpiHandle port = it ? vpi_scan(it) : NULL;
    vpi_printf("OM|selected %s def=%s host=%s port=%s decoy=%d\n", name, def, host,
               S(vpi_get_str(vpiName, port)), H("top.h.bank[0].m.bound") != NULL);
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;
    let prefix =
        "module probe(input logic[3:0] s);endmodule\nmodule mem;logic[3:0]data=7;endmodule\n";
    let host = "module host#(parameter N=2,SEL=1);for(genvar g=0;g<N;g++)begin:bank mem m();end endmodule\n";
    let sources = [
        format!("{prefix}{host}module binder;localparam SEL=1;bind top.h.bank[SEL].m probe bound(.s(data));endmodule module top;localparam SEL=0;host h();binder cfg();initial begin #1;$walk;$finish;end endmodule"),
        format!("{prefix}{host}module top;host h();bind top.h.bank[top.h.SEL].m probe bound(.s(data));initial begin #1;$walk;$finish;end endmodule"),
        format!("{prefix}{host}module top;host#(1) h();defparam h.N=2;bind top.h.bank[1].m probe bound(.s(data));initial begin #1;$walk;$finish;end endmodule"),
        "interface probe(input logic[3:0]s);endinterface interface mem;logic[3:0]data=7;endinterface interface host;for(genvar g=0;g<2;g++)begin:bank mem m();end endinterface module top;host h();bind top.h.bank[1].m probe bound(.s(data));initial begin #1;$walk;$finish;end endmodule".to_string(),
        format!("{prefix}{host}module top;host h();if(1)begin:active bind top.h.bank[1].m probe bound(.s(data));end initial begin #1;$walk;$finish;end endmodule"),
    ];
    for (index, source) in sources.iter().enumerate() {
        let got = walk(&format!("vpi_bind_sibling_{index}"), c, source, Some("top"));
        assert_eq!(
            got,
            ["OM|selected top.h.bank[1].m.bound def=probe host=mem port=s decoy=0"],
            "variant {index}"
        );
    }
}

/// §23.11/§37: equal bind offsets in different files retain both instances
/// and the source location of each declaration.
#[test]
fn design_walk_bind_equal_offsets_across_files() {
    let c = r###"#include "common.h"
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    const char *names[] = {"top.m.a", "top.m.b"};
    for (int i=0; i<2; ++i) {
        vpiHandle b=H(names[i]);
        vpi_printf("OM|%s file=%s line=%d\n", names[i], S(vpi_get_str(vpiFile,b)), vpi_get(vpiLineNo,b));
    }
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;
    let got = walk_sources(
        "vpi_bind_file_offsets",
        c,
        &[
            (
                "top.sv",
                "module mem;endmodule module probe;endmodule module top;mem m();initial begin #1;$walk;$finish;end endmodule",
            ),
            ("first.sv", "bind top.m probe a();\n"),
            ("second.sv", "bind top.m probe b();\n"),
        ],
        Some("top"),
    );
    assert_eq!(
        got,
        [
            "OM|top.m.a file=first.sv line=1",
            "OM|top.m.b file=second.sv line=1"
        ]
    );
}

/// §27.5, §27.6: the object model names generate scopes as elaboration does:
/// a direct nest adds no scope, an implicit name avoids an explicit one.
const SCOPE_TREE_C: &str = r###"#include "common.h"
static void tree(vpiHandle s) {
    vpi_printf("OM|scope %s\n", fname(s));
    vpiHandle it = vpi_iterate(vpiInternalScope, s), c;
    while (it && (c = vpi_scan(it))) tree(c);
}
static PLI_INT32 walk(PLI_BYTE8 *u) {
    (void)u;
    tree(H("top"));
    return 0;
}
static void reg(void) { reg_task("$walk", walk); }
void (*vlog_startup_routines[])(void) = {reg, 0};
"###;

#[test]
fn design_walk_direct_nest_and_padded_scope_names() {
    let got = walk(
        "vpi_om_scope_tree",
        SCOPE_TREE_C,
        r#"module top;
  parameter genblk3 = 0;
  if (1) if (1) begin : d int x; end
  if (0) ; else case (1) 1: begin int y; end endcase
  if (1) begin int z; end
  if (1) begin if (1) begin : n int w; end end
  initial $walk;
endmodule
"#,
        Some("top"),
    );
    check(
        &got,
        &[
            "OM|scope top",
            "OM|scope top.d",
            "OM|scope top.genblk2",
            "OM|scope top.genblk03",
            "OM|scope top.genblk4",
            "OM|scope top.genblk4.n",
        ],
    );
}
