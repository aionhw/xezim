//! §23.3.3: an input port connection is a continuous assignment into the
//! port's own net. xezim substitutes the actual for every read inside the
//! child, so that net only matters to something that names it: a dotted
//! reference, a force, `$monitor`, a waveform dump, VPI, a DPI backdoor.
//! With none of those possible the CLI leaves the net and its connection
//! assign out (`xezim_core::elaborate::set_port_elision`).
//!
//! Each case runs the CLI twice — elision allowed, and with
//! `XEZIM_KEEP_PORTS=1` — and requires identical output plus the concrete
//! values below. `XEZIM_ELIDE_STRICT=1` turns any lookup of a left-out net
//! into a panic, so a by-name reference the analysis missed fails the test.
//! `[ELIDE] N` (under `--verbose`) counts the nets left out.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xezim_elide_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Run {
    stdout: String,
    stderr: String,
}

fn run_in(dir: &Path, src: &str, args: &[&str], env: &[(&str, &str)]) -> Run {
    std::fs::write(dir.join("top.sv"), src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--no-cache", "--verbose"])
        .args(args)
        .args(["-s", "top", "top.sv"])
        .current_dir(dir)
        .env("XEZIM_ELIDE_STRICT", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run xezim");
    let r = Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    };
    assert!(
        out.status.success(),
        "xezim failed:\n{}\n{}",
        r.stdout,
        r.stderr
    );
    r
}

/// The design's own output: `$display` lines, without the run banner.
fn user_lines(r: &Run) -> Vec<String> {
    r.stdout
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

fn elided(r: &Run) -> usize {
    r.stderr
        .lines()
        .chain(r.stdout.lines())
        .find_map(|l| l.strip_prefix("[ELIDE] "))
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Run with elision allowed and with it off; the design output must match
/// and carry `expect`. Returns (elided count, output lines).
fn check(tag: &str, src: &str, args: &[&str], expect: &[&str]) -> (usize, Vec<String>) {
    let dir = scratch(tag);
    let on = run_in(&dir, src, args, &[]);
    let off = run_in(&dir, src, args, &[("XEZIM_KEEP_PORTS", "1")]);
    let (a, b) = (user_lines(&on), user_lines(&off));
    assert_eq!(a, b, "elision changed the output");
    assert_eq!(elided(&off), 0, "XEZIM_KEEP_PORTS=1 must keep every port");
    for e in expect {
        assert!(a.iter().any(|l| l == e), "missing `{e}` in {a:#?}");
    }
    (elided(&on), a)
}

/// A bit-cell array: every cell reads rs/wl/we/d through substituted input
/// ports. `we` is a whole-net actual, `rs`/`wl`/`d` are selects.
const CELLS: &str = r#"
module leaf(input rs, input wl, input we, input d, output q_o);
  reg q;
  always @(posedge wl) if (we) q <= d;
  assign q_o = rs & q;
endmodule
module row(input rs, input wl, input we, input [3:0] d, output [3:0] q);
  genvar c;
  generate for (c = 0; c < 4; c = c + 1) begin : u_c
    leaf bitc(.rs(rs), .wl(wl), .we(we), .d(d[c]), .q_o(q[c]));
  end endgenerate
endmodule
module top;
  reg clk = 0, we = 0;
  reg [1:0] a = 0;
  reg [3:0] d = 0;
  wire [3:0] rs, wl;
  wire [3:0] q0, q1, q2, q3;
  genvar r;
  generate for (r = 0; r < 4; r = r + 1) begin : u_r
    assign rs[r] = (a == r);
    assign wl[r] = clk & rs[r];
  end endgenerate
  row r0(.rs(rs[0]), .wl(wl[0]), .we(we), .d(d), .q(q0));
  row r1(.rs(rs[1]), .wl(wl[1]), .we(we), .d(d), .q(q1));
  row r2(.rs(rs[2]), .wl(wl[2]), .we(we), .d(d), .q(q2));
  row r3(.rs(rs[3]), .wl(wl[3]), .we(we), .d(d), .q(q3));
  wire [3:0] rd = q0 | q1 | q2 | q3;
  integer i;
  initial begin
    for (i = 0; i < 4; i = i + 1) begin
      #1 we = 1; a = i; d = 4'h3 + i * 5;
      #1 clk = 1; #1 clk = 0; we = 0;
    end
    for (i = 0; i < 4; i = i + 1) begin
      #1 a = i;
      #1 $display("T|rd %0d %h", i, rd);
    end
    PROBE
  end
endmodule
"#;

fn cells(probe: &str) -> String {
    CELLS.replace("PROBE", probe)
}

const CELL_VALUES: [&str; 4] = ["T|rd 0 3", "T|rd 1 8", "T|rd 2 d", "T|rd 3 2"];

/// Nothing names a port: every leaf's rs/wl/d (16 × 3) plus each row's
/// rs/wl (4 × 2) and the whole-net `we`/`d` chains (4 × 2 + 16) go.
#[test]
fn unobserved_ports_are_left_out() {
    let (n, _) = check("plain", &cells(""), &[], &CELL_VALUES);
    assert_eq!(n, 16 * 4 + 4 * 4, "every substituted input port is elided");
}

/// §23.6: a dotted reference names the port's own net. Its leaf keeps
/// every port of that name; the rest still go.
#[test]
fn dotted_references_keep_their_ports() {
    let probe = r#"$display("T|d %b %b", r0.u_c[2].bitc.d, r3.u_c[0].bitc.d);
    d = 4'b1010;
    #1 $display("T|d %b %b", r0.u_c[1].bitc.d, $root.top.r2.u_c[3].bitc.d);
    $display("T|we %b", r1.u_c[2].bitc.we);"#;
    let (n, _) = check(
        "dotted",
        &cells(probe),
        &[],
        &CELL_VALUES
            .iter()
            .copied()
            .chain(["T|d 0 0", "T|d 1 1", "T|we 0"])
            .collect::<Vec<_>>(),
    );
    // rs and wl remain elidable (16 leaves + 4 rows each).
    assert_eq!(n, 16 * 2 + 4 * 2);
}

/// §10.6.2: force and release through a hierarchical path to the port net.
#[test]
fn force_release_of_a_port_path() {
    let probe = r#"force r1.u_c[0].bitc.d = 1'b1;
    #1 $display("T|forced %b", r1.u_c[0].bitc.d);
    release r1.u_c[0].bitc.d;
    #1 $display("T|released %b", r1.u_c[0].bitc.d);"#;
    check("force", &cells(probe), &[], &["T|forced 1", "T|released 0"]);
}

/// §21.2.3: `$monitor` and `$strobe` of a port path.
#[test]
fn monitor_and_strobe_of_a_port_path() {
    let probe = r#"$monitor("T|mon %b", r2.u_c[1].bitc.d);
    #1 d = 4'b0000;
    #1 d = 4'b0010;
    #1 $strobe("T|strobe %b", r2.u_c[1].bitc.rs);"#;
    check(
        "monitor",
        &cells(probe),
        &[],
        &["T|mon 1", "T|mon 0", "T|strobe 0"],
    );
}

/// §21.7: a waveform dump walks every net under the dumped scope, port
/// nets included — `--wave` turns elision off and the VCD declares them.
#[test]
fn wave_dump_declares_port_nets() {
    let probe = r#"$dumpfile("w.vcd"); $dumpvars(0, top);
    #1 $display("T|dumped");"#;
    let dir = scratch("wave");
    let r = run_in(&dir, &cells(probe), &["--wave"], &[]);
    assert_eq!(elided(&r), 0, "a dump run keeps every port");
    let vcd = std::fs::read_to_string(dir.join("w.vcd")).expect("vcd written");
    let bitc = vcd
        .split("$scope module bitc $end")
        .nth(1)
        .expect("a bitc scope");
    let bitc = bitc.split("$upscope").next().unwrap();
    for p in [" rs $end", " wl $end", " we $end", " d $end"] {
        assert!(bitc.contains(p), "port `{p}` not dumped:\n{bitc}");
    }
}

/// A dump system task in the SOURCES keeps every port even without
/// `--wave`: `$fsdbDumpvars` starts a dump on its own.
#[test]
fn dump_tasks_in_the_sources_keep_ports() {
    let (n, _) = check(
        "fsdb",
        &cells(r#"$fsdbDumpfile("w.fst"); $fsdbDumpvars(0, top);"#),
        &[],
        &CELL_VALUES,
    );
    assert_eq!(n, 0);
    let (n, _) = check("dumpvars", &cells(r#"$dumpvars;"#), &[], &CELL_VALUES);
    assert_eq!(n, 0);
}

/// §35: a DPI import anywhere in the sources may reach nets by string (a
/// backdoor through VPI or the UVM HDL routines): no elision.
#[test]
fn dpi_import_keeps_ports() {
    let src = cells("").replace(
        "module top;",
        "module top;\n  import \"DPI-C\" function int c_unused(input int x);",
    );
    let (n, _) = check("dpi", &src, &[], &CELL_VALUES);
    assert_eq!(n, 0);
}

/// §32: SDF annotation names instance ports.
#[test]
fn sdf_annotate_keeps_ports() {
    let (n, _) = check(
        "sdf",
        &cells(r#"if (0) $sdf_annotate("none.sdf", top);"#),
        &[],
        &CELL_VALUES,
    );
    assert_eq!(n, 0);
}

/// X warnings name the net that went x, port nets included.
#[test]
fn x_warn_keeps_ports() {
    let dir = scratch("xwarn");
    let r = run_in(&dir, &cells(""), &["--x-warn"], &[]);
    assert_eq!(elided(&r), 0);
}

/// §23.8/§21.2.1: `%m` of a child whose only nets were substituted ports
/// still names its scope.
#[test]
fn scope_of_a_child_with_only_input_ports() {
    let src = r#"
module sink(input a, input [3:0] b);
  initial #2 $display("T|%m %b %h", a, b);
endmodule
module top;
  reg x = 1; reg [7:0] y = 8'h5a;
  sink u_s(.a(x), .b(y[6:3]));
  sink u_t(.a(~x), .b(y[3:0]));
endmodule
"#;
    let (n, _) = check("scope", src, &[], &["T|top.u_s 1 b", "T|top.u_t 0 a"]);
    // `.a(~x)` computes, so that port keeps its net (see
    // expression_port_actuals.rs); the three renames go.
    assert_eq!(n, 3);
}

/// A class declared inside the child is not rewritten into the instance, a
/// clocking block falls back to the formal's net for an expression actual:
/// such a child keeps its ports.
#[test]
fn class_or_clocking_block_in_child_keeps_ports() {
    let src = r#"
module kid(input clk, input [3:0] v);
  class Peek;
    function int get(); return 1; endfunction
  endclass
  initial #3 $display("T|kid %h", v);
endmodule
module ckid(input clk, input [3:0] v);
  clocking cb @(posedge clk); input v; endclocking
  initial begin @(cb); #0 $display("T|ckid %h", cb.v); end
endmodule
module top;
  reg clk = 0; reg [3:0] w = 4'h9;
  kid u_k(.clk(clk), .v(w));
  ckid u_c(.clk(clk), .v(w + 4'h1));
  initial #1 clk = 1;
endmodule
"#;
    let (n, _) = check("classclk", src, &[], &["T|kid 9", "T|ckid a"]);
    assert_eq!(n, 0);
}

/// §23.11: a checker bound into the child reads the port by its simple
/// name; the bind is expanded inside the child, where it is substituted.
#[test]
fn bound_checker_reads_the_port() {
    let src = cells("").replace(
        "module top;",
        "module chk(input x, input y);\n  initial #30 $display(\"T|bound %m %b %b\", x, y);\nendmodule\nbind leaf chk u_chk(.x(d), .y(rs));\nmodule top;",
    );
    let (_, lines) = check("bind", &src, &[], &CELL_VALUES);
    assert!(
        lines
            .iter()
            .any(|l| l == "T|bound top.r1.u_c[1].bitc.u_chk 1 0"),
        "{lines:#?}"
    );
}

/// §25: interface instance ports are reached through virtual interfaces by
/// name at run time; interfaces never lose their port nets.
#[test]
fn interface_ports_are_kept() {
    let src = r#"
interface bus_if(input logic clk, input logic [7:0] data);
endinterface
class Mon;
  virtual bus_if vif;
  function new(virtual bus_if v); vif = v; endfunction
  task show(); $display("T|vif %h", vif.data); endtask
endclass
module top;
  logic clk = 0; logic [7:0] w = 8'h3c;
  bus_if u_if(.clk(clk), .data(w ^ 8'hff));
  initial begin
    Mon m = new(u_if);
    #1 m.show();
  end
endmodule
"#;
    check("iface", src, &[], &["T|vif c3"]);
}

/// §23.3.3: a hierarchical continuous assign can drive an input port whose
/// chain above is unconnected; the readers below bind to it.
#[test]
fn hierarchical_assign_into_an_unconnected_port_chain() {
    let src = r#"
module leaf(input en, input d, output reg q);
  always @(posedge en) q <= d;
endmodule
module mid(input en, input d, output q);
  leaf u_l(.en(en), .d(d), .q(q));
endmodule
module top;
  reg d = 1; reg go = 0;
  wire q;
  mid u_m(.d(d), .q(q));
  assign u_m.u_l.en = go;
  initial begin #1 go = 1; #1 $display("T|q %b", q); end
endmodule
"#;
    check("hierassign", src, &[], &["T|q 1"]);
}

/// §38: a VPI library may look any net up by name.
#[test]
fn vpi_library_keeps_ports() {
    const C: &str = r#"
#include <string.h>
#include "vpi_user.h"
static PLI_INT32 peek(PLI_BYTE8 *ud) {
    vpiHandle h = vpi_handle_by_name((PLI_BYTE8 *)"top.r2.u_c[1].bitc.d", NULL);
    if (!h) { vpi_printf("T|vpi none\n"); return 0; }
    s_vpi_value v; v.format = vpiBinStrVal;
    vpi_get_value(h, &v);
    vpi_printf("T|vpi %s\n", v.value.str);
    return 0;
}
static void startup(void) {
    s_vpi_systf_data d; memset(&d, 0, sizeof d);
    d.type = vpiSysTask; d.tfname = (PLI_BYTE8 *)"$peek"; d.calltf = peek;
    vpi_register_systf(&d);
}
void (*vlog_startup_routines[])(void) = { startup, 0 };
"#;
    let dir = scratch("vpi");
    let c = dir.join("peek.c");
    let so = dir.join("peek.so");
    std::fs::write(&c, C).unwrap();
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-I"])
        .arg(&include)
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("cc")
        .success();
    assert!(ok, "cc failed");
    let so = so.to_string_lossy().into_owned();
    let r = run_in(&dir, &cells("$peek;"), &["--vpi-lib", &so], &[]);
    assert_eq!(elided(&r), 0);
    assert!(r.stdout.contains("T|vpi 1"), "{}", r.stdout);
}

/// Library callers look signals up after the run; the API never elides.
#[test]
fn library_api_keeps_ports() {
    let sim = xezim::simulate(&cells(""), 1000).expect("simulate");
    let d = sim
        .get_signal("r2.u_c[1].bitc.d")
        .or_else(|| sim.get_signal("top.r2.u_c[1].bitc.d"))
        .expect("port net present");
    assert_eq!(d.to_u64(), Some(1));
}

/// §23.3.3/§9.4.2: `.wl(wl[1])` into a child whose own port is also named
/// `wl` substitutes the parent-ROOTED `wl[1]` into the child's
/// `@(posedge wl)`. The edge-select rewrite spelled that base under the
/// child's scope — the child's 1-bit port net — and watched its bit 1, so
/// rows 1-3 never latched (values from the reference simulator). Runs
/// through the library API, where no port is ever elided.
#[test]
fn posedge_on_a_rooted_select_actual() {
    let sim = xezim::simulate(&cells(""), 1000).expect("simulate");
    for (row, want) in [(0, 0x3u64), (1, 0x8), (2, 0xd), (3, 0x2)] {
        let mut v = 0u64;
        for c in 0..4 {
            let q = sim
                .get_signal(&format!("r{row}.u_c[{c}].bitc.q"))
                .unwrap_or_else(|| panic!("r{row} cell {c} q"));
            v |= q
                .to_u64()
                .unwrap_or_else(|| panic!("r{row} cell {c} q is x"))
                << c;
        }
        assert_eq!(v, want, "row {row}");
    }
}

/// Processes inside the child — a task, a function, a fork, a string
/// format, a `wait` — all read the substituted actual, never the port net.
#[test]
fn child_subroutines_and_processes_read_the_actual() {
    let src = r#"
module kid(input clk, input [7:0] v, input en);
  function automatic [7:0] twice(input [7:0] x); return x + v; endfunction
  task automatic show(input string tag);
    $display("T|%s %0d %0d %b", tag, v, twice(v), en);
  endtask
  initial begin
    wait (en);
    show("wait");
    fork
      begin @(posedge clk); show("fork"); end
    join
    begin : blk
      integer i; for (i = 0; i < 2; i = i + 1) #1 $display("T|loop %0d %0d", i, v + i);
    end
    $display("T|%s", $sformatf("fmt %h", v));
  end
endmodule
module top;
  reg clk = 0; reg [7:0] a = 8'd20; reg go = 0;
  kid u_k(.clk(clk), .v(a), .en(go));
  initial begin #2 go = 1; #1 a = 8'd7; #1 clk = 1; end
endmodule
"#;
    let (n, _) = check(
        "subr",
        src,
        &[],
        &[
            "T|wait 20 40 1",
            "T|fork 7 14 1",
            "T|loop 0 7",
            "T|loop 1 8",
            "T|fmt 07",
        ],
    );
    assert_eq!(n, 3);
}

/// Code coverage enumerates nets: no elision.
#[test]
fn code_coverage_keeps_ports() {
    let dir = scratch("cov");
    let r = run_in(&dir, &cells(""), &["--code-coverage=toggle"], &[]);
    assert_eq!(elided(&r), 0);
}

/// §5.6.1: an escaped port name is the same identifier as its plain
/// spelling; a dotted reference to it keeps it.
#[test]
fn escaped_port_names() {
    let src = r#"
module kid(input \dat , input \sel );
  initial #1 $display("T|kid %b %b", \dat , sel);
endmodule
module top;
  reg a = 1, b = 0;
  kid u_k(.dat(a), .\sel (b));
  initial #2 $display("T|peek %b", u_k.\dat );
endmodule
"#;
    let (n, _) = check("escaped", src, &[], &["T|kid 1 0", "T|peek 1"]);
    // `dat` is reached through a dotted path; `sel` goes.
    assert_eq!(n, 1);
}

/// The design cache keys on the elision switch: a warm run that dumps must
/// not reuse an elaboration that left port nets out.
#[test]
fn design_cache_does_not_leak_elision_into_a_dump_run() {
    let dir = scratch("cache");
    let cache = dir.join("cache");
    let cache = cache.to_string_lossy().into_owned();
    // Cold run: elision on, and the elaboration is cached.
    let plain = run_in(&dir, &cells(""), &["--cache-dir", &cache], &[]);
    assert!(elided(&plain) > 0);
    // Same sources with --wave: must not come from that cache entry.
    let dumped = run_in(&dir, &cells(""), &["--cache-dir", &cache, "--wave"], &[]);
    assert_eq!(elided(&dumped), 0, "a --wave run must elaborate afresh");
    assert_eq!(user_lines(&plain), user_lines(&dumped));
}

/// A class method, a package function or compilation-unit code runs in the
/// scope of the instance that calls it: a bare name it does not declare
/// itself is looked up under that instance, where it can land on the
/// instance's own port net. Any port named in such code keeps its net.
/// Here the method's `token[15:12]` must read its argument: with the port
/// net gone, the lookup fell through to the testbench's own `token`.
#[test]
fn class_code_names_keep_ports() {
    let src = r#"
package util;
  function automatic logic [3:0] low(input logic [15:0] word);
    return word[3:0];
  endfunction
endpackage
class dec_c;
  function logic [3:0] hi(input logic [15:0] token);
    return token[15:12];
  endfunction
endclass
module ep(input logic clk, input logic [15:0] token, input logic [15:0] word);
  dec_c d = new();
  logic [3:0] r, s;
  always @(posedge clk) begin r = d.hi(token); s = word[3:0]; end
endmodule
module top;
  logic clk = 0;
  logic [1:0][15:0] token, word;
  assign token[0] = 16'hA123; assign token[1] = 16'hB456;
  assign word[0] = 16'h0007; assign word[1] = 16'h0009;
  ep e0(.clk(clk), .token(token[0]), .word(word[0]));
  ep e1(.clk(clk), .token(token[1]), .word(word[1]));
  initial begin
    #1 clk = 1;
    #1 $display("T|%h %h %h %h %h", e0.r, e1.r, e0.s, e1.s, util::low(16'h1234));
  end
endmodule
"#;
    let (n, _) = check("classcode", src, &[], &["T|a b 7 9 4"]);
    // Only the two `clk` ports go; `token` and `word` are named in class
    // and package code.
    assert_eq!(n, 2);
}

/// §29: a UDP instance's terminals are resolved in the enclosing module's
/// own names, not through the port substitution — they read the port net.
/// A module instantiating a UDP keeps its ports.
#[test]
fn udp_terminals_keep_ports() {
    let src = r#"
primitive u_and(out, a, b);
  output out; input a, b;
  table
    1 1 : 1;
    0 ? : 0;
    ? 0 : 0;
  endtable
endprimitive
module cellm(input i0, input i1, output y);
  u_and g(y, i0, i1);
endmodule
module top;
  reg p = 0, q = 1; wire y;
  cellm u(.i0(p), .i1(q), .y(y));
  initial begin #1 p = 1; #1 $display("T|udp %b", y); end
endmodule
"#;
    let (n, _) = check("udp", src, &[], &["T|udp 1"]);
    assert_eq!(n, 0);
}
