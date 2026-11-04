//! Regression neighbors for compilation-unit scope and net storage (§3.12.1, §6.7).
use xezim::simulate;

fn trace(source: &str) -> Vec<String> {
    simulate(source, 100)
        .expect("simulation failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|s| s.starts_with("T|"))
        .collect()
}

#[test]
fn unit_qualified_writes_do_not_target_later_module_declarations() {
    for (body, want) in [
        ("$unit::x = 7;", 7),
        ("$unit::x <= 8;", 8),
        ("{$unit::x} = 9;", 9),
        ("$unit::x++;", 1),
        ("for ($unit::x = 0; $unit::x < 3; $unit::x++) ;", 3),
    ] {
        // §6.5: keep the qualified write ahead of the module declaration, but
        // observe the module variable only after its declaration.
        let source = format!(
            "int x=0; module top; initial begin {body} end int x=19; initial begin #1; $display(\"T|%0d|%0d\",$unit::x,x); $finish; end endmodule"
        );
        assert_eq!(trace(&source), [format!("T|{want}|19")], "{body}");
    }
}

#[test]
fn unit_nets_keep_drivers_resolution_and_pulls() {
    let source = r#"
bit drive = 0;
wire follow = drive;
tri1 up;
tri0 down;
wand combined;
wire (weak1, weak0) strength = 1'b1;
module top;
  assign combined = drive;
  assign combined = 1'b1;
  assign (strong1, strong0) strength = 1'b0;
  initial begin
    #1; $display("T|%b|%b|%b|%b|%b",follow,up,down,combined,strength);
    drive = 1;
    #1; $display("T|%b|%b|%b|%b|%b",follow,up,down,combined,strength);
    $finish;
  end
endmodule
"#;
    assert_eq!(trace(source), ["T|0|1|0|0|0", "T|1|1|0|1|0"]);
}

#[test]
fn unit_net_initializer_keeps_unit_scope_when_a_module_shadows_its_source() {
    let source = r#"
bit source_bit = 0;
wire result_bit = source_bit;
module top;
  bit source_bit = 1;
  bit result_bit = 0;
  initial begin
    #1; $display("T|%b|%b|%b|%b", $unit::source_bit, $unit::result_bit, source_bit, result_bit);
    $unit::source_bit = 1;
    #1; $display("T|%b|%b|%b|%b", $unit::source_bit, $unit::result_bit, source_bit, result_bit);
    $finish;
  end
endmodule
"#;
    assert_eq!(trace(source), ["T|0|0|1|0", "T|1|1|1|0"]);
}

/// §6.5: an ordinary module write must not bind a later declaration.
#[test]
fn writes_before_module_declarations_are_rejected() {
    for body in [
        "result = 1;",
        "result <= 1;",
        "result++;",
        "{result} = 1;",
        "for(result=0; result<1; result++) ;",
    ] {
        let source = format!("module top; initial begin {body} end int result=0; endmodule");
        let error = match simulate(&source, 100) {
            Ok(_) => panic!("accepted {body}"),
            Err(e) => e.to_string(),
        };
        assert!(
            error.contains("result") && error.contains("before its declaration"),
            "{body}: {error}"
        );
    }
}
