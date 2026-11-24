use std::process::Command;

/// §9.4.2.2: a timed implicit event control watches what its guarded
/// statements read. A body with no reads cannot advance the simulation.
#[test]
fn timed_star_waits_for_inferred_inputs() {
    let sim = xezim::simulate(
        r#"
`timescale 1ns/1ns
module top;
  reg src, copied, waited, inert;
  reg enabled;
  always @(*) #3 copied = src;
  always @(*) wait (enabled) waited = src;
  always @(*) #1 inert = 1;
  initial begin
    #1 src = 1;
    #1 enabled = 1;
    #3 $display("first=%b%b%b", copied, waited, inert);
    src = 0;
    #4 $display("second=%b%b%b", copied, waited, inert);
    $finish;
  end
endmodule
"#,
        20,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|out| out.message.as_str()).collect();
    assert!(lines.contains(&"first=11x"), "{lines:?}");
    assert!(lines.contains(&"second=00x"), "{lines:?}");
}

/// §4.4.2, §9.2.2: the startup scheduler arms event-headed processes
/// before initial writes. Nonblocking feedback wakes after rearming.
#[test]
fn event_headed_always_arms_before_time_zero_initial_writes() {
    let sim = xezim::simulate(
        r#"
`timescale 1ns/1ns
module top;
  reg blocking_clock, nba_clock, nested_clock;
  integer n_blocking = 0, n_nba = 0, n_nested = 0;
  initial begin
    blocking_clock = 0;
    nba_clock = 0;
    nested_clock = 0;
  end
  always @(blocking_clock) #10 blocking_clock = ~blocking_clock;
  always @(*) #10 nba_clock <= ~nba_clock;
  always begin @(nested_clock) #10 nested_clock = ~nested_clock; end
  always @(posedge blocking_clock) n_blocking++;
  always @(posedge nba_clock) n_nba++;
  always @(posedge nested_clock) n_nested++;
  initial begin #45; $display("edges=%0d,%0d,%0d", n_blocking, n_nba, n_nested); $finish; end
endmodule
"#,
        60,
    )
    .expect("simulate");
    let lines: Vec<_> = sim.output.iter().map(|out| out.message.as_str()).collect();
    assert!(lines.contains(&"edges=1,2,1"), "{lines:?}");
}

/// IEEE 1364-2005 §6.2.1 and IEEE 1800-2023 §6.8: Verilog mode uses a
/// time-zero assignment, including for library modules in a mixed build.
#[test]
fn verilog_initializers_are_opt_in_per_source_file() {
    let dir = std::env::temp_dir().join(format!("xezim_1545_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("legacy.v"),
        r#"
`timescale 1ns/1ns
module legacy(output reg a = 1'b1, b = 1'b1);
  reg c = 1'b0;
  wire driven = 1'b1;
  time a_seen, b_seen, c_seen, net_seen;
  always @(posedge a) a_seen = $time;
  always @(posedge b) b_seen = $time;
  always @(negedge c) c_seen = $time;
  always @(posedge driven) net_seen = $time;
endmodule
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("top.sv"),
        r#"
`timescale 1ns/1ns
module top;
  legacy u();
  reg before_time_zero = 1'b1;
  time sv_seen;
  always @(posedge before_time_zero) sv_seen = $time;
  initial begin
    #1;
    $display("seen=%0d,%0d,%0d,%0d,%0d", u.a_seen, u.b_seen, u.c_seen, u.net_seen, sv_seen);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();

    let outputs: Vec<_> = [&[][..], &["--verilog-2005-initializers"][..]]
        .iter()
        .map(|flags| {
            Command::new(env!("CARGO_BIN_EXE_xezim"))
                .current_dir(&dir)
                .args(["--no-cache", "-s", "top", "top.sv", "-v", "legacy.v"])
                .args(*flags)
                .output()
                .expect("run xezim")
        })
        .collect();
    std::fs::remove_dir_all(&dir).unwrap();
    for (output, expected) in outputs.iter().zip(["seen=x,x,x,0,x", "seen=0,0,0,0,x"]) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(expected),
            "wanted {expected:?}\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
