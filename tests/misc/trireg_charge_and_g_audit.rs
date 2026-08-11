//! G6/G10/G11 audit closures — reference-validated.
//!
//! G6  §6.6.4 `trireg` charge storage: bits no driver drives HOLD the last
//!     driven value (modeled as an implicit weak self-driver in the net
//!     fold); a never-driven trireg reads x, not z.
//! G10 §14.4 `#1step` input skew parses and samples just before the edge.
//! G11 §11.10.3 `$bits("")` is 8 — an empty string literal is "\0".

use xezim::simulate;

fn line(sim: &xezim::compiler::Simulator, tag: &str) -> String {
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .find(|m| m.starts_with(tag))
        .unwrap_or_else(|| panic!("no output line starting with {}", tag))
}

/// Reference: initial=xxxx, driven=1010, held=1010 (charge), redriven=0101,
/// held2=0101. A plain `tri` would read z where the trireg holds.
#[test]
fn trireg_holds_charge_when_undriven() {
    let src = r#"
module tb;
  trireg [3:0] tv;
  trireg       ts;
  logic [3:0] d = 4'b1010;
  logic en = 0;
  assign tv = en ? d : 4'bzzzz;
  assign ts = en ? d[0] : 1'bz;
  initial begin
    $display("A=[%b]", tv);
    #1 en = 1;
    #1 $display("B=[%b][%b]", tv, ts);
    en = 0;
    #1 $display("C=[%b][%b]", tv, ts);
    d = 4'b0101; en = 1;
    #1 $display("D=[%b]", tv);
    en = 0;
    #1 $display("E=[%b]", tv);
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(line(&sim, "A="), "A=[xxxx]", "never-driven trireg reads x");
    assert_eq!(line(&sim, "B="), "B=[1010][0]");
    assert_eq!(line(&sim, "C="), "C=[1010][0]", "charge holds when all drivers go z");
    assert_eq!(line(&sim, "D="), "D=[0101]");
    assert_eq!(line(&sim, "E="), "E=[0101]", "charge tracks the new driven value");
}

/// Reference: 8 / 8 / 16.
#[test]
fn bits_of_string_literals() {
    let src = r#"
module tb;
  initial $display("S=[%0d][%0d][%0d]", $bits(""), $bits("a"), $bits("ab"));
endmodule
"#;
    let sim = simulate(src, 10).expect("simulate failed");
    assert_eq!(line(&sim, "S="), "S=[8][8][16]");
}

/// Reference: seen=1 — `#1step` input skew samples in the Preponed region of
/// the edge slot (the value just before the edge).
#[test]
fn onestep_input_skew_parses_and_samples() {
    let src = r#"
module tb;
  logic clk = 0, d = 0;
  logic seen;
  clocking cb @(posedge clk);
    input #1step d;
  endclocking
  always #5 clk = ~clk;
  initial begin
    #4 d = 1;
    @(cb);
    seen = cb.d;
    $display("T=[%0d]", seen);
    $finish;
  end
endmodule
"#;
    let sim = simulate(src, 100).expect("simulate failed");
    assert_eq!(line(&sim, "T="), "T=[1]");
}
