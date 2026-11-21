//! §10.3.3: sample a variable delay when its driver changes.
use xezim::simulate;

#[test]
fn changing_a_delay_does_not_retime_an_in_flight_transition() {
    let sim = simulate(
        r#"
`timescale 1ns/1ps
module top;
  logic a = 1;
  real dr = 0.0;
  int di = 0;
  wire wr, wi;
  assign #(dr) wr = a;
  assign #(di) wi = a;
  initial begin
    #1 dr = 5.5; di = 5;
    #10 a = 0;
    #1 dr = 0.0; di = 0;
    $display("pending=%b%b", wr, wi);
    #4.25 $display("integer-first=%b%b", wr, wi);
    #0.5 $display("arrived=%b%b", wr, wi);
  end
endmodule"#,
        20000,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(lines, ["pending=11", "integer-first=10", "arrived=00"]);
}

#[test]
fn scoped_variable_delay_selects_rise_fall_and_turnoff() {
    let sim = simulate(
        r#"
`timescale 1ns/1ns
module drv #(parameter BIAS=1)(input logic a, input int d, output wire y);
  assign #(d+BIAS, d+2, d+3) y = a;
endmodule
module top;
  logic a = 0;
  int d = 1;
  wire w;
  drv u(a,d,w);
  initial begin
    #5 a = 1;
    #1 $display("rise-pending=%b",w);
    #2 a = 0;
    #2 $display("fall-pending=%b",w);
    #2 a = 1'bz;
    #3 $display("off-pending=%b",w);
    #2 $display("off=%b",w);
  end
endmodule"#,
        30,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        lines,
        ["rise-pending=0", "fall-pending=1", "off-pending=0", "off=z"]
    );
}

#[test]
fn net_declaration_and_gate_cancel_short_pulses() {
    let sim = simulate(
        r#"
`timescale 1ns/1ns
module top;
  logic a = 0;
  int d = 2;
  wire #(d) net_copy = a;
  wire gated;
  buf #(d) g(gated,a);
  initial begin
    #4 a = 1;
    #1 a = 0;
    #2 $display("filtered=%b%b", net_copy, gated);
    d = 4; a = 1;
    #3 $display("pending=%b%b", net_copy, gated);
    #2 $display("arrived=%b%b", net_copy, gated);
  end
endmodule"#,
        20,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(lines, ["filtered=00", "pending=00", "arrived=11"]);
}

#[test]
fn dynamic_zero_driver_precedes_inactive_and_nba() {
    let sim = simulate(
        r#"
module top;
  logic a = 0, q = 0, sampled = 1;
  int delay_ticks = 0;
  wire y, chained;
  assign #(delay_ticks) y = a;
  assign #(delay_ticks) chained = y;
  always @(posedge chained) sampled = q;
  initial begin
    #2 a = 1; q <= 1;
    #0 $display("inactive=%b%b%b%b", y, chained, sampled, q);
    #1 $display("later=%b%b", sampled, q);
  end
endmodule"#,
        8,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(lines, ["inactive=1100", "later=01"]);
}

#[test]
fn structural_delay_modes_use_separate_prepared_caches() {
    let dir = std::env::temp_dir().join(format!("xezim_driver_mode_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join("mode.sv");
    std::fs::write(
        &sv,
        r#"`timescale 1ns/1ns
module top;
  logic a = 1;
  wire [1:0] y;
  buf #(0,5) g(y[0],a);
  initial begin
    #8 a = 0;
    #2 $display("after-two=%b",y[0]);
    #5 $finish;
  end
endmodule"#,
    )
    .unwrap();
    let cache = dir.join("cache");
    for (flag, want, hit) in [
        (None, "after-two=1", false),
        (Some("+delay_mode_zero"), "after-two=0", false),
        (Some("+delay_mode_unit"), "after-two=0", false),
        (None, "after-two=1", true),
        (Some("+delay_mode_unit"), "after-two=0", true),
        (Some("+delay_mode_zero"), "after-two=0", true),
    ] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--verbose", "--cache-dir"])
            .arg(&cache)
            .args(flag)
            .arg(&sv)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{flag:?}: {stdout}\n{stderr}");
        assert!(stdout.contains(want), "{flag:?}: {stdout}\n{stderr}");
        assert_eq!(stderr.contains("prepared-comb hit:"), hit, "{flag:?}: {stderr}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
