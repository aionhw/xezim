//! IEEE 1800-2023 §23.10, §27.4 (#721): elaboration holds the prepared body
//! of a parameter environment only while a later instance can still share
//! it, and prepares it again when a released environment comes back. A
//! module generated under a distinct value for each of 60 instances, the
//! first environments again, and a row of 40 environments instantiated three
//! times elaborate to the same design as with every body held: each instance
//! reads its own parameters, generate scope and constant function. Expected
//! lines are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module leaf #(parameter integer BASE = 0, parameter integer TAG = 0) (output logic [15:0] q);
  function automatic integer calc(input integer value);
    return value + BASE;
  endfunction
  generate
    for (genvar k = BASE; k < BASE + 1; k++) begin : bucket
      localparam integer V = BASE * 3 + 1;
      logic [15:0] stage;
      initial begin
        stage = V;
        #1 q = calc(stage) + k;
      end
    end
  endgenerate
endmodule
// A row instantiates the same 40 distinct environments each time it is
// instantiated itself: more than one definition holds without a reuse.
module row #(parameter integer ROW = 0) (output logic [31:0] sum);
  logic [15:0] q [40];
  for (genvar i = 0; i < 40; i++) begin : col
    leaf #(.BASE(i), .TAG(ROW)) u (.q(q[i]));
  end
  always_comb begin
    sum = 0;
    for (int i = 0; i < 40; i++) sum += q[i] * (i + 1);
  end
endmodule
module top;
  logic [15:0] single [60];
  logic [31:0] sums [3];
  // 60 environments used once each, then the first ones again.
  for (genvar i = 0; i < 60; i++) begin : once
    leaf #(.BASE(100 + i)) u (.q(single[i]));
  end
  logic [15:0] again [4];
  for (genvar i = 0; i < 4; i++) begin : back
    leaf #(.BASE(100 + i)) u (.q(again[i]));
  end
  for (genvar r = 0; r < 3; r++) begin : rows
    row #(.ROW(r)) u (.sum(sums[r]));
  end
  initial begin
    #2;
    for (int i = 0; i < 60; i += 13) $display("T|single[%0d]=%0d", i, single[i]);
    $display("T|single[59]=%0d", single[59]);
    for (int i = 0; i < 4; i++) $display("T|again[%0d]=%0d", i, again[i]);
    for (int r = 0; r < 3; r++) $display("T|sums[%0d]=%0d", r, sums[r]);
    $display("T|%0d %0d %0d", once[17].u.bucket[117].V, rows[2].u.col[39].u.bucket[39].V,
             back[3].u.bucket[103].stage);
    $finish;
  end
endmodule
"#;

const EXPECTED: [&str; 14] = [
    "T|single[0]=501",
    "T|single[13]=566",
    "T|single[26]=631",
    "T|single[39]=696",
    "T|single[52]=761",
    "T|single[59]=796",
    "T|again[0]=501",
    "T|again[1]=506",
    "T|again[2]=511",
    "T|again[3]=516",
    "T|sums[0]=107420",
    "T|sums[1]=107420",
    "T|sums[2]=107420",
    "T|352 118 310",
];

fn lines(text: impl Iterator<Item = String>) -> Vec<String> {
    text.filter(|l| l.starts_with("T|")).collect()
}

#[test]
fn released_and_returning_environments_elaborate_alike() {
    let sim = simulate(SRC, 10).expect("simulate");
    assert_eq!(
        lines(sim.output.iter().map(|o| o.message.clone())),
        EXPECTED
    );
}

/// The same design through the design cache, a compiled artifact and the
/// interpreted method path.
#[test]
fn released_environments_survive_the_cache_and_an_artifact() {
    let dir = std::env::temp_dir().join(format!("xezim_prepared_721_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("design.sv"), SRC).expect("write source");
    let run = |args: &[&str], interpreted: bool| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.args(args).current_dir(&dir);
        if interpreted {
            cmd.env("XEZIM_COMPILE_METHODS", "0");
        }
        let out = cmd.output().expect("run xezim");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    for interpreted in [false, true] {
        let direct = run(&["--no-cache", "design.sv"], interpreted);
        assert_eq!(lines(direct.lines().map(str::to_string)), EXPECTED);
    }
    for _ in 0..2 {
        let cached = run(&["--cache-dir", "cache", "design.sv"], false);
        assert_eq!(lines(cached.lines().map(str::to_string)), EXPECTED);
    }
    run(
        &["--compile", "--no-cache", "design.sv", "-o", "design.xez"],
        false,
    );
    let loaded = run(&["--simulate", "design.xez"], false);
    assert_eq!(lines(loaded.lines().map(str::to_string)), EXPECTED);
    let _ = std::fs::remove_dir_all(&dir);
}
