//! §23.2.2.1 — a non-ANSI port's data type may be supplied by a SEPARATE
//! variable declaration completing the port:
//!
//!   module m(Q_out, Q_in);
//!     output [W-1:0] Q_out;   // direction (implicit net type)
//!     reg    [W-1:0] Q_out;   // completes the port as a variable  <-- legal
//!
//! xezim previously rejected the completing `reg` with "duplicate declaration
//! of 'Q_out'". The DataDeclaration elaboration path lacked the port-completion
//! merge that NetDeclaration (`wire` completes port) and PortDeclaration
//! (direction completes a var) already had. Mirrors the Samsung RF2 compiled-
//! memory helper `ln04lpp_*_error_injection` port style.

use xezim::simulate;

fn line(src: &str) -> Vec<String> {
    simulate(src, 100)
        .expect("sim")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn output_completed_by_reg() {
    let src = r#"
module dut (Q_out, Q_in);
    parameter word_width = 8;
    output [word_width-1:0] Q_out;
    input  [word_width-1:0] Q_in;
    reg [word_width-1:0] Q_out;      // legal non-ANSI completion
    always @(*) Q_out = Q_in;
endmodule
module tb;
    reg  [7:0] in;
    wire [7:0] out;
    dut d(.Q_out(out), .Q_in(in));
    initial begin
        in = 8'hA5;
        #1 $display("Q_out=%0h", out);
    end
endmodule
"#;
    // Elaboration must succeed (no "duplicate declaration of 'Q_out'") AND the
    // completing `reg` must be a procedurally-drivable variable output.
    let out = line(src);
    assert!(
        out.iter().any(|m| m == "Q_out=a5"),
        "non-ANSI output+reg completion failed to elaborate/run; got {:?}",
        out
    );
}

#[test]
fn input_completed_by_wire_still_ok() {
    // Regression guard: the pre-existing wire-completes-port path must remain.
    let src = r#"
module m(a, y);
    input a;
    output y;
    wire a;                         // completes the input port as a net
    assign y = a;
    initial #1 $display("y=%b", y);
endmodule
"#;
    let out = line(src);
    assert!(
        out.iter().any(|m| m.starts_with("y=")),
        "wire-completes-port regressed; got {:?}",
        out
    );
}
