//! §6.10, §27: an implicit net is declared in the generate block that uses
//! it; each loop iteration has its own, reachable by path (`g[0].n`) from the
//! module and from inside an instance. Expected lines are the reference
//! simulator's.
use xezim::simulate;

/// The `T|` lines, sorted.
fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 1000).unwrap_or_else(|e| panic!("rejected: {e}"));
    let mut lines: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect();
    lines.sort();
    lines
}

const IMPLICIT_NETS: &str = r#"
module sub(input a, output y); assign y = a; endmodule
module child;
  reg [1:0] r = 2'b10;
  for (genvar i = 0; i < 2; i++) begin : g
    sub s(.a(r[i]), .y(n));
    assign m = ~n;
  end
  initial #1 $display("T|n0=%b n1=%b m0=%b m1=%b", g[0].n, g[1].n, g[0].m, g[1].m);
endmodule
"#;

#[test]
fn each_loop_iteration_has_its_own_implicit_nets() {
    let src = format!("{IMPLICIT_NETS}module top; child c(); endmodule");
    assert_eq!(t_lines(&src), ["T|n0=0 n1=1 m0=1 m1=0"]);
}

#[test]
fn a_blocks_implicit_nets_are_reached_by_path_from_the_top_module() {
    let src = r#"
module sub(input a, output y); assign y = a; endmodule
module top;
  reg [1:0] r = 2'b10;
  for (genvar i = 0; i < 2; i++) begin : g
    sub s(.a(r[i]), .y(n));
    assign m = ~n;
  end
  initial #1 $display("T|n0=%b n1=%b m0=%b m1=%b", g[0].n, g[1].n, g[0].m, g[1].m);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|n0=0 n1=1 m0=1 m1=0"]);
}

#[test]
fn a_port_actual_implicit_net_of_a_block_is_scalar() {
    let src = r#"
module m(input logic i, output logic o); assign o = i; endmodule
module top;
  if (1) begin : g
    m u(.i(1'b1), .o(w));
    initial #1 $display("T|w=%b %0d", w, $bits(w));
  end
  initial #2 $display("T|p=%b", g.w);
endmodule
"#;
    assert_eq!(t_lines(src), ["T|p=1", "T|w=1 1"]);
}
