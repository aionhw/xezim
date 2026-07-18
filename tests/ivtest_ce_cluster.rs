//! Ratchet for the compile-error (CE) cluster of the a reference simulator `ivtest` suite:
//! illegal SystemVerilog that xezim must REJECT (non-zero exit / elaboration
//! error) rather than silently accept.
//!
//! Each group pins one shared root cause. The snippets are copied (trimmed)
//! from `ivtest/ivltests/*.v`. A handful of LEGAL companion snippets guard the
//! checks against over-rejection — the neighbouring valid form must still
//! compile.

use xezim::simulate;

/// A `ce` design: elaboration/parse must reject it.
fn reject(name: &str, src: &str) {
    let r = simulate(src, 100_000);
    assert!(
        r.is_err(),
        "{name}: expected xezim to REJECT this illegal form, but it compiled"
    );
}

/// A legal design: must still compile (guards against over-rejection).
fn accept(name: &str, src: &str) {
    let r = simulate(src, 100_000);
    assert!(
        r.is_ok(),
        "{name}: expected xezim to ACCEPT this legal form, but it errored: {:?}",
        r.err()
    );
}

// ---------------------------------------------------------------------------
// A. Enum value-domain rules (§6.19) — value overflow, duplicates, negative,
//    non-constant initializer, illegal name-sequence bounds.
// ---------------------------------------------------------------------------

#[test]
fn enum_value_too_large() {
    reject(
        "pr3366217a",
        "module top; enum bit[4:0] {some[4] = 100} val; endmodule",
    );
}

#[test]
fn enum_negative_value_unsigned_base() {
    reject(
        "pr3366217b",
        "module top; enum bit[1:0] {nega = -1, b, c} val; endmodule",
    );
}

#[test]
fn enum_inferred_overflow() {
    reject(
        "pr3366217c",
        "module top; enum bit[1:0] {a = 3, b, c} val; endmodule",
    );
}

#[test]
fn enum_duplicate_values() {
    reject(
        "pr3366217g",
        "module top; enum {red = 1, green, blue = 2} light; endmodule",
    );
}

#[test]
fn enum_bad_name_sequence_bounds() {
    reject(
        "pr3366217d-x",
        "module top; enum {udef1[1'bx:1]} u; endmodule",
    );
    reject("pr3366217d-zero", "module top; enum {zdef[0]} u; endmodule");
    reject("pr3366217d-neg", "module top; enum {ndef[-1]} u; endmodule");
}

#[test]
fn enum_non_constant_initializer() {
    reject(
        "enum_test6",
        "module top; enum {VAL4, XX4 = $time} en4; endmodule",
    );
}

#[test]
fn enum_value_rules_do_not_over_reject() {
    // §6.19: a SIZED negative literal that fits the base width is legal — its
    // bits wrap into the base (`-4'sd1` == 4'b1111 in a 4-bit base).
    accept(
        "enum_test4",
        "module top; enum bit [3:0] {first, second, third, fourth, last = -4'sd1} t;\n\
         initial $display(\"ok\"); endmodule",
    );
    // A normal, in-range enum must compile.
    accept(
        "enum-legal",
        "module top; enum bit[2:0] {A, B, C=3, D} e; initial $display(\"ok\"); endmodule",
    );
}

// ---------------------------------------------------------------------------
// B. Block / fork end labels (§9.3.4) — an end label must match the name.
// ---------------------------------------------------------------------------

#[test]
fn named_begin_end_label_mismatch() {
    reject(
        "named_begin_fail",
        "module top; initial begin : named_begin\n $display(\"x\");\n end : wrong_name\nendmodule",
    );
}

#[test]
fn named_fork_end_label_mismatch() {
    reject(
        "named_fork_fail",
        "module top; initial fork : named_begin\n $display(\"x\");\n join : wrong_name\nendmodule",
    );
}

#[test]
fn matching_end_label_ok() {
    accept(
        "named_begin_ok",
        "module top; initial begin : nb\n $display(\"ok\");\n end : nb\nendmodule",
    );
}

// ---------------------------------------------------------------------------
// C. Queue bounds (§7.10) — must be a defined, non-negative constant.
// ---------------------------------------------------------------------------

#[test]
fn queue_bound_illegal() {
    reject(
        "sv_queue_vec-neg",
        "module top; int q1 [$:-1]; endmodule",
    );
    reject(
        "sv_queue_vec-undef",
        "module top; int q2 [$:'X]; endmodule",
    );
    reject(
        "sv_queue_vec-nonconst",
        "module top; int bound = 2; int q3 [$:bound]; endmodule",
    );
}

#[test]
fn queue_bound_legal() {
    accept(
        "queue-ok",
        "module top; int q [$]; int qb [$:7]; initial $display(\"ok\"); endmodule",
    );
}

// ---------------------------------------------------------------------------
// D. Wildcard-equality operand type (§11.4.6) — integral operands only.
// ---------------------------------------------------------------------------

