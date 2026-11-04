use std::process::Command;

fn output(source: &str) -> String {
    let sim = xezim::simulate(source, 1000).expect("simulate");
    sim.output
        .iter()
        .map(|line| line.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn delayed_observer_preserves_clocked_updates() {
    let source = r#"
`timescale 1ps/1ps
module top;
  reg clock=0;
  int rising=0, falling=0, samples=0;
  reg observed;
  always #1 clock=~clock;
  always @(posedge clock) rising<=rising+1;
  always @(negedge clock) falling<=falling+1;
  OBSERVER
  initial begin
    #100;
    $display("COUNTS %0d %0d",rising,falling);
    if(observed===1'bx) $display("OBSERVER_FAIL");
    $finish;
  end
endmodule
"#;
    for observer in [
        "always @(clock) begin #1; samples++; observed=clock; end",
        "always begin @(clock); #1; samples++; observed=clock; end",
        "always @* begin #1; observed=clock; end",
        "always @(*) begin #1; observed=clock; end",
    ] {
        let result = output(&source.replace("OBSERVER", observer));
        assert!(result.contains("COUNTS 50 49"), "{result}");
        assert!(!result.contains("OBSERVER_FAIL"), "{result}");
    }
}

#[test]
fn condition_wait_in_event_body_suspends_and_ignores_busy_edges() {
    for event in ["pulse", "posedge pulse"] {
        let source = r#"
module top;
  reg pulse=0, permit=0;
  int started=0, ended=0;
  always @(EVENT) begin
    started++;
    wait(permit);
    ended++;
  end
  initial begin
    #2 pulse=1;
    #2 $display("WAIT_A %0d %0d",started,ended);
    pulse=0;
    #2 pulse=1;
    #2 permit=1;
    #2 $display("WAIT_B %0d %0d",started,ended);
    $finish;
  end
endmodule
"#;
        let result = output(&source.replace("EVENT", event));
        assert!(result.contains("WAIT_A 1 0"), "{result}");
        assert!(result.contains("WAIT_B 1 1"), "{result}");
    }
}

#[test]
fn explicit_level_nested_event_misses_busy_edges() {
    let result = output(
        r#"
module top;
  reg trigger=0, release_wait=0;
  int entered=0, resumed=0;
  always @(trigger) begin
    entered++;
    @(posedge release_wait);
    resumed++;
  end
  initial begin
    #2 trigger=1;
    #2 trigger=0;
    #2 trigger=1;
    #2 release_wait=1;
    #2 $display("FIRST %0d %0d",entered,resumed);
    trigger=0;
    #2 release_wait=0;
    #2 release_wait=1;
    #2 $display("SECOND %0d %0d",entered,resumed);
    $finish;
  end
endmodule
"#,
    );
    assert!(result.contains("FIRST 1 1"), "{result}");
    assert!(result.contains("SECOND 2 2"), "{result}");
}

#[test]
fn queued_flow_drains_with_edge_skipping_enabled_and_disabled() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/scheduling/fixtures/queued_flow.sv"
    );
    // The three configurations are independent runs: start them together.
    let runs: Vec<_> = [("1", "1"), ("1", "0"), ("0", "1")]
        .into_iter()
        .map(|(skip, armed)| {
            let child = Command::new(env!("CARGO_BIN_EXE_xezim"))
                .env("XEZIM_EVENT_EDGE", skip)
                .env("XEZIM_ARMED_EDGE", armed)
                .args([source, "-s", "flow_tb", "--max-time", "50ns"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("run simulator");
            (skip, child)
        })
        .collect();
    for (skip, child) in runs {
        let result = child.wait_with_output().expect("run simulator");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.status.success(), "skip={skip}: {text}");
        assert!(!text.contains("FLOW_FAIL"), "skip={skip}: {text}");
        assert!(
            text.contains("FLOW_PASS ports=4 requests=240 cycles=981"),
            "skip={skip}: {text}"
        );
        for port in 0..4 {
            let marker = format!("SOURCE[{port}] TAKE");
            assert_eq!(text.matches(&marker).count(), 240, "skip={skip}: {text}");
        }
    }
}
