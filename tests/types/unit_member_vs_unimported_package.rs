//! §26.3, §6.19 — an enum member that `$unit` or a module declares, read in a
//! scope that does not import a package declaring a member of the same name
//! (aionhw/xezim-core#295).
//!
//! A package's names are visible only through an import or a `pkg::` prefix
//! (§26.3), so a bare `EEW8` in a module that imports nothing is the `$unit`
//! enum's member (§3.12.1). xezim registered every package's enum members
//! under their bare names whether or not anything imported the package, and
//! the package registered last won: with the testbench package
//! `typedef enum int {EEW_NONE = 0, EEW1 = 1, EEW8 = 8} eew_e;` in the
//! compilation, the RTL's `$unit` enum `typedef enum logic [2:0]
//! {EEW_NONE, EEW1, EEW8} EEW_e;` read `EEW8` as 8, which a 3-bit `EEW_e`
//! truncates to `EEW_NONE`. The Coral NPU RVV backend's decoder computed
//! `eew_max = EEW_NONE` for every instruction from its UVM testbench and
//! discarded all of them. A module's own enum member read the package's
//! value the same way.
//!
//! Expected values come from the LRM; Verilator 5.054 prints the same lines.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

/// The decoder's shape: a `$unit` enum, an `always_comb` that assigns its
/// members in a child module, and a later package that is never imported.
#[test]
fn unit_member_beats_unimported_package_member() {
    let src = r#"
typedef enum logic [2:0] {EEW_NONE, EEW1, EEW8, EEW16} EEW_e;

module dec(input logic v, input logic [2:0] sew, output EEW_e eew);
  always_comb begin
    eew = EEW_NONE;
    case (1'b1)
      v: case (sew)
           3'd0: eew = EEW8;
           3'd1: eew = EEW16;
         endcase
    endcase
  end
endmodule

package tb_pkg;
  typedef enum int {EEW_NONE = 0, EEW1 = 1, EEW8 = 8, EEW16 = 16} eew_e;
endpackage

module t;
  logic v = 0;
  logic [2:0] sew = 0;
  EEW_e eew;
  EEW_e e;
  dec u(.v(v), .sew(sew), .eew(eew));
  initial begin
    #1 v = 1;
    #1 $display("T|dec %0d %s", eew, eew.name());
    sew = 1;
    #1 $display("T|dec %0d %s", eew, eew.name());
    e = EEW8;
    $display("T|top %0d %0d %s", EEW8, e, e.name());
    case (e) EEW8: $display("T|case EEW8"); default: $display("T|case other"); endcase
    $display("T|pkg %0d %0d", tb_pkg::EEW8, tb_pkg::EEW16);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|dec 2 EEW8",
            "T|dec 3 EEW16",
            "T|top 2 2 EEW8",
            "T|case EEW8",
            "T|pkg 8 16",
        ]
    );
}

/// A module's own enum, with the package first in the source: the module
/// reads its member, the package's constant and function read the package's.
#[test]
fn module_member_beats_unimported_package_member() {
    let src = r#"
package tb_pkg;
  typedef enum int {EEW_NONE = 0, EEW1 = 1, EEW8 = 8} eew_e;
  localparam int PE = EEW8;
  function automatic int f(); return EEW8; endfunction
endpackage

module t;
  typedef enum logic [2:0] {EEW_NONE, EEW1, EEW8} EEW_e;
  EEW_e e;
  initial begin
    e = EEW8;
    #1 $display("T|mod %0d %0d %s pkg %0d %0d", EEW8, e, e.name(), tb_pkg::PE, tb_pkg::f());
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|mod 2 2 EEW8 pkg 8 8"]);
}

/// A module's own enum member also beats one its wildcard import provides
/// (§26.3), while the imported constant keeps the package's value.
#[test]
fn module_member_beats_wildcard_imported_member() {
    let src = r#"
package tb_pkg;
  typedef enum int {EEW_NONE = 0, EEW1 = 1, EEW8 = 8} eew_e;
  localparam int PE = EEW8;
endpackage

module t;
  import tb_pkg::*;
  typedef enum logic [2:0] {EEW_NONE, EEW1, EEW8} EEW_e;
  EEW_e e;
  initial begin
    e = EEW8;
    #1 $display("T|wild %0d %0d %0d", EEW8, e, PE);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|wild 2 2 8"]);
}

