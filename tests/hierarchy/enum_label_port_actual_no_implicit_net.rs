//! §6.19 + §6.10: an enum label used in a port actual of an
//! instance inside a module (`.din(miss_state == STREAM)`, the enum typedef
//! declared in that module) names a declared constant, not a net.
//!
//! The pass that creates §6.10 implicit nets for undeclared names in an
//! instance's port actuals checked each name against the instantiating
//! module's declarations, but an enum's labels are registered only when that
//! module's items are, after the pass. Every such label got a spurious
//! "implicit 1-bit net created" warning, under `default_nettype none` too,
//! where §22.8 allows no implicit net at all. The value bound was correct.
//!
//! The same candidate list also made a label from a package (`import pk::*`,
//! §26.3) or `$unit` a local name of the module being inlined, so every use
//! of it in that module took the instance path (`m.l.SYS_CSR`), which names
//! nothing: two or more levels down, `op == SYS_CSR` read x there, silently.

use std::process::Command;

/// `{CONN}` is the `ff` instance's connection list, `{PRE}` comes first in
/// the file.
const SRC: &str = r#"{PRE}
module ff (input wire logic clk, input wire logic din, output logic dout);
  always_ff @(posedge clk) dout <= din;
endmodule
module thr (input wire logic clk);
  typedef enum logic [3:0] {IDLE = 4'd0, STREAM = 4'd6} st_t;
  st_t miss_state = STREAM;
  enum logic [1:0] {DA, DB} d = DB;
  logic dout;
  ff u ({CONN});
  if (1) begin : g
    logic dout;
    ff u2 (.clk(clk), .din(miss_state != IDLE && d == DB), .dout(dout));
  end
endmodule
module t;
  logic clk = 0;
  always #5 clk = ~clk;
  thr inst (.clk(clk));
  initial begin
    #22;
    if (inst.dout === 1'b1 && inst.g.dout === 1'b1) $display("PASS");
    else $display("FAIL %b %b", inst.dout, inst.g.dout);
    $finish;
  end
endmodule
"#;

fn run(tag: &str, pre: &str, conn: &str) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_enum_port_actual_{tag}_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("enum_actual.sv");
    std::fs::write(&sv, SRC.replace("{PRE}", pre).replace("{CONN}", conn)).expect("write sv");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_NO_CACHE", "1")
        .args(["-s", "t"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("PASS"),
        "{tag}: stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("implicit") && !stdout.contains("implicit"),
        "{tag}: spurious implicit-net diagnostic:\n{stderr}{stdout}"
    );
}

#[test]
fn enum_label_in_named_connection_is_no_implicit_net() {
    run("named", "", ".clk(clk), .din(miss_state == STREAM), .dout(dout)");
}

#[test]
fn enum_label_beside_implicit_name_connections() {
    run("dotname", "", ".din(miss_state == STREAM), .dout, .clk");
    run("dotstar", "", ".*, .din((miss_state == STREAM))");
}

#[test]
fn enum_label_in_ordered_connection_is_no_implicit_net() {
    run("ordered", "", "clk, miss_state == STREAM, dout");
}

/// §22.8: under `default_nettype none` no implicit net may be created, and
/// a declared label must not be reported as one.
#[test]
fn enum_label_under_default_nettype_none() {
    run(
        "none",
        "`default_nettype none",
        ".clk(clk), .din(miss_state == STREAM), .dout(dout)",
    );
}

/// One leaf per way a label can be visible in the module: a wildcard import
/// in the header and in the body, an explicit import, a qualified name, a
/// module-local typedef and a `$unit` typedef. Each uses its label in a port
/// actual and directly, and must read it there.
const IMPORTED: &str = r#"
package pk;
  typedef enum logic [2:0] { SYS_CSR, SYS_ERET } sys_op_e;
endpackage
typedef enum logic [1:0] { U_A, U_B } u_e;
module probe (input i);
endmodule
`define CHECK(LBL, OTHER) \
  logic [2:0] op = 3'd0; \
  probe p (.i(op == LBL)); \
  initial #1 $display("%s %m direct=%b port=%b", \
    ((op == LBL) === 1'b1 && (op != OTHER) === 1'b1 && p.i === 1'b1) ? "PASS" : "FAIL", \
    op == LBL, p.i);
module leaf_wild_hdr import pk::*; ();
  `CHECK(SYS_CSR, SYS_ERET)
