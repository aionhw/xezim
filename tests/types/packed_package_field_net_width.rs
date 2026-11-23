/// A packed package parameter supplies the width of a dependent net (§7.2.1, §26.3).
#[test]
fn packed_package_field_net_width() {
    let result = xezim::simulate(
        r#"
package types;
  typedef struct packed { logic [1:0] hi; logic [11:0] irq; logic [1:0] lo; } config_t;
endpackage
package opts;
  import types::config_t;
  parameter config_t pt = 16'h03fc;
  parameter int unsigned count = pt.irq - 43;
endpackage
module top;
  wire [opts::count-1:0] bus;
  initial $display("irq=%0d count=%0d width=%0d", opts::pt.irq, opts::count, $bits(bus));
endmodule
"#,
        100,
    )
    .expect("simulate");
    assert!(
        result
            .output
            .iter()
            .any(|line| line.message == "irq=255 count=212 width=212")
    );
}
