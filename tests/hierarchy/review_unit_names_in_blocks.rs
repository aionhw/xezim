//! §3.12.1: `$unit::u` in a generate block that declares `u` reads the
//! compilation unit's `u`, at the top, beside a module-level shadow and in an
//! instance. Expected lines are the reference simulator's.
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
fn unit_qualified_names_in_blocks_that_declare_them_read_the_unit() {
    let src = r#"
int u = 5;
module top;
  if (1) begin : g
    int u = 7;
    initial #2 $display("T|g u=%0d unit=%0d", u, $unit::u);
  end
  if (1) begin
    int u = 8;
    initial #3 $display("T|genblk2 u=%0d unit=%0d", u, $unit::u);
  end
  initial #1 $display("T|top u=%0d", u);
endmodule
"#;
    assert_eq!(
        t_lines(src),
        ["T|g u=7 unit=5", "T|genblk2 u=8 unit=5", "T|top u=5"]
    );
}

#[test]
fn unit_qualified_names_beside_a_module_shadow_and_in_an_instance() {
    let src = r#"
int u = 5;
module child;
  if (1) begin : g int u = 7; initial #2 $display("T|c.g unit=%0d", $unit::u); end
endmodule
module top;
  int u = 6;
  child c();
  initial #1 $display("T|top u=%0d unit=%0d", u, $unit::u);
  if (1) begin : g
    int u = 7;
    initial #2 $display("T|g u=%0d unit=%0d", u, $unit::u);
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        ["T|c.g unit=5", "T|g u=7 unit=5", "T|top u=6 unit=5"]
    );
}
