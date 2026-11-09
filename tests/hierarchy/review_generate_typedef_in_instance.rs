//! §6.18, §27.4: a typedef that reads the genvar, in a generate-for block of
//! an instance, takes each iteration's width. Expected lines are the
//! reference simulator's.
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

#[test]
fn a_genvar_typedef_in_a_loop_block_of_an_instance_takes_each_width() {
    let src = r#"
module child;
  if (1) begin : g
    typedef logic [5:0] w_t;
    w_t v = '1;
    initial #1 $display("T|g bits=%0d v=%0d", $bits(v), v);
  end
  for (genvar i = 1; i <= 2; i++) begin : r
    typedef logic [4*i-1:0] x_t;
    x_t q = '1;
    initial #1 $display("T|r[%0d] bits=%0d q=%0d", i, $bits(x_t), q);
  end
endmodule
module top; child c(); endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|g bits=6 v=63",
            "T|r[1] bits=4 q=15",
            "T|r[2] bits=8 q=255"
        ]
    );
}
