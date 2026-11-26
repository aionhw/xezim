//! IEEE 1800-2023 §10.3, §10.3.3 (#721): the continuous assignments of an
//! instance record one shared scope string as their origin. Each instance
//! keeps its own delay and its own drivers, and statement coverage counts
//! the assignments of every instance. Expected lines are the reference
//! simulator's.

use std::process::Command;
use xezim::simulate;

const SRC: &str = r#"
module unit #(parameter integer D = 1)(input wire pulse, output wire echo, output wire [1:0] both);
  wire mirrored = ~pulse;
  wire tap0, tap1;
  assign tap0 = pulse;
  assign tap1 = tap0, both = {tap1, mirrored};
  assign #D echo = tap1;
endmodule
module mid(input wire pulse, output wire [1:0] echo);
  wire gated;
  assign gated = pulse;
  wire [1:0] ba, bb;
  unit #(.D(2)) a(.pulse(gated), .echo(echo[0]), .both(ba));
  unit #(.D(4)) b(.pulse(gated), .echo(echo[1]), .both(bb));
endmodule
module top;
  reg pulse = 0, other = 0;
  wire [1:0] e0, e1;
  wire all_up;
  assign all_up = &e0 & &e1;
  mid m0(.pulse(pulse), .echo(e0));
  mid m1(.pulse(other), .echo(e1));
  initial begin
    #5 pulse = 1;
    #1 $display("T|6 e0=%b e1=%b", e0, e1);
    #2 $display("T|8 e0=%b e1=%b", e0, e1);
    #2 $display("T|10 e0=%b e1=%b all=%b", e0, e1, all_up);
    other = 1;
    #3 $display("T|13 e0=%b e1=%b", e0, e1);
    #2 $display("T|15 e0=%b e1=%b all=%b", e0, e1, all_up);
    pulse = 0;
    #5 $display("T|20 e0=%b e1=%b all=%b both=%b%b", e0, e1, all_up, m0.a.both, m1.b.both);
    $finish;
  end
endmodule
"#;

const EXPECTED: [&str; 6] = [
    "T|6 e0=00 e1=00",
    "T|8 e0=01 e1=00",
    "T|10 e0=11 e1=00 all=0",
    "T|13 e0=11 e1=01",
    "T|15 e0=11 e1=11 all=1",
    "T|20 e0=00 e1=11 all=0 both=0110",
];

#[test]
fn instance_assignments_keep_their_delays_and_drivers() {
    let sim = simulate(SRC, 100).expect("simulate");
    let lines: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(lines, EXPECTED);
}

/// Statement coverage reads the origin of every assignment: the six
/// spelled-out assignment statements of the two modules and the top are
/// instrumented, and the run prints the same lines.
#[test]
fn statement_coverage_counts_the_assignments_of_every_instance() {
    let dir = std::env::temp_dir().join(format!("xezim_origin_721_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.sv"), SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .env("XEZIM_COV_DB", dir.join("cov.json"))
        .env_remove("XEZIM_CODE_COVERAGE")
        .env_remove("XEZIM_VERBOSE")
        .args(["--no-cache", "--code-coverage=stmt", "-s", "top", "t.sv"])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("T|")).collect();
    assert_eq!(lines, EXPECTED, "{stdout}");
    let json = std::fs::read_to_string(dir.join("cov.json")).unwrap_or_default();
    // Per instance: `m0` saw three changes of its input, `m1` two. Lines of
    // `SRC` (the raw string starts with an empty line).
    let assign = |line: u32, count: u32| {
        format!(
            "{{\"file\": \"t.sv\", \"line\": {line}, \"kind\": \"assign\", \"count\": {count}}}"
        )
    };
    for (scope, unit, rows) in [
        ("top", "top", vec![(20, 9)]),
        ("top.m0", "mid", vec![(11, 3)]),
        ("top.m1", "mid", vec![(11, 2)]),
        ("top.m0.a", "unit", vec![(3, 3), (5, 3), (6, 6), (7, 3)]),
        ("top.m0.b", "unit", vec![(3, 3), (5, 3), (6, 6), (7, 3)]),
        ("top.m1.a", "unit", vec![(3, 2), (5, 2), (6, 4), (7, 2)]),
        ("top.m1.b", "unit", vec![(3, 2), (5, 2), (6, 4), (7, 2)]),
    ] {
        let head = format!("\"scope\": \"{scope}\",\n        \"design_unit\": \"{unit}\"");
        let at = json
            .find(&head)
            .unwrap_or_else(|| panic!("no scope {scope}: {json}"));
        let entry = &json[at..];
        let entry = &entry[..entry.find("]\n").unwrap_or(entry.len())];
        for (line, count) in rows {
            assert!(
                entry.contains(&assign(line, count)),
                "{scope} line {line}: {entry}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
