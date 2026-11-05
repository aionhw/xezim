//! §23.9: a name in one definition is not a name of another. The lint's
//! checks must not read the top module's variables or parameters for a
//! child's names, and still reject what is illegal in the child's own scope.

use xezim::simulate;

fn run(src: &str, expected: &str) {
    let sim = simulate(src, 20).unwrap_or_else(|e| panic!("rejected: {e:?}\n{src}"));
    assert!(
        sim.output.iter().any(|o| o.message == expected),
        "{:?}\n{src}",
        sim.output
    );
}

fn reject(src: &str, why: &str) {
    match simulate(src, 20) {
        Ok(_) => panic!("accepted:\n{src}"),
        Err(e) => assert!(format!("{e:?}").contains(why), "{e:?}\n{src}"),
    }
}

#[test]
fn a_variable_is_not_a_queue_of_another_module() {
    // `mirror.sent_q` is a 4-bit variable; only `top.sent_q` is a queue.
    run(
        "module mirror (input logic clk, input logic [3:0] d, output logic [3:0] q);\n\
           logic [3:0] sent_q;\n\
           always_ff @(posedge clk) sent_q <= 4'h0 | d;\n\
           assign q = sent_q;\n\
         endmodule\n\
         module top;\n\
           logic clk = 0;\n\
           always #1 clk = ~clk;\n\
           logic [3:0] y, z;\n\
           logic [7:0] sent_q[$];\n\
           mirror u (.clk(clk), .d(4'h5), .q(y));\n\
           mirror v (.clk(clk), .d(4'h6), .q(z));\n\
           initial begin\n\
             sent_q.push_back(8'h12);\n\
             #4 $display(\"%h %h %0d\", y, z, sent_q.size());\n\
           end\n\
         endmodule\n",
        "5 6 1",
    );
    run(
        "interface bus; logic [3:0] sent_q; assign sent_q = 4'h3 + 4'h1; endinterface\n\
         module top; int sent_q[$]; bus b();\n\
           initial begin sent_q.push_back(1); #1 $display(\"%h %0d\", b.sent_q, sent_q.size()); end\n\
         endmodule\n",
        "4 1",
    );
}

#[test]
fn a_parameter_of_the_top_is_not_one_of_the_child() {
    // Each top `W`, `K` or `N` would fail its check; the child's own is legal.
    let cases = [
        // §11.5.1: a 4-bit indexed part-select.
        (
            "module child #(parameter int W = 4) (input logic [7:0] a, output logic [3:0] o);\n\
               assign o = a[0 +: W];\n\
             endmodule\n",
            "localparam int W = 0;",
            "R|5",
        ),
        // §27.5: `W == 0` is false, so the zero-width branch is not generated.
        (
            "module child #(parameter int W = 4) (input logic [7:0] a, output logic [3:0] o);\n\
               if (W == 0) begin : g assign o = a[0 +: 0]; end\n\
               else begin : g2 assign o = a[3:0]; end\n\
             endmodule\n",
            "localparam int W = 0;",
            "R|5",
        ),
        // §6.19: `B` is 1, not a duplicate of `A`.
        (
            "module child (input logic [7:0] a, output logic [3:0] o);\n\
               localparam int K = 1;\n\
               typedef enum int {A = 0, B = K} e_t;\n\
               assign o = 4'(B) + 4'd4;\n\
             endmodule\n",
            "localparam int K = 0;",
            "R|5",
        ),
        // §7.4: four elements.
        (
            "module child (input logic [7:0] a, output logic [3:0] o);\n\
               localparam int N = 4;\n\
               logic [3:0] m [N];\n\
               assign o = 4'($size(m)) + 4'd1;\n\
             endmodule\n",
            "localparam int N = -1;",
            "R|5",
        ),
        // §7.10: a queue of at most four.
        (
            "module child (input logic [7:0] a, output logic [3:0] o);\n\
               localparam int N = 4;\n\
               int q[$:N];\n\
               assign o = 4'd5;\n\
             endmodule\n",
            "localparam int N = -1;",
            "R|5",
        ),
        // §10.9.1: two entries, two items.
        (
            "module child (input logic [7:0] a, output logic [3:0] o);\n\
               localparam int N = 1;\n\
               int m [0:N] = '{2, 3};\n\
               assign o = 4'(m[0] + m[1]);\n\
             endmodule\n",
            "localparam int N = 3;",
            "R|5",
        ),
        // §6.19: a 4-bit base, so `4'h1` has the base's width.
        (
            "module child (input logic [7:0] a, output logic [3:0] o);\n\
               localparam int W = 4;\n\
               typedef enum logic [W-1:0] {A = 4'h1, B = 4'h5} e_t;\n\
               assign o = B;\n\
             endmodule\n",
            "localparam int W = 8;",
            "R|5",
        ),
        // §11.4.14.3: an 8-bit stream into the child's own 8-bit target.
        (
            "module child (input logic [7:0] a, output logic [3:0] o);\n\
               localparam int W = 8;\n\
               logic [W-1:0] t = {>>{a}};\n\
               assign o = 4'd5;\n\
             endmodule\n",
            "localparam int W = 4;",
            "R|5",
        ),
    ];
    for (child, top_decl, expected) in cases {
        run(
            &format!(
                "{child}module top; {top_decl}\n\
                   logic [7:0] a = 8'hA5; logic [3:0] o;\n\
                   child u(.a(a), .o(o));\n\
                   initial #1 $display(\"R|%h\", o);\n\
                 endmodule\n"
            ),
            expected,
        );
    }
}

