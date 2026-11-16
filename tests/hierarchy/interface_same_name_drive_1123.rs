//! IEEE 1800-2023 regression and main-passing guards for upstream #1123.
//!
//! A hierarchical continuous drive onto a net of a sub-instance or interface
//! whose internal net has the SAME name as the driving signal (`assign
//! u_same.n = n;` inside `tb` which also has `n`) previously requalified the
//! driver read into the child — a self-read that resolved the net to x once
//! its internal driver also existed. Each driver's RHS must instead evaluate
//! where its assignment is written (§23.6), so the top drive's value flows
//! through and a child z driver stays high-impedance (§23.3.3.7, §28.12).

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct RunDir(std::path::PathBuf);
impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check(source: &str, expected: &[&str], no_warnings: bool) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = RunDir(std::env::temp_dir().join(format!(
        "xezim-1123-{}-{unique}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    std::fs::write(dir.0.join("test.sv"), source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("XEZIM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .current_dir(&dir.0)
        .args(["--no-cache", "test.sv"])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    if no_warnings {
        assert!(!stderr.to_ascii_lowercase().contains("warning"), "{stderr}");
    }
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("T|") || line.starts_with("PASS") || line.starts_with("FAIL")
        })
        .collect();
    // Independent initial blocks may print at the same time; compare the
    // multiset so the test does not impose an Active-region ordering (§4.7).
    let mut actual = lines;
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{stdout}\n{stderr}");
}

/// #1123 main case: same-named drive onto an interface net that has its own
/// internal z driver must carry the driving value, not read x.
#[test]
fn interface_same_name_drive() {
    check(
        r#"interface sub_if;
    wire  n;
    logic val = 1'b0;
    bit   en = 1'b0;
    assign n = en ? val : 1'bz;
endinterface

module tb;
    logic  n = 1'b0;
    int    fails = 0;
    sub_if u_same ();
    assign u_same.n = n;
    task automatic check(string what, logic got, logic exp);
        if (got !== exp) begin
            fails++;
            $display("FAIL %-4s got %b, expected %b", what, got, exp);
        end else begin
            $display("PASS %s", what);
        end
    endtask
    initial begin
        #1;
        check("t1", u_same.n, 1'b0);
        n = 1'b1;
        #1;
        check("t2", u_same.n, 1'b1);
        u_same.en  = 1'b1;
        u_same.val = 1'b1;
        #1;
        check("t3", u_same.n, 1'b1);
        $display("%s", fails ? "FAIL final" : "PASS final");
        $finish;
    end
endmodule
"#,
        &["PASS t1", "PASS t2", "PASS t3", "PASS final"],
        false,
    );
}

/// A differently-named driving signal always worked; guard that it still does.
#[test]
fn keeps_different_name_control() {
    check(
        r#"module leaf;
    wire n;
    assign n = 1'bz;
endmodule

module tb;
    logic q = 1'b1;
    leaf l ();
    assign l.n = q;
    initial begin
        #1;
        if (l.n !== 1'b1) begin
            $display("FAIL got %b, expected 1", l.n);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS"],
        false,
    );
}

/// Interface net with no internal driver, same-named drive — a single
/// cross-scope drive with a name collision must still flow through.
#[test]
fn keeps_no_internal_driver() {
    check(
        r#"interface sub_if;
    wire n;
endinterface

module tb;
    logic n = 1'b0;
    sub_if u_same ();
    assign u_same.n = n;
    initial begin
        #1;
        if (u_same.n !== 1'b0) begin
            $display("FAIL low got %b, expected 0", u_same.n);
        end else begin
            $display("PASS");
        end
        n = 1'b1;
        #1;
        if (u_same.n !== 1'b1) begin
            $display("FAIL high got %b, expected 1", u_same.n);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS", "PASS"],
        false,
    );
}

/// Cross-scope z driver against a same-named top drive: the leaf's internal
/// z stays high-impedance, so the top value wins (§23.3.3.7, §28.12).
#[test]
fn keeps_cross_scope_z_resolution() {
    check(
        r#"module leaf;
    wire n;
    assign n = 1'bz;
endmodule

module tb;
    logic n = 1'b0;
    leaf l ();
    assign l.n = n;
    initial begin
        #1;
        if (l.n !== 1'b0) begin
            $display("FAIL low got %b, expected 0", l.n);
        end else begin
            $display("PASS");
        end
        n = 1'b1;
        #1;
        if (l.n !== 1'b1) begin
            $display("FAIL high got %b, expected 1", l.n);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS", "PASS"],
        false,
    );
}


/// Cousin: a hierarchical-LHS continuous assign inside an inlined sub-module
/// (`assign lp.v = ~v;`) must root its RHS in the writing scope, not resolve
/// `v` into `lp`.
#[test]
fn submodule_hier_rhs_same_name() {
    check(
        r#"module leaf_pkg;
    wire [1:0] v;
endmodule

module mid_mod(input logic [1:0] v);
    leaf_pkg lp();
    assign lp.v = ~v;
endmodule

module tb;
    logic [1:0] v = 2'b01;
    mid_mod mm(.v(v));
    initial begin
        #5;
        if (mm.lp.v !== 2'b10) begin
            $display("FAIL got %b, expected 10", mm.lp.v);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS"],
        false,
    );
}

/// #1095 cousin: a signed input formal bound to an unsigned part-select
/// (`u[31:0]`) must keep the formal's signedness (§23.3.3.8, §11.8.1).
#[test]
fn signed_formal_unsigned_select_port() {
    check(
        r#"module m_signed(input int p, output longint o);
    assign o = p;
endmodule

module tb;
    logic [31:0] u = -5;
    longint o;
    m_signed dut(.p(u[31:0]), .o(o));
    initial begin
        #1;
        if (o !== -64'sd5) begin
            $display("FAIL got %0d, expected -5", o);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS"],
        false,
    );
}