/// The package still reads its own member, in a constant, a function and a
/// class method; a module that imports the package reads the package's
/// member while one that does not reads the `$unit` member.
#[test]
fn package_and_importers_keep_the_package_member() {
    let src = r#"
typedef enum logic [2:0] {EEW_NONE, EEW1, EEW8} EEW_e;

package tb_pkg;
  typedef enum int {EEW_NONE = 0, EEW1 = 1, EEW8 = 8} eew_e;
  localparam int PE = EEW8;
  function automatic int f(); return EEW8; endfunction
  class C; function int g(); return EEW8 + PE; endfunction endclass
endpackage

module imp(output int o, output int l);
  import tb_pkg::*;
  localparam int L = EEW8;
  initial o = EEW8;
  assign l = L;
endmodule

module noimp(output int o);
  initial o = EEW8;
endmodule

module t;
  int oi, li, on;
  tb_pkg::C c;
  imp ui(.o(oi), .l(li));
  noimp un(.o(on));
  initial begin
    c = new;
    #1 $display("T|pkg %0d %0d %0d", tb_pkg::PE, tb_pkg::f(), c.g());
    $display("T|imp %0d %0d noimp %0d", oi, li, on);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|pkg 8 8 16", "T|imp 8 8 noimp 2"]);
}

/// A package constant brought in by an explicit import keeps the value its
/// initializer gives in the package, in a localparam of the importer too,
/// while the importer's bare `EEW8` is the `$unit` member.
#[test]
fn explicitly_imported_constant_reads_the_package_member() {
    let src = r#"
typedef enum logic [2:0] {EEW_NONE, EEW1, EEW8} EEW_e;

package tb_pkg;
  typedef enum int {EEW_NONE = 0, EEW1 = 1, EEW8 = 8} eew_e;
  localparam int PE = EEW8;
endpackage

module t;
  import tb_pkg::PE;
  localparam int L = PE;
  int x;
  initial begin
    x = PE;
    #1 $display("T|explicit %0d %0d %0d %0d", L, PE, x, EEW8);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|explicit 8 8 8 2"]);
}

/// Two packages declaring the same member and no `$unit` declaration: each
/// importer reads its own package's member.
#[test]
fn two_packages_same_member_each_importer_reads_its_own() {
    let src = r#"
package p1;
  typedef enum logic [3:0] {A = 3, B = 4} e1_t;
endpackage
package p2;
  typedef enum logic [3:0] {A = 9, C = 10} e2_t;
endpackage
module m1(output int o);
  import p1::*;
  initial o = A;
endmodule
module m2(output int o);
  import p2::*;
  initial o = A;
endmodule
module t;
  int o1, o2;
  m1 u1(.o(o1));
  m2 u2(.o(o2));
  initial #1 $display("T|two %0d %0d %0d %0d %0d %0d", o1, o2, p1::A, p2::A, p1::B, p2::C);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|two 3 9 3 9 4 10"]);
}

/// Only the member the package also declares is put back: the module's own
/// `A` keeps its value next to the `$unit` `B`.
#[test]
fn module_member_keeps_its_value_beside_a_restored_unit_member() {
    let src = r#"
typedef enum logic [2:0] {A = 1, B = 2} U;

package tb_pkg;
  typedef enum int {B = 8} P;
endpackage

module t;
  typedef enum logic [3:0] {A = 5} V;
  V v;
  U u;
  initial begin
    v = A;
    u = B;
    #1 $display("T|mixed %0d %0d %0d %0d %0d", A, B, v, u, $bits(v));
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|mixed 5 2 5 2 4"]);
}

/// §6.19: `A[2]` declares `A0` and `A1`; an un-imported package's `A0` does
/// not replace the generated `$unit` member.
#[test]
fn ranged_unit_member_beats_unimported_package_member() {
    let src = r#"
typedef enum int {A[2] = 1} U;

package tb_pkg;
  typedef enum int {A0 = 8, Z} P;
endpackage

module t;
  U u;
  initial begin
    u = A0;
    #1 $display("T|ranged %0d %0d %0d %s %0d", A0, A1, u, u.name(), tb_pkg::A0);
  end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|ranged 1 2 1 A0 8"]);
}

/// §6.19, §26.3: a `$unit` member initialized from a package parameter that
/// reads one of the package's enum members takes the parameter's value, with
/// and without another package declaring the same member name.
#[test]
fn unit_member_from_a_package_parameter() {
    let with_q = r#"
package p;
  typedef enum int {E = 3} T;
  localparam int K = E;
endpackage

typedef enum int {A = p::K, A2} U;

package q;
  typedef enum int {A = 8} Q;
endpackage

module t;
  U u;
  initial begin
    u = A;
    #1 $display("T|dep %0d %0d %0d %s", A, A2, u, u.name());
  end
endmodule
"#;
    assert_eq!(t_lines(with_q), ["T|dep 3 4 3 A"]);
    let without_q = with_q.replace(
        "package q;\n  typedef enum int {A = 8} Q;\nendpackage\n",
        "",
    );
    assert_eq!(t_lines(&without_q), ["T|dep 3 4 3 A"]);
}
