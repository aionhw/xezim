//! IEEE 1800-2023 §7.4–§7.7 compile-time legality checks: dimension
//! bounds, slice direction/constness, unpacked array operators, illegal
//! `new[]` targets, and array assignment / formal equivalence rules.

use xezim::simulate;

fn sim_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("legal design should compile and simulate");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn rejected(src: &str) -> String {
    match simulate(src, 100) {
        Ok(sim) => panic!("expected design to be rejected:\n{src}\noutput: {:?}", sim.output),
        Err(e) => e,
    }
}

#[test]
fn rejects_illegal_dimensions_and_slices_and_allows_legal_indexed_and_queue_slices() {
    // Packed dims on non-packed typedef
    rejected("module top; typedef int t; t [3:0] x; endmodule");
    // Unsized packed dimension
    rejected("module top; logic [] x; endmodule");

    // Zero / negative unpacked dimensions across var, net, typedef, local, class
    rejected("module top; int a[0]; endmodule");
    rejected("module top; wire w[0]; endmodule");
    rejected("module top; typedef int z_t[0]; z_t a; endmodule");
    rejected("module top; initial begin int l[-2]; end endmodule");
    rejected("class C; int m[0]; endclass module top; C c; endmodule");

    // Variable-bound unpacked slice and variable-width indexed part-select
    rejected("module top; int a[4]; int b[2]; int i = 0, j = 1; initial b = a[i:j]; endmodule");
    rejected("module top; int a[4]; int b[2]; int w = 2; initial b = a[0 +: w]; endmodule");

    // Reversed unpacked array slices (fixed and dynamic)
    rejected("module top; int a[0:3]; int b[2]; initial b = a[2:1]; endmodule");
    rejected("module top; int d[]; int b[2]; initial begin d = new[4]; b = d[2:1]; end endmodule");

    // Associative array slice
    rejected("module top; int aa[int]; int b[2]; initial b = aa[1:2]; endmodule");

    // Unary and non-equality binary ops on unpacked arrays
    rejected("module top; int a[2] = '{1, 2}; int b[2]; initial b = ~a; endmodule");
    rejected("module top; int a[2] = '{1, 2}; int b[2]; initial b = -a; endmodule");
    rejected("module top; int a[2] = '{1, 2}, b[2] = '{3, 4}, c[2]; initial c = a + b; endmodule");
    rejected("module top; int a[2] = '{1, 2}, b[2] = '{3, 4}; bit r; initial r = (a < b); endmodule");

    // Legal: variable-base constant-width indexed part-select + queue slice
    let out = sim_lines(
        r#"
module top;
  int a[4] = '{10, 20, 30, 40};
  int b[2];
  int q[$] = '{1, 2, 3, 4, 5};
  int qs[$];
  int i = 1, j = 3;
  initial begin
    b = a[i +: 2];
    qs = q[i:j];
    $display("LEGAL|%0d %0d|%0d %0d", b[0], b[1], qs.size(), qs[2]);
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|l| l == "LEGAL|20 30|3 4"),
        "unexpected legal slice output: {out:?}"
    );
}

#[test]
fn rejects_illegal_new_array_targets() {
    rejected("module top; int f[4]; initial f = new[4]; endmodule");
    rejected("module top; int q[$]; initial q = new[4]; endmodule");
    rejected("module top; int aa[int]; initial aa = new[4]; endmodule");
    rejected("module top; int arr3[1][2][]; initial arr3 = new[4]; endmodule");
    rejected("module top; int arr[2][2][]; initial arr[0][1][1] = new[2]; endmodule");
}

#[test]
fn rejects_incompatible_array_assignments_and_formals() {
    // Packed vs unpacked
    rejected("module top; logic [3:0] p; int u[4]; initial p = u; endmodule");
    // Faster-varying dimension mismatch
    rejected("module top; int a[2][3]; int b[2][]; initial begin b[0] = new[3]; b[1] = new[3]; a = b; end endmodule");
    // Base vs derived class array invariance (§7.6)
    rejected(
        "class Base; endclass class Sub extends Base; endclass\n\
         module top; Base ba[2]; Sub da[2]; initial ba = da; endmodule",
    );
    // Associative <-> dynamic mismatch
    rejected("module top; int aa[int]; int d[]; initial d = aa; endmodule");
    rejected("module top; int aa[int]; int d[]; initial aa = d; endmodule");
    // Mismatched associative array key types (including wildcard [*])
    rejected("module top; int a[int]; int b[string]; initial a = b; endmodule");
    rejected("module top; int a[int]; int b[*]; initial a = b; endmodule");
    // Non-equivalent ref formal
    rejected(
        "module top;\n\
           task automatic t(ref int r[4]); r[0] = 1; endtask\n\
           bit [31:0] u[4];\n\
           initial t(u);\n\
         endmodule",
    );
    // Incompatible output/inout formal in call
    rejected(
        "module top;\n\
           task automatic t(output int o[2][3]); o[0][0] = 1; endtask\n\
           int b[2][4];\n\
           initial t(b);\n\
         endmodule",
    );
}
