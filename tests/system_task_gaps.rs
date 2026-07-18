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

// ---------------------------------------------------------------- group 4

#[test]
fn fread_reg_memory_and_eof() {
    let dir = std::env::temp_dir().join(format!("xezim_fread_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("data.bin");
    std::fs::write(&bin, [1u8, 2, 3, 4, 5, 6, 7, 8, 0xAA, 0xBB]).unwrap();
    let src = format!(
        r#"
module tb;
  integer fd;
  integer n1, n2, n3, n4, n5, n6;
  reg [15:0] r16;
  reg [15:0] r16b;
  reg [7:0] mem [0:3];
  reg [11:0] w12;
  reg [7:0] m0, m1, m2, m3, p1, p2;
  initial begin
    fd = $fopen("{bin}", "rb");
    n1 = $fread(r16, fd);
    n2 = $fread(mem, fd);
    m0 = mem[0]; m1 = mem[1]; m2 = mem[2]; m3 = mem[3];
    n3 = $fread(w12, fd);
    n4 = $fread(r16b, fd);
    n5 = $fread(r16b, fd);
    $fclose(fd);
    fd = $fopen("{bin}", "rb");
    n6 = $fread(mem, fd, 1, 2);
    p1 = mem[1]; p2 = mem[2];
    $fclose(fd);
  end
endmodule
"#,
        bin = bin.display()
    );
    let sim = simulate(&src, 1000).expect("simulate failed");
    // All values verified against Icarus (iverilog -g2012).
    assert_eq!(u(&sim, "n1"), 2);
    assert_eq!(u(&sim, "r16"), 0x0102);
    assert_eq!(u(&sim, "n2"), 4);
    assert_eq!((u(&sim, "m0"), u(&sim, "m1"), u(&sim, "m2"), u(&sim, "m3")), (3, 4, 5, 6));
    assert_eq!(u(&sim, "n3"), 2);
    assert_eq!(u(&sim, "w12"), 0x708, "12-bit dest keeps low 12 bits of 0x0708");
    assert_eq!(u(&sim, "n4"), 2);
    assert_eq!(u(&sim, "r16b"), 0xAABB);
    assert_eq!(u(&sim, "n5"), 0, "EOF returns 0");
    assert_eq!(u(&sim, "n6"), 2, "start/count form reads 2 elements");
    assert_eq!((u(&sim, "p1"), u(&sim, "p2")), (1, 2));
    assert!(sim.warned_system_task_names().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- group 5

#[test]
fn sdf_annotate_applies_iopath_delay_at_runtime() {
    let dir = std::env::temp_dir().join(format!("xezim_sdf_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sdf = dir.join("t.sdf");
    std::fs::write(
        &sdf,
        r#"(DELAYFILE
  (SDFVERSION "3.0")
  (TIMESCALE 1ns)
  (CELL (CELLTYPE "andcell") (INSTANCE u1)
    (DELAY (ABSOLUTE (IOPATH a y (5.0:5.0:5.0) (5.0:5.0:5.0))
                     (IOPATH b y (5.0:5.0:5.0) (5.0:5.0:5.0)))))
)
"#,
    )
    .unwrap();
    let tpl = r#"
`timescale 1ns/1ns
module andcell(input a, input b, output y);
  assign y = a & b;
endmodule
module tb;
  reg a, b; wire y;
  reg mid, fin;
  andcell u1(.a(a), .b(b), .y(y));
  initial begin
    ANNOTATE
    a = 1; b = 0;
    #10 b = 1;
    #3  mid = y;  // 5ns IOPATH: still 0 here; without SDF: 1
    #10 fin = y;  // settled: 1 either way
  end
endmodule
"#;
    let with = tpl.replace("ANNOTATE", &format!("$sdf_annotate(\"{}\");", sdf.display()));
    let sim = simulate(&with, 1000).expect("simulate failed");
    assert_eq!(u(&sim, "mid") & 1, 0, "IOPATH delay must postpone y past t=13");
    assert_eq!(u(&sim, "fin") & 1, 1);
    let without = tpl.replace("ANNOTATE", "");
    let sim2 = simulate(&without, 1000).expect("simulate failed");
    assert_eq!(u(&sim2, "mid") & 1, 1, "without annotation y updates immediately");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sdf_annotate_missing_file_is_fatal() {
    let src = r#"
`timescale 1ns/1ns
module tb;
  reg done;
  initial begin
    $sdf_annotate("/nonexistent/xezim_no_such.sdf");
    #1 done = 1;  // must never run
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    // hard failure: simulation ends at time 0, so `#1 done = 1` never runs
    let done = sim
        .get_signal("tb.done")
        .or_else(|| sim.get_signal("done"))
        .and_then(|v| v.to_u64())
        .unwrap_or(0);
    assert_ne!(done, 1, "simulation must stop before #1 after a missing SDF file");
}

// ---------------------------------------------------------------- group 6

#[test]
fn fsdb_dump_tasks_write_fst() {
    let dir = std::env::temp_dir().join(format!("xezim_fsdb_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fsdb = dir.join("wave.fsdb");
    let fst = dir.join("wave.fst");
    let src = format!(
        r#"
`timescale 1ns/1ns
module tb;
  reg clk = 0;
  reg [3:0] cnt = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt <= cnt + 1;
  initial begin
    $fsdbDumpfile("{fsdb}");
    $fsdbDumpvars(0, tb);
    #40 $finish;
  end
endmodule
"#,
        fsdb = fsdb.display()
    );
    let sim = simulate(&src, 1000).expect("simulate failed");
    let meta = std::fs::metadata(&fst).expect(".fsdb path must be rewritten to .fst");
    assert!(meta.len() > 0, "FST dump must not be empty");
    assert!(
        sim.warned_system_task_names().iter().any(|n| n.contains("$fsdbDump")),
        "one fsdb mapping note must be recorded"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vcdpluson_maps_to_vcd() {
    let dir = std::env::temp_dir().join(format!("xezim_vcdplus_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let vcd = dir.join("plus.vcd");
    let src = format!(
        r#"
`timescale 1ns/1ns
module tb;
  reg clk = 0;
  always #5 clk = ~clk;
  initial begin
    $dumpfile("{vcd}");
    $vcdpluson;
    #20 $vcdplusoff;
    #20 $finish;
  end
endmodule
"#,
        vcd = vcd.display()
    );
    let sim = simulate(&src, 1000).expect("simulate failed");
    let text = std::fs::read_to_string(&vcd).expect("VCD file must exist");
    assert!(text.contains("$enddefinitions"), "VCD must have a header");
    assert!(text.contains("#5"), "VCD must record changes while on");
    assert!(
        sim.warned_system_task_names().contains(&"$vcdpluson".to_string()),
        "one vcdplus mapping note must be recorded"
    );
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
