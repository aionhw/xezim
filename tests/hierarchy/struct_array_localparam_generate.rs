//! §6.20.2, §7.2.1, §27.5: an unpacked-array-of-struct `localparam` in a
//! module below the top, read by the module's generate conditions
//! (`if (Info[k].variant == ...)`) and by its continuous assignments
//!, for a `localparam` of the module and for a package
//! parameter.
//!
//! The instance's body parameters are folded before its generate constructs
//! are expanded. An unpacked array's elements were folded with the element
//! type as written, so a struct typedef of the same body, not bound yet,
//! made every element 0; and no element layout was registered under the
//! array's name, so `Info[k].variant` did not resolve in a generate
//! condition even with a package type. Every branch fell to `else`, and the
//! assignments in the others were lost. The fold now inlines the instance's
//! own types, as the scalar `localparam` arm does, and registers the
//! element layout.
//!
//! A package array parameter's elements (`V[k]` after `import p::*`,
//! `import p::V`, or `p::V[k]`) were not visible to an instance's constant
//! evaluation at all: the instance's map holds only its own parameters, the
//! element lookup did not consult the package snapshot the scalar lookup
//! falls back to, and it returned the element default (0). The package pass
//! also overwrote the bare name of a struct array with 0, folding it as a
//! scalar.
//!
//! Expected values are Verilator's (5.046).

use xezim::simulate;

