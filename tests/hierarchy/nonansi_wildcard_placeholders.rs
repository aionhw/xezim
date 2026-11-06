//! §23.3.2.4: `.*` connects only the ports that have a name. A non-ANSI null
//! port (`(a, , q)`) and an unnamed port expression (`(a[1:0])`) stay
//! unconnected: they used to be connected through the parser's placeholder
//! names, so a non-ANSI parent's own port expression was wired to the child's,
//! and the check for a missing `.*` name reported the placeholder. A name
//! with no net or variable in the instantiating scope is an error for an
//! ANSI and a non-ANSI child alike (no implicit net). The reference simulator
//! gives the same results, except that beside an unnamed port expression it
//! also leaves the child's NAMED output unconnected (it reports the missing
//! connection itself); the LRM connects it, and so does this test (core #163).
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

#[test]
fn wildcard_skips_a_port_expression_and_a_null_port() {
    let src = r#"
module leaf(a[1:0], , q); input [3:0] a; output q; assign q = ^a; endmodule
module mid(b, q); input [3:0] b; output q; logic [3:0] a; assign a = b; leaf u(.*); endmodule
module top;
  logic [3:0] x = 4'b0111;
  logic q;
  mid m(x, q);
  initial begin #1; $display("T|q=%b", q); $finish; end
endmodule
"#;
    // `u.a[1:0]` is unconnected (z), so `^a` is x; `q` is connected.
    assert_eq!(t_lines(src), ["T|q=x"]);
}

#[test]
fn wildcard_connects_an_output_of_a_nested_nonansi_child() {
    let src = r#"
module leaf(a, q); input [3:0] a; output q; assign q = ^a; endmodule
module mid(b, q); input [3:0] b; output q; logic [3:0] a; assign a = b; leaf u(.*); endmodule
module top;
  logic [3:0] x = 4'b0111;
  logic q;
  mid m(x, q);
  initial begin #1; $display("T|q=%b", q); $finish; end
endmodule
"#;
    assert_eq!(t_lines(src), ["T|q=1"]);
}

#[test]
fn a_wildcard_name_missing_from_the_parent_is_an_error_for_both_header_forms() {
    for leaf in [
        "module leaf(clk, q); input clk; output q; assign q = clk; endmodule",
        "module leaf(input clk, output q); assign q = clk; endmodule",
    ] {
        let src = format!(
            "{leaf}
             module mid(output q); leaf u(.*); endmodule
             module top;
               logic clk = 1;
               logic q;
               mid m(.q(q));
               initial begin #1; $display(\"T|q=%b\", q); $finish; end
             endmodule"
        );
        let err = simulate(&src, 100)
            .err()
            .unwrap_or_else(|| panic!("accepted:\n{src}"));
        assert!(
            err.contains("wildcard port connection (.*) found no matching signal for port 'clk'"),
            "{err}"
        );
    }
}
