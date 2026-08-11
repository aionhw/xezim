//! §22 preprocessor directives — reference-validated (audit round K-W):
//! `line applied to __LINE__/__FILE__, same-line code after `endif kept,
//! `\`" escaped-quote stringify, and `unconnected_drive pulling
//! unconnected inputs.

use xezim::simulate;

fn lines(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn line_directive_overrides_line_and_file() {
    let src = r#"
`line 100 "virt.sv" 0
module tb; initial $display("T|%0d %s", `__LINE__, `__FILE__); endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    // `line 100 ... makes the NEXT line 100; the display sits on it.
    assert!(
        lines(&sim).iter().any(|m| m == "T|100 virt.sv"),
        "got {:?}",
        lines(&sim)
    );
}

#[test]
fn code_after_endif_on_same_line_survives() {
    let src = r#"
`define A
module tb;
`ifdef A
 initial $display("T|one");
`endif initial $display("T|two");
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let msgs = lines(&sim);
    assert!(msgs.iter().any(|m| m == "T|one"), "got {:?}", msgs);
    assert!(msgs.iter().any(|m| m == "T|two"), "post-`endif stmt dropped: {:?}", msgs);
}

#[test]
fn stringify_escaped_quote() {
    let src = r#"
`define MSG(x) `"x is `\`"x`\`"`"
module tb; initial $display("T|%s", `MSG(hi)); endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert!(
        lines(&sim).iter().any(|m| m == "T|hi is \"hi\""),
        "got {:?}",
        lines(&sim)
    );
}

#[test]
fn unconnected_drive_pulls_inputs() {
    let src = r#"
`unconnected_drive pull1
module pass_thru(input wire i, output wire o); assign o = i; endmodule
`nounconnected_drive
module pass_thru0(input wire i, output wire o); assign o = i; endmodule
module tb;
  wire o1, o0;
  pass_thru  u1(.i(), .o(o1)); // declared under pull1: reads 1
  pass_thru0 u0(.i(), .o(o0)); // declared outside: stays z
  int r1, r0z;
  initial #1 begin
    r1  = (o1 === 1'b1);
    r0z = (o0 === 1'bz);
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    let g = |n: &str| {
        sim.get_signal(n)
            .or_else(|| sim.get_signal(&format!("tb.{}", n)))
            .and_then(|v| v.to_u64())
            .unwrap_or(99)
    };
    assert_eq!(g("r1"), 1, "pull1 module's unconnected input reads 1");
    assert_eq!(g("r0z"), 1, "module outside the region keeps z");
}

/// §14.11: `##n` requires a `default clocking` block. A clocking block
/// declared WITHOUT `default` does not qualify — the reference rejects it,
/// xezim used to run it silently.
#[test]
fn cycle_delay_requires_default_clocking() {
    let no_default = r#"
`timescale 1ns/1ns
module tb;
  logic clk = 0; always #5 clk = ~clk;
  clocking cb @(posedge clk); endclocking
  initial begin ##1; $display("T|ran"); end
endmodule
"#;
    let err = match simulate(no_default, 100) {
        Ok(_) => panic!("##n without a default clocking block must be rejected"),
        Err(e) => e,
    };
    assert!(
        err.contains("default clocking"),
        "diagnostic should name the missing default clocking block, got: {err}"
    );

    // With `default`, the same code elaborates and runs.
    let with_default = no_default.replace("clocking cb", "default clocking cb");
    let sim = simulate(&with_default, 100).expect("valid ##n must still run");
    assert!(
        lines(&sim).iter().any(|m| m == "T|ran"),
        "got {:?}",
        lines(&sim)
    );
}

/// §22: a conditional directive's name is an identifier and ends at the
/// first character that cannot continue one. `\`endif;` was not recognised
/// as a directive by the inline-splitter (it required WHITESPACE after the
/// keyword), so the line never got split and the `\`endif` handler swallowed
/// it whole — taking the `;` that terminated the wrapped statement with it.
#[test]
fn conditional_directive_ends_at_non_identifier_char() {
    let src = r#"
`define A
module tb;
  int x, y, z;
  initial begin
`ifdef A
    x = 1
`endif;
    y = (1
`ifdef A
      + 2
`endif);
    z = 3;
    $display("T|%0d %0d %0d", x, y, z);
  end
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        lines(&sim).iter().any(|m| m == "T|1 3 3"),
        "got {:?}",
        lines(&sim)
    );
}

/// K-R1: a macro BODY carrying `\`ifdef/\`else/\`endif`. The body keeps its
/// line breaks, the post-expansion re-scan resolves the conditional, and the
/// `;` after the macro call survives (it rides on the `\`endif` line).
#[test]
fn ifdef_inside_macro_body_expands_and_keeps_trailing_text() {
    let src = r#"
`define CFG_BIG
`define SEL_SIZE \
  `ifdef CFG_BIG \
    4096 \
  `else \
    64 \
  `endif
module tb;
  localparam integer WDBUF_SIZE = `SEL_SIZE;
  initial $display("T|%0d", WDBUF_SIZE);
endmodule
"#;
    let sim = simulate(src, 20).expect("simulate failed");
    assert!(
        lines(&sim).iter().any(|m| m == "T|4096"),
        "got {:?}",
        lines(&sim)
    );
}
