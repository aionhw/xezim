//! §23.9, §27.5: a name in a generate block resolves from that block outward. A
//! declaration in a nested or sibling block is not visible by its bare name, so
//! it must not capture a reference to the same name in an outer scope. The
//! reference used to name a `g.W` that does not exist: a width of 1 bit, a
//! value of x, or an "Undeclared identifier" error, by where it was written.

use xezim::{simulate, simulate_multi};

#[track_caller]
fn expect(src: &str, max_time: u64, want: &[(&str, u64)]) {
    let named = simulate_multi(
        &[src.to_string()],
        max_time,
        Some("tb"),
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let found = simulate(src, max_time).expect("simulate failed");
    for (how, sim) in [("named top", named), ("found top", found)] {
        let read = |n: &str| sim.get_signal(n).and_then(|v| v.to_u64());
        for (name, value) in want {
            let got = read(name).or_else(|| read(&format!("tb.{name}")));
            assert_eq!(got, Some(*value), "{name} under a {how}");
        }
    }
}

const NESTED_W: &str = "if (1) begin : h localparam int W = 2; end";

/// `y` reads 8 once `body` has run in block `g`, which also holds a nested `W`.
#[track_caller]
fn sees_the_outer_w(body: &str) {
    let src = format!(
        "module tb;
           localparam int W = 8;
           logic [31:0] y;
           if (1) begin : g
             {body}
             {NESTED_W}
           end
         endmodule"
    );
    expect(&src, 10, &[("y", 8)]);
}

#[test]
fn continuous_assign() {
    sees_the_outer_w("assign y = W;");
}

#[test]
fn initial_block() {
    sees_the_outer_w("initial y = W;");
}

#[test]
fn always_comb_block() {
    sees_the_outer_w("logic [31:0] w = W; always_comb y = w;");
}

#[test]
fn data_declaration_width() {
    sees_the_outer_w("logic [W-1:0] x; assign y = $bits(x);");
}

#[test]
fn parameter_type_width() {
    sees_the_outer_w("localparam logic [W-1:0] P = 8'hA5; assign y = $bits(P);");
}

#[test]
fn parameter_keeps_its_value() {
    let src = format!(
        "module tb;
           localparam int W = 8;
           logic [31:0] y;
           if (1) begin : g
             localparam logic [W-1:0] P = 8'hA5;
             assign y = P;
             {NESTED_W}
           end
         endmodule"
    );
    expect(&src, 10, &[("y", 0xA5)]);
}

#[test]
fn function_body() {
    sees_the_outer_w("function automatic int f(); return W; endfunction initial y = f();");
}

#[test]
fn child_parameter_override() {
    let src = format!(
        "module child #(parameter int A = 0) (output logic [31:0] o);
           assign o = A;
         endmodule
         module tb;
           localparam int W = 8;
           logic [31:0] y;
           if (1) begin : g
             child #(.A(W)) u (.o(y));
             {NESTED_W}
           end
         endmodule"
    );
    expect(&src, 10, &[("y", 8)]);
}

#[test]
fn the_nested_block_is_a_case_arm_or_a_loop() {
    for nested in [
        "case (1) 1: begin : h localparam int W = 2; end endcase",
        "for (genvar j = 0; j < 2; j++) begin : h localparam int W = 2; end",
        "if (1) begin localparam int W = 2; end",
        "if (0) begin : h localparam int W = 2; end",
    ] {
        let src = format!(
            "module tb;
               localparam int W = 8;
               logic [31:0] y;
               if (1) begin : g assign y = W; {nested} end
             endmodule"
        );
        expect(&src, 10, &[("y", 8)]);
    }
}

#[test]
fn the_renamed_block_is_a_loop() {
    let src = format!(
        "module tb;
           localparam int W = 8;
           logic [31:0] y0, y1;
           for (genvar i = 0; i < 2; i++) begin : g
             logic [31:0] t;
             assign t = W + i;
             {NESTED_W}
           end
           assign y0 = g[0].t;
           assign y1 = g[1].t;
         endmodule"
    );
    expect(&src, 10, &[("y0", 8), ("y1", 9)]);
}

#[test]
fn a_sibling_block_sees_the_outer_name() {
    let src = format!(
        "module tb;
           localparam int W = 8;
           logic [31:0] y;
           if (1) begin : g
             {NESTED_W}
             if (1) begin : k assign y = W; end
           end
         endmodule"
    );
    expect(&src, 10, &[("y", 8)]);
}

#[test]
fn every_scope_of_a_shadowed_name_reads_its_own() {
    let src = "module tb;
           localparam int W = 8;
           logic [31:0] a, b, c, d;
           assign a = W;
           if (1) begin : g
             localparam int W = 4;
             assign b = W;
             if (1) begin : h
               localparam int W = 2;
               assign c = W;
             end
             if (1) begin : k assign d = W; end
           end
         endmodule";
    expect(src, 10, &[("a", 8), ("b", 4), ("c", 2), ("d", 4)]);
}

#[test]
fn a_nested_signal_is_reached_through_its_block() {
    let src = "module tb;
           logic [31:0] y, z;
           if (1) begin : g
             if (1) begin : h
               localparam int V = 2;
               logic [31:0] s = 32'h9;
             end
             initial begin #1 y = h.V; z = h.s; end
           end
         endmodule";
    expect(src, 10, &[("y", 2), ("z", 9)]);
}

#[test]
fn an_instance_is_reached_within_and_through_its_block() {
    let src = "module child (output logic [31:0] o);
           logic [31:0] n = 32'h5;
           assign o = n + 1;
         endmodule
         module tb;
           logic [31:0] y, z;
           if (1) begin : g
             if (1) begin : h
               child u (.o(z));
               assign y = u.n;
             end
           end
         endmodule";
    expect(src, 10, &[("y", 5), ("z", 6)]);
}

#[test]
fn a_nested_block_reaches_the_declarations_of_its_ancestors() {
    let src = "module tb;
           logic [31:0] y;
           for (genvar i = 0; i < 2; i++) begin : g
             localparam int X = i + 5;
             logic [31:0] s;
             if (1) begin : h
               localparam int Z = X + 1;
               assign s = Z;
             end
           end
           assign y = g[1].s;
         endmodule";
    expect(src, 10, &[("y", 7)]);
}