#[test]
fn struct_array_localparam_in_generate_conditions_below_the_top() {
    let got: Vec<String> = simulate(
        r#"
package q;
  typedef struct packed { logic [1:0] variant; logic [5:0] size; } pinfo_t;
endpackage
module ctrl (output logic [3:0] req, output logic [3:0] preq, output logic [1:0] v2, output logic [5:0] s3);
  typedef struct packed { logic [1:0] variant; logic [5:0] size; } info_t;
  localparam info_t Info [4] = '{'{2'd0, 6'd5}, '{2'd1, 6'd6}, '{2'd0, 6'd7}, '{2'd2, 6'd8}};
  localparam q::pinfo_t PInfo [4] = '{'{2'd1, 6'd1}, '{2'd0, 6'd2}, '{2'd1, 6'd3}, '{2'd0, 6'd4}};
  for (genvar k = 0; k < 4; k++) begin : gp
    if (Info[k].variant == 2'd1) begin : gb
      assign req[k] = 1'b1;
    end else if (Info[k].variant == 2'd2) begin : gl
      assign req[k] = 1'b1;
    end else begin : gu
      assign req[k] = 1'b0;
    end
    if (PInfo[k].variant == 2'd1) begin : pb
      assign preq[k] = 1'b1;
    end else begin : pu
      assign preq[k] = 1'b0;
    end
  end
  assign v2 = Info[2].variant ^ Info[3].variant;
  assign s3 = Info[3].size;
endmodule
module top;
  logic [3:0] req, preq; logic [1:0] v2; logic [5:0] s3;
  ctrl dut(.req(req), .preq(preq), .v2(v2), .s3(s3));
  initial begin #1 $display("g13: req=%b preq=%b v2=%0d s3=%0d", req, preq, v2, s3); end
endmodule
"#,
        100,
    )
    .expect("simulate failed")
    .output
    .iter()
    .map(|o| o.message.clone())
    .collect();
    let want = "g13: req=1010 preq=0101 v2=2 s3=8";
    assert!(
        got.iter().any(|l| l == want),
        "missing `{want}` in {got:#?}"
    );
}

fn lines(src: &str) -> Vec<String> {
    simulate(src, 2000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

fn assert_has(src: &str, expected: &[&str]) {
    let got = lines(src);
    for e in expected {
        assert!(got.iter().any(|l| l == e), "missing `{e}` in {got:#?}");
    }
}

/// §26.3: an `int` array parameter of a package, read in a submodule's
/// generate condition through `import p::*`, `p::V[k]` and `import p::V`.
#[test]
fn package_int_array_in_a_submodule_generate() {
    for src in [
        r#"
package p; parameter int V [4] = '{0, 1, 0, 2}; endpackage
module ctrl (output logic [3:0] req);
  import p::*;
  for (genvar k = 0; k < 4; k++) begin : g
    if (V[k] == 1) begin : b1 assign req[k] = 1'b1; end
    else begin : b0 assign req[k] = 1'b0; end
  end
endmodule
module top; logic [3:0] req; ctrl dut(.req(req)); initial #1 $display("pv1: req=%b (expect 0010)", req); endmodule
"#,
        r#"
package p; parameter int V [4] = '{0, 1, 0, 2}; endpackage
module ctrl (output logic [3:0] req);
  for (genvar k = 0; k < 4; k++) begin : g
    if (p::V[k] == 1) begin : b1 assign req[k] = 1'b1; end
    else begin : b0 assign req[k] = 1'b0; end
  end
endmodule
module top; logic [3:0] req; ctrl dut(.req(req)); initial #1 $display("pv2: req=%b (expect 0010)", req); endmodule
"#,
        r#"
package p; parameter int V [4] = '{0, 1, 0, 2}; endpackage
module ctrl (output logic [3:0] req);
  import p::V;
  for (genvar k = 0; k < 4; k++) begin : g
    if (V[k] == 1) begin : b1 assign req[k] = 1'b1; end
    else begin : b0 assign req[k] = 1'b0; end
  end
endmodule
module top; logic [3:0] req; ctrl dut(.req(req)); initial #1 $display("pv3: req=%b (expect 0010)", req); endmodule
"#,
    ] {
        let got = lines(src);
        assert!(
            got.iter().any(|l| l.ends_with("req=0010 (expect 0010)")),
            "{got:#?}"
        );
    }
}

/// §26.3: two submodules that import a same-named array parameter from
/// different packages, one by name and one by `*`, each read their own
/// package's elements.
#[test]
fn same_named_package_arrays_in_two_submodules() {
    for import_q in ["import q::V;", "import q::*;"] {
        let src = r#"
package p; parameter int V [4] = '{0, 1, 0, 2}; endpackage
package q; parameter int V [4] = '{1, 0, 1, 1}; endpackage
module cq (output logic [3:0] req);
  IMPORT_Q
  for (genvar k = 0; k < 4; k++) begin : g
    if (V[k] == 1) begin : b1 assign req[k] = 1'b1; end
    else begin : b0 assign req[k] = 1'b0; end
  end
endmodule
module cp (output logic [3:0] req);
  import p::*;
  for (genvar k = 0; k < 4; k++) begin : g
    if (V[k] == 1) begin : b1 assign req[k] = 1'b1; end
    else begin : b0 assign req[k] = 1'b0; end
  end
endmodule
module top;
  logic [3:0] a, b;
  cq u0(.req(b));
  cp u1(.req(a));
  initial #1 $display("two packages: p=%b q=%b", a, b);
endmodule
"#
        .replace("IMPORT_Q", import_q);
        assert_has(&src, &["two packages: p=0010 q=1101"]);
    }
}

/// §26.3, §23.10: a parameter override is evaluated in the instantiating
/// scope, so `#(.N(V[0]))` reads the parent's imported `V`, not the one the
/// child imports, while the child's own generate condition reads its own.
#[test]
fn override_reads_the_parent_import() {
    for (packages, leaf_import, cond) in [
        ("", "", "N == 3"),
        (
            "package q; parameter int V [2] = '{7, 9}; endpackage",
            "import q::V;",
            "V[1] == 9",
        ),
    ] {
        let src = r#"
package p; parameter int V [2] = '{3, 5}; endpackage
PACKAGES
module leaf #(parameter int N = 0) (output int n, output int m);
  LEAF_IMPORT
  assign n = N;
  if (COND) begin : g9 assign m = 1; end
  else begin : gx assign m = 0; end
endmodule
module mid (output int n, output int m);
  import p::V;
  leaf #(.N(V[0])) u(.n(n), .m(m));
endmodule
module top;
  int n, m;
  mid u(.n(n), .m(m));
  initial #1 $display("override: N=%0d leaf=%0d", n, m);
endmodule
"#
        .replace("PACKAGES", packages)
        .replace("LEAF_IMPORT", leaf_import)
        .replace("COND", cond);
        assert_has(&src, &["override: N=3 leaf=1"]);
    }
}

/// The issue's package struct-array forms: element assignments to a parent
/// vector, downward hierarchical assignments, and the otp_ctrl shape.
#[test]
fn package_struct_array_in_a_submodule_generate() {
    assert_has(
        r#"
`timescale 1ns/1ps
package p;
  typedef enum logic [1:0] {Unbuf, Buf, Lc} variant_e;
  typedef struct packed { variant_e variant; } info_t;
  parameter info_t Info [4] = '{'{Unbuf}, '{Buf}, '{Unbuf}, '{Lc}};
endpackage

module ctrl (output logic [3:0] req);
  import p::*;
  for (genvar k = 0; k < 4; k++) begin : gen_partitions
    if (Info[k].variant == Unbuf) begin : gen_unbuffered
      assign req[k] = 1'b0;
    end else if (Info[k].variant == Buf) begin : gen_buffered
      assign req[k] = 1'b1;
    end else begin : gen_lifecycle
      assign req[k] = 1'b0;
    end
  end
endmodule

module top;
  logic [3:0] req;
  ctrl dut(.req(req));
  initial begin
    #1;
    $display("req = %b (expect 0010) %s", req, req === 4'b0010 ? "PASS" : "FAIL");
    $finish;
  end
endmodule
"#,
        &["req = 0010 (expect 0010) PASS"],
    );
    assert_has(
        r#"
`timescale 1ns/1ps
package p;
  typedef enum logic [1:0] {Unbuf, Buf, Lc} variant_e;
  typedef struct packed { variant_e variant; } info_t;
  parameter info_t Info [3] = '{'{Unbuf}, '{Buf}, '{Lc}};
endpackage

module chk;
  logic flag;
  initial begin
    #1ps;
    $display("%s %m flag=%b", (flag === 1'b1) ? "PASS" : "FAIL", flag);
  end
endmodule

module ctrl;
  import p::*;
  for (genvar k = 0; k < 3; k++) begin : g
    if (Info[k].variant == Unbuf) begin : b_unbuf
      chk u_a();
      assign u_a.flag = 1'b1;
    end else if (Info[k].variant == Buf) begin : b_buf
      chk u_b();
      assign u_b.flag = 1'b1;
    end else begin : b_lc
      chk u_c();
      assign u_c.flag = 1'b1;
    end
  end
endmodule

module top;
  ctrl dut();
  initial #1 $finish;
endmodule
"#,
        &[
            "PASS top.dut.g[0].b_unbuf.u_a flag=1",
            "PASS top.dut.g[1].b_buf.u_b flag=1",
            "PASS top.dut.g[2].b_lc.u_c flag=1",
        ],
    );
    assert_has(
        r#"
`timescale 1ns/1ps
package p;
  typedef enum logic [1:0] {Unbuf, Buf, Lc} variant_e;
  typedef struct packed { variant_e variant; } info_t;
  parameter info_t Info [3] = '{'{Unbuf}, '{Buf}, '{Lc}};
endpackage

module cnt #(parameter bit En = 1);
  logic unused_assert_connected;
  initial begin
    #1ps;
    if (unused_assert_connected === 1'b1 || !En) $display("PASS %m");
    else $display("FAIL %m unused_assert_connected=%b", unused_assert_connected);
  end
endmodule

module part;
  cnt u_cnt();
endmodule

module ctrl;
  import p::*;
  for (genvar k = 0; k < 3; k++) begin : gen_partitions
    if (Info[k].variant == Unbuf) begin : gen_unbuffered
      part u_part_unbuf();
      assign u_part_unbuf.u_cnt.unused_assert_connected = 1'b1;
    end else if (Info[k].variant == Buf) begin : gen_buffered
      part u_part_buf();
      assign u_part_buf.u_cnt.unused_assert_connected = 1'b1;
    end else begin : gen_lifecycle
      part u_part_buf();
      assign u_part_buf.u_cnt.unused_assert_connected = 1'b1;
    end
  end
  part u_dai();
  assign u_dai.u_cnt.unused_assert_connected = 1'b1;
endmodule

module top;
  ctrl dut();
  initial #1 $finish;
endmodule
"#,
        &[
            "PASS top.dut.gen_partitions[0].gen_unbuffered.u_part_unbuf.u_cnt",
            "PASS top.dut.gen_partitions[1].gen_buffered.u_part_buf.u_cnt",
            "PASS top.dut.gen_partitions[2].gen_lifecycle.u_part_buf.u_cnt",
            "PASS top.dut.u_dai.u_cnt",
        ],
    );
}

#[test]
fn explicit_array_import_wins_in_both_declaration_orders() {
    for imports in [
        "import other::*; import chosen::V;",
        "import chosen::V; import other::*;",
    ] {
        let src = r#"
package chosen; parameter int V [2] = '{2, 5}; endpackage
package other; parameter int V [2] = '{7, 1}; endpackage
module child(output wire [1:0] flags);
  IMPORTS
  for (genvar k = 0; k < 2; k++) begin
    if (V[k] == 5) assign flags[k] = 1'b1;
    else assign flags[k] = 1'b0;
  end
endmodule
module top;
  wire [1:0] flags;
  child c(flags);
  initial #1 $display("selected=%b", flags);
endmodule
"#
        .replace("IMPORTS", imports);
        assert_has(&src, &["selected=10"]);
    }
}
