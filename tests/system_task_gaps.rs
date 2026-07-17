//! Regression tests for the July-2026 missing-system-task audit.
//!
//! Group 1: unknown-system-task meta-diagnostic (once per name, never for
//!          names serviced by either dispatcher or by internals).
//! Group 2: $exit terminates like $finish.
//! Group 3: $fstrobe/$fmonitor file variants.
//! Group 4: $fread binary load (reg + memory forms).
//! Group 5: $sdf_annotate runtime annotation.
//! Group 6: $fsdbDumpfile/$fsdbDumpvars/$vcdpluson mapping.
//! Group 7: recognized-warn stubs.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

// ---------------------------------------------------------------- group 1

#[test]
fn unknown_task_warns_once_per_name() {
    let src = r#"
module tb;
  integer n;
  initial begin
    $bogus_task(1);
    $bogus_task(2);
    n = $bogus_func(3);
    repeat (3) $another_missing;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let warned = sim.warned_system_task_names();
    assert!(warned.contains(&"$bogus_task".to_string()), "warned: {:?}", warned);
    assert!(warned.contains(&"$bogus_func".to_string()), "warned: {:?}", warned);
    assert!(warned.contains(&"$another_missing".to_string()), "warned: {:?}", warned);
    // unknown function returns 0, does not abort simulation
    assert_eq!(u(&sim, "n"), 0);
}

// ---------------------------------------------------------------- group 2

#[test]
fn exit_terminates_like_finish() {
    let src = r#"
module tb;
  initial begin
    $display("before");
    #5 $exit;
    $display("after");
  end
  initial #20 $display("late");
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let outs: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert!(outs.contains(&"before"), "outs: {:?}", outs);
    assert!(!outs.contains(&"after"), "$exit must stop the process: {:?}", outs);
    assert!(!outs.contains(&"late"), "$exit must end simulation: {:?}", outs);
    assert!(sim.warned_system_task_names().is_empty());
}

// ---------------------------------------------------------------- group 3

#[test]
fn fstrobe_and_fmonitor_write_to_file() {
    let dir = std::env::temp_dir().join(format!("xezim_fstrobe_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("out.txt");
    let src = format!(
        r#"
module tb;
  integer fd;
  reg [7:0] a;
  initial begin
    fd = $fopen("{out}", "w");
    a = 8'h11;
    $fstrobe(fd, "strobe a=%h t=%0t", a, $time);
    a = 8'h22;
    #1;
    $fmonitor(fd, "mon a=%h t=%0t", a, $time);
    #1 a = 8'h33;
    #1 a = 8'h44;
    #1 $fclose(fd);
    $finish;
  end
endmodule
"#,
        out = out.display()
    );
    let sim = simulate(&src, 1000).expect("simulate failed");
    let text = std::fs::read_to_string(&out).expect("fstrobe output file missing");
    // Matches Icarus (iverilog -g2012) verbatim: strobe sees the post-update
    // value 22; fmonitor prints once when armed and on each change.
    assert_eq!(
        text,
        "strobe a=22 t=0\nmon a=22 t=1\nmon a=33 t=2\nmon a=44 t=3\n"
    );
    assert!(sim.warned_system_task_names().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn handled_names_do_not_trip_unknown_warning() {
    // Function-only names in statement position (result discarded) and
    // ordinary handled tasks must NOT be reported as unknown.
    let src = r#"
module tb;
  integer x;
  initial begin
    $urandom;
    $random;
    x = $countones(8'hF0);
    $display("x=%0d", x);
    $strobe("s=%0d", x);
    $monitoroff;
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    assert!(
        sim.warned_system_task_names().is_empty(),
        "spurious unknown-task warnings: {:?}",
        sim.warned_system_task_names()
    );
}