endmodule
module leaf_wild_body;
  import pk::*;
  `CHECK(SYS_CSR, SYS_ERET)
endmodule
module leaf_explicit;
  import pk::SYS_CSR;
  import pk::SYS_ERET;
  `CHECK(SYS_CSR, SYS_ERET)
endmodule
module leaf_qualified;
  `CHECK(pk::SYS_CSR, pk::SYS_ERET)
endmodule
module leaf_local;
  typedef enum logic [2:0] { L_A, L_B } l_e;
  `CHECK(L_A, L_B)
endmodule
module leaf_unit;
  `CHECK(U_A, U_B)
endmodule
module all_leaves;
  leaf_wild_hdr wh(); leaf_wild_body wb(); leaf_explicit ex();
  leaf_qualified q(); leaf_local lo(); leaf_unit un();
endmodule
module depth2; all_leaves a(); endmodule
module depth3; depth2 b(); endmodule
"#;

/// The leaves one, two and three levels below the top.
#[test]
fn imported_enum_label_in_port_actual_reads_at_every_depth() {
    let dir = std::env::temp_dir().join(format!(
        "xezim_enum_port_actual_imported_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("imported.sv");
    std::fs::write(&sv, IMPORTED).expect("write sv");
    for top in ["all_leaves", "depth2", "depth3"] {
        let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .env("XEZIM_NO_CACHE", "1")
            .args(["-s", top])
            .arg(&sv)
            .output()
            .expect("run xezim");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stdout.matches("PASS").count(),
            6,
            "-s {top}: stdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("implicit") && !stdout.contains("implicit"),
            "-s {top}: spurious implicit-net diagnostic:\n{stderr}{stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// §6.10: the net `LINK` that connects a driver to a receiver below the
/// top, two procedural blocks. `{PRE}` comes first in the file, `{BLOCK}`
/// in the wrapper.
const LINK_NET: &str = r#"{PRE}
module drv (output logic o);
  initial #1 o = 1'b1;
endmodule
module rcv (input logic i, output logic seen);
  always @(i) seen = i;
endmodule
module wrap (output logic seen);
  {BLOCK}
  drv d (.o(LINK));
  rcv r (.i(LINK), .seen(seen));
endmodule
module t;
  logic seen;
  wrap w (.seen(seen));
  initial begin
    #5;
    if (seen === 1'b1) $display("PASS");
    else $display("FAIL seen=%b", seen);
    $finish;
  end
endmodule
"#;

fn run_link(tag: &str, pre: &str, block: &str) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_enum_port_actual_link_{tag}_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("link.sv");
    std::fs::write(&sv, LINK_NET.replace("{PRE}", pre).replace("{BLOCK}", block))
        .expect("write sv");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_NO_CACHE", "1")
        .args(["-s", "t"])
        .arg(&sv)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("PASS"), "{tag}: stdout:\n{stdout}\nstderr:\n{stderr}");
}

/// §6.10, §27.5: a label declared in a generate block is the block's, so a
/// port actual of the same name in the module scope is still an implicit
/// net of the module. Taken for a declared constant, the net was not
/// created and the receiver never saw the driver's value.
#[test]
fn generate_block_label_keeps_the_module_implicit_net() {
    run_link(
        "block_label",
        "",
        "if (0) begin : g\n    typedef enum logic {LINK, IDLE} t;\n  end",
    );
}

/// §3.13, §6.10: definitions have a name space of their own, so a module
/// named `LINK` leaves the bare port actual `LINK` an implicit net.
#[test]
fn module_name_keeps_the_implicit_net() {
    run_link("module_name", "module LINK;\nendmodule", "");
}

#[test]
fn package_and_implicit_net_share_a_name_in_port_actuals() {
    let sim = xezim::simulate(
        r#"
package LINK;
  typedef enum bit {OFF = 0, ON = 1} state_t;
endpackage
module driver(output logic o);
  initial begin o = 0; #2 o = 1; end
endmodule
module sink(input logic i, input logic tag);
  initial begin
    #1 $display("actuals=%b%b", i, tag);
    #2 $display("actuals=%b%b", i, tag);
  end
endmodule
module middle;
  driver d(.o(LINK));
  sink s(.i(LINK), .tag(LINK::ON));
endmodule
module top; middle u(); endmodule
"#, 10_000,
    ).expect("simulate actuals");
    let observed: Vec<_> = sim.output.iter().map(|line| line.message.trim())
        .filter(|line| line.starts_with("actuals=")).collect();
    assert_eq!(sim.error_count, 0);
    assert_eq!(observed, ["actuals=01", "actuals=11"]);
}