#[test]
fn a_package_import_of_a_sibling_does_not_reach_the_child() {
    // `pa::W` is 0, but the child names only its own `W`.
    run(
        "package pa; parameter int W = 0; endpackage\n\
         module child #(parameter int W = 4) (input logic [7:0] a, output logic [3:0] o);\n\
           assign o = a[0 +: W];\n\
         endmodule\n\
         module other; import pa::*; endmodule\n\
         module top; import pa::*; other w();\n\
           logic [7:0] a = 8'hA5; logic [3:0] o;\n\
           child u(.a(a), .o(o));\n\
           initial #1 $display(\"R|%h\", o);\n\
         endmodule\n",
        "R|5",
    );
}

#[test]
fn a_child_s_own_constants_do_not_need_the_top() {
    // §7.10, §6.19, §27.5: the child's `N`, with no `N` in the top.
    for body in [
        "localparam int N = 4; int q[$:N]; initial begin q.push_back(1); #1 $display(\"R|%0d\", q.size()); end",
        "localparam int N = 16; typedef enum {A = $clog2(N), B} e_t; e_t e = B; initial #1 $display(\"R|%0d\", e);",
        "int q[$:pa::P]; initial begin q.push_back(1); #1 $display(\"R|%0d\", q.size()); end",
    ] {
        let expected = if body.contains("$clog2") { "R|5" } else { "R|1" };
        run(
            &format!(
                "package pa; parameter int P = 2; endpackage\n\
                 module child; {body} endmodule\n\
                 module top; child u(); endmodule\n"
            ),
            expected,
        );
    }
}

#[test]
fn a_child_under_a_generate_block_or_beside_another_top_reads_its_own_names() {
    let child = "module child #(parameter int W = 4) (input logic [7:0] a, output logic [3:0] o);\n\
                   assign o = a[0 +: W];\n\
                 endmodule\n";
    let wire = "logic [7:0] a = 8'hA5; logic [3:0] o;";
    let show = "initial #1 $display(\"R|%h\", o);";
    for tops in [
        format!("module top; localparam int W = 0; {wire}\n\
                   case (W) 0: begin : g child u(.a(a), .o(o)); end endcase\n{show}\n\
                 endmodule\n"),
        format!("module top; localparam int W = 0; {wire}\n\
                   if (1) begin : g if (W == 0) begin : h child u(.a(a), .o(o)); end end\n{show}\n\
                 endmodule\n"),
        // Two tops (§23.3.1): neither is instantiated.
        format!("module top; localparam int W = 0; {wire} child u(.a(a), .o(o)); {show} endmodule\n\
                 module other; localparam int W = 0; endmodule\n"),
    ] {
        run(&format!("{child}{tops}"), "R|5");
    }
    // §24.3: a program as the child.
    run(
        "program child #(parameter int W = 4) (input logic [7:0] a);\n\
           initial #1 $display(\"R|%h\", a[0 +: W]);\n\
         endprogram\n\
         module top; localparam int W = 0; logic [7:0] a = 8'hA5; child u(.a(a)); endmodule\n",
        "R|5",
    );
}

#[test]
fn what_is_illegal_in_the_child_s_own_scope_is_still_rejected() {
    // Written with literals or package constants.
    for (body, why) in [
        ("logic [7:0] x; logic [3:0] y; initial y = x[0 +: 0];", "§11.5.1"),
        ("typedef enum int {A = 1, B = 1} e_t;", "same value"),
        ("typedef enum int {A = pa::Z, B = 0} e_t;", "same value"),
        ("int m [0];", "§7.4"),
        ("int q[$:-1];", "§7.10"),
        ("int q[$:pa::M];", "§7.10"),
        ("int m [0:1] = '{1, 2, 3};", "flat ordered"),
        ("int m [0:pa::Z] = '{1, 2, 3};", "flat ordered"),
        ("typedef enum logic [3:0] {A = 8'h1} e_t;", "sized literal"),
    ] {
        reject(
            &format!(
                "package pa; parameter int Z = 0; parameter int M = -1; endpackage\n\
                 module child; {body} endmodule\n\
                 module top; child u(); endmodule\n"
            ),
            why,
        );
    }
    // §7.6: an integral value is not a queue, parenthesized or not.
    for rhs in ["1 + 2", "(1 + 2)"] {
        reject(&format!("module top; int q[$]; initial q = {rhs}; endmodule\n"), "§7.6");
    }
}
