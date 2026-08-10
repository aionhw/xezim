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