#[test]
fn wildcard_cmp_real_operand() {
    reject(
        "wild_cmp_err",
        "module top; parameter weq1 = 2'b01 ==? 0.0; endmodule",
    );
}

#[test]
fn wildcard_cmp_real_string_signal() {
    reject(
        "wild_cmp_err2-real",
        "module top; reg [1:0] rv; real rl; reg res;\n\
         initial res = rl ==? rv; endmodule",
    );
    reject(
        "wild_cmp_err2-string",
        "module top; reg [1:0] rv; string st; reg res;\n\
         initial res = st ==? rv; endmodule",
    );
}

#[test]
fn wildcard_cmp_integral_ok() {
    accept(
        "wild_cmp-ok",
        "module top; reg [1:0] a, b; reg r; initial r = a ==? b; endmodule",
    );
}

// ---------------------------------------------------------------------------
// E. Program-block contents (§24.3) — no always / module instantiation.
// ---------------------------------------------------------------------------

#[test]
fn program_illegal_always() {
    reject(
        "program_hello2",
        "program main (); initial $display(\"x\"); always #1 $finish; endprogram",
    );
}

#[test]
fn program_illegal_module_inst() {
    reject(
        "program5b",
        "module test(input wire foo); initial $display(\"x\", foo); endmodule\n\
         program main; reg foo = 1; test dut(foo); endprogram",
    );
}

// ---------------------------------------------------------------------------
// F. Illegal dimensions (§7.4) — unsized packed `[]`, zero-size unpacked.
// ---------------------------------------------------------------------------

#[test]
fn unsized_packed_dim() {
    reject("br_ml20181012b", "module test(); reg [] illegal; endmodule");
}

#[test]
fn zero_size_unpacked_dim() {
    reject("br_ml20181012d", "module test(); reg illegal[0]; endmodule");
}

// ---------------------------------------------------------------------------
// G. Port connections (§23.3.2) — named/implicit/wildcard must resolve.
// ---------------------------------------------------------------------------

const M_AB: &str = "module m(input a, output b); assign b = a; endmodule\n";

#[test]
fn named_port_not_in_module() {
    reject(
        "implicit-port3",
        &format!("{M_AB} module top; reg a; wire b; wire c; m foo(.a, .b, .c); endmodule"),
    );
}

#[test]
fn implicit_port_missing_signal() {
    reject(
        "implicit-port2",
        &format!("{M_AB} module top; reg a; m foo(.a, .b); endmodule"),
    );
}

#[test]
fn wildcard_port_missing_signal() {
    reject(
        "implicit-port6",
        &format!("{M_AB} module top; reg a; m foo(.*); endmodule"),
    );
}

#[test]
fn port_connections_legal() {
    accept(
        "ports-ok",
        &format!("{M_AB} module top; reg a; wire b; m foo(.a, .b); endmodule"),
    );
    // `.*` where every port has a same-named signal must compile.
    accept(
        "wildcard-ok",
        &format!("{M_AB} module top; reg a; wire b; m foo(.*); endmodule"),
    );
}

// ---------------------------------------------------------------------------
// H. `new[]` array constructor target (§7.5) — dynamic arrays only.
// ---------------------------------------------------------------------------

#[test]
fn new_array_to_fixed_target() {
    reject(
        "sv_new_array_error",
        "module test(); logic [1:0] array = new[4]; endmodule",
    );
}

#[test]
fn new_array_to_dynamic_ok() {
    accept(
        "new-array-ok",
        "module test(); int dyn[] = new[4]; initial $display(\"ok\"); endmodule",
    );
}

// ---------------------------------------------------------------------------
// I. Subroutine output/inout port default (§13.5.3) — not allowed.
// ---------------------------------------------------------------------------

#[test]
fn output_port_default_rejected() {
    reject(
        "sv_port_default14",
        "module test(); integer b;\n\
         task k(input integer i = 0, output integer j = b); j = i; endtask endmodule",
    );
}

// ---------------------------------------------------------------------------
// J. Constructor `super.new(...)` ordering (§8.15) — must be first.
// ---------------------------------------------------------------------------

#[test]
fn super_new_not_first() {
    reject(
        "br_gh390a",
        "package p;\n\
           class base; function new(); endfunction endclass\n\
           class derived extends base;\n\
             function new();\n\
               $display(\"before\");\n\
               super.new();\n\
             endfunction\n\
           endclass\n\
         endpackage",
    );
}

#[test]
fn super_new_first_ok() {
    accept(
        "super-new-ok",
        "package p;\n\
           class base; function new(); endfunction endclass\n\
           class derived extends base;\n\
             function new();\n\
               super.new();\n\
               $display(\"after\");\n\
             endfunction\n\
           endclass\n\
         endpackage\n\
         module top; initial $display(\"ok\"); endmodule",
    );
}
