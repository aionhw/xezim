use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| m.starts_with("T|"))
        .collect()
}

/// §26.3, §6.19: a class in an importing child module reads the package's
/// enum member even when a compilation-unit parameter has the same name.
#[test]
fn class_checks_imported_enum_member_over_unit_parameter() {
    if !super::compiled_method_test_env::policies(&[("0", "1000"), ("1", "0")]) {
        return;
    }
    let src = r#"
parameter RED = 6'd0;
package pkg;
  typedef enum logic [7:0] {ADD = 0, RED = 64} opcode_t;
endpackage
package other;
  parameter RED = 33;
endpackage
module child;
  import pkg::*;
  localparam int MODULE_OP = RED;
  class base;
    localparam int RED = 7;
  endclass
  class derived extends base;
    function int inherited(); return RED; endfunction
  endclass
  class decoder;
    localparam int CLASS_OP = RED;
    function void check(opcode_t op);
      $display("T|member=%0d reduction=%0d constants=%0d,%0d", RED, op inside {RED}, MODULE_OP, CLASS_OP);
    endfunction
  endclass
  initial begin
    decoder d = new;
    derived c = new;
    $display("T|inherited=%0d", c.inherited());
    d.check(ADD);
    d.check(RED);
  end
endmodule
module t;
  child c();
  initial $display("T|unit=%0d", RED);
endmodule
"#;
    let mut lines = t_lines(src);
    lines.sort();
    assert_eq!(
        lines,
        [
            "T|inherited=7",
            "T|member=64 reduction=0 constants=64,64",
            "T|member=64 reduction=1 constants=64,64",
            "T|unit=0",
        ]
    );
}
