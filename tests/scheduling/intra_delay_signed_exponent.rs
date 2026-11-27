//! IEEE 1800-2023 §5.7.2 and §9.4.5: a bare real intra-assignment delay
//! includes the sign of its scientific-notation exponent.
//! Under §5.8 and A.8.7, an adjacent time-unit name starts the RHS.

const SIGNED_EXPONENTS: &str = r#"
`timescale 10ns/1ps
module top;
  localparam int ns = 100, s = 42;
  localparam int s_var = 21, s$var = 22, nsvar = 23;
  real x = 0.0, y = 0.0;
  initial begin
    x = #(1.5e-1) 10.0;
    $display("R control %.4f %.1f", $realtime, x);
    x = #1.5e-1 11.0;
    $display("R blocking %.4f %.1f", $realtime, x);
    y <= #2.5e-1 12.0;
    $display("R scheduled %.4f %.1f", $realtime, y);
    #0.2499;
    $display("R before NBA %.4f %.1f", $realtime, y);
    #0.0002;
    $display("R after NBA %.4f %.1f", $realtime, y);

    x = #1e-1 13.0;
    $display("R integer mantissa %.4f %.1f", $realtime, x);
    x = #1E+0 14.0;
    $display("R positive exponent %.4f %.1f", $realtime, x);
    x = #1_0.5_e-1 15.0;
    $display("R underscores %.4f %.1f", $realtime, x);

    x = #1.5e-1ns + 7;
    $display("R adjacent ns addition %.4f %.1f", $realtime, x);
    x = #1.5e-1ns;
    $display("R adjacent ns %.4f %.1f", $realtime, x);
    x = #1.5e-1s;
    $display("R adjacent s %.4f %.1f", $realtime, x);
    x = #1.5e-1s * 2;
    $display("R adjacent s multiplication %.4f %.1f", $realtime, x);
    x = #1.5e-1_s + 3;
    $display("R adjacent s after exponent underscore %.4f %.1f", $realtime, x);
    x = #1e-1ns + 8;
    $display("R integer mantissa with adjacent RHS %.4f %.1f", $realtime, x);
    x = #1.5e-1s_var;
    $display("R underscore in RHS identifier %.4f %.1f", $realtime, x);
    x = #1.5e-1s$var;
    $display("R dollar in RHS identifier %.4f %.1f", $realtime, x);
    x = #1.5e-1nsvar;
    $display("R letter in RHS identifier %.4f %.1f", $realtime, x);
    $finish;
  end
endmodule
"#;

fn simulate_lines(src: &str) -> Vec<String> {
    xezim::simulate(src, 200_000)
        .expect("simulate")
        .output
        .iter()
        .filter_map(|entry| entry.message.strip_prefix("R ").map(str::to_owned))
        .collect()
}

#[test]
fn signed_real_delays_schedule_blocking_and_nba_updates() {
    assert_eq!(
        simulate_lines(SIGNED_EXPONENTS),
        [
            "control 0.1500 10.0",
            "blocking 0.3000 11.0",
            "scheduled 0.3000 0.0",
            "before NBA 0.5499 0.0",
            "after NBA 0.5501 12.0",
            "integer mantissa 0.6501 13.0",
            "positive exponent 1.6501 14.0",
            "underscores 2.7001 15.0",
            "adjacent ns addition 2.8501 107.0",
            "adjacent ns 3.0001 100.0",
            "adjacent s 3.1501 42.0",
            "adjacent s multiplication 3.3001 84.0",
            "adjacent s after exponent underscore 3.4501 45.0",
            "integer mantissa with adjacent RHS 3.5501 108.0",
            "underscore in RHS identifier 3.7001 21.0",
            "dollar in RHS identifier 3.8501 22.0",
            "letter in RHS identifier 4.0001 23.0",
        ]
    );
}

const OTHER_DELAYS: &str = r#"
`timescale 1ns/1ps
module top;
  localparam real e = 0.3, E = 0.4, d_e = 0.5, d = 0.2;
  int x, v = 7, w;
  bit driver = 1;
  wire delayed;
  assign #1.5e-1 delayed = driver;

  bit clk;
  always #1 clk = !clk;
  default clocking cb @(posedge clk);
    output w;
  endclocking

  task automatic show;
    $display("R %.3f %0d", $realtime, x);
  endtask

  initial begin
    #1.5e-1;
    #1step;
    $display("R procedural and continuous %.3f %b", $realtime, delayed);
    x = #1step 6; show();
    x = #1.5e0 7; show();
    x = #1.5e2 8; show();
    x = #e-1; show();
    x = #E-1; show();
    x = #d_e-1; show();
    x = #e+1; show();
    x = #d - 1; show();
    x = #d -1 - v; show();
    x = #1-1; show();
    x = #1 -v; show();
    x = #10+2; show();

    @(cb);
    cb.w <= ##1 - 1;
    repeat (2) @(cb);
    $display("R cycle %0d", w);
    $finish;
  end
endmodule
"#;

#[test]
fn other_delays_keep_their_rhs_and_timing() {
    assert_eq!(
        simulate_lines(OTHER_DELAYS),
        [
            "procedural and continuous 0.151 1",
            "0.152 6",
            "1.652 7",
            "151.652 8",
            "151.952 -1",
            "152.352 -1",
            "152.852 -1",
            "153.152 1",
            "153.352 -1",
            "153.552 -8",
            "154.552 -1",
            "155.552 -7",
            "165.552 2",
            "cycle -1",
        ]
    );
}

#[cfg(unix)]
#[test]
fn preprocessed_signed_real_delays_run_from_source_and_artifact() {
    use std::path::PathBuf;
    use std::process::Command;

    const TOP: &str = r#"
`timescale 1ns/1ps
module top;
  real x = 0.0, y = 0.0;
  initial begin
    `include "delays.svh"
  end
endmodule
"#;
    const INCLUDE: &str = r#"
`define F128_DELAY 1.5e-1
`define F128_NBA(value) y <= #2.5e-1 value
    x = #`F128_DELAY 11.0;
    `F128_NBA(12.0);
    #0.249;
    $display("R early %.3f %.1f %.1f", $realtime, x, y);
    #0.002;
    $display("R late %.3f %.1f %.1f", $realtime, x, y);
    $finish;
"#;

    let temp = Command::new("mktemp")
        .arg("-d")
        .arg(std::env::temp_dir().join("xezim-intra-delay.XXXXXX"))
        .output()
        .expect("run mktemp");
    assert!(temp.status.success(), "{temp:?}");
    let scratch = PathBuf::from(String::from_utf8(temp.stdout).unwrap().trim_end());
    std::fs::write(scratch.join("top.sv"), TOP).unwrap();
    std::fs::write(scratch.join("delays.svh"), INCLUDE).unwrap();

    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .current_dir(&scratch)
            .args(args)
            .output()
            .expect("run xezim");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{args:?}:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    };
    let source = run(&["--simulate", "--no-cache", "+incdir+.", "top.sv"]);
    run(&[
        "--compile",
        "--no-cache",
        "+incdir+.",
        "top.sv",
        "-o",
        "top.xez",
    ]);
    let artifact = run(&["--simulate", "top.xez"]);
    std::fs::remove_dir_all(&scratch).unwrap();
    for output in [source, artifact] {
        let got: Vec<_> = output
            .lines()
            .filter_map(|line| line.strip_prefix("R "))
            .collect();
        assert_eq!(got, ["early 0.399 11.0 0.0", "late 0.401 11.0 12.0"]);
    }
}
