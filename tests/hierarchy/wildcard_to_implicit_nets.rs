//! §23.3.2.4 / §6.10: `.*` connects a port to a net of the same name in the
//! instantiating scope, and an undeclared identifier in another instance's
//! port connection list is such a net (an implicit scalar net). Elaboration
//! connected these, but the second-pass check for missing `.*` names only
//! counted explicit declarations, so the design was rejected with "found no
//! matching signal". Verilator prints `a=0 b=1` for both orders.
use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).unwrap_or_else(|e| panic!("rejected: {e}"));
    sim.output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

const CELLS: &str = "
module child(input a, output b); assign b = ~a; endmodule
module drv(output y); assign y = 1'b0; endmodule
module snk(input [1:0] x); endmodule
";

#[test]
fn wildcard_binds_implicit_nets_declared_by_later_instances() {
    let src = format!(
        "{CELLS}
         module top;
           child c(.*);
           drv d(.y(a));
           snk s(.x({{b, b}}));
           initial begin #1; $display(\"T|a=%b b=%b\", a, b); $finish; end
         endmodule"
    );
    assert_eq!(t_lines(&src), ["T|a=0 b=1"]);
}

#[test]
fn wildcard_binds_implicit_nets_declared_by_earlier_instances() {
    let src = format!(
        "{CELLS}
         module top;
           drv d(a);
           snk s(.x({{b, b}}));
           child c(.*);
           initial begin #1; $display(\"T|a=%b b=%b\", a, b); $finish; end
         endmodule"
    );
    assert_eq!(t_lines(&src), ["T|a=0 b=1"]);
}

#[test]
fn a_wildcard_name_that_no_connection_declares_is_still_an_error() {
    let src = format!(
        "{CELLS}
         module top;
           drv d(.y(a));
           child c(.*);
           initial begin #1; $display(\"T|a=%b\", a); $finish; end
         endmodule"
    );
    let err = simulate(&src, 100)
        .err()
        .unwrap_or_else(|| panic!("accepted:\n{src}"));
    assert!(
        err.contains("wildcard port connection (.*) found no matching signal for port 'b'"),
        "{err}"
    );
}
