//! IEEE 1800-2023 §6.20.2, §10.9.1 (#721): the instances of a module share
//! one copy of the declared type of each of its value parameters; what the
//! type means stays per instance. Parameters typed by an inline struct, a
//! vector sized by another parameter, a package struct and enum, and a
//! typedef of the instance take their `default:` keys, widths and member
//! values from each instance's own parameters, through the interpreted
//! method path, the design cache and a compiled artifact as well. Expected
//! lines are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
package p;
  typedef struct packed { logic [7:0] a; logic [3:0] b; } pair_t;
  typedef enum logic [1:0] {RED, GREEN, BLUE} color_e;
  localparam pair_t PA = '{a: 8'h11, b: 4'h2}, PB = '{default: '1};
endpackage
module leaf #(
  parameter int W = 8,
  parameter int ID = 0,
  parameter struct packed { logic [3:0] hi; logic [3:0] lo; } S0 = '{hi: 4'h1, lo: 4'h2},
                                                             S1 = '{default: 4'hf},
  parameter logic [W-1:0] MASK = '1,
  parameter p::color_e COL = p::GREEN
) ();
  typedef logic [W-1:0] word_t;
  typedef struct packed { word_t w; logic [1:0] t; } rec_t;
  localparam word_t LOCAL = '1;
  localparam rec_t REC = '{w: ID + 3, t: 2'b10};
  localparam rec_t REC2 = '{default: '1};
  localparam p::pair_t NAMED = '{a: 8'hA0 + ID, default: '0};
  for (genvar i = 0; i < 2; i++) begin : g
    localparam logic [W-1:0] STEP = MASK - i;
  end
  rec_t v;
  initial begin
    v = REC;
    v = '{default: '0, t: REC.t};
    #1 $display("T|%m W=%0d S0=%h.%h S1=%h MASK=%h bits=%0d/%0d/%0d LOCAL=%h REC=%h/%h REC2=%h NAMED=%h.%h COL=%s STEP=%h,%h v=%h",
                W, S0.hi, S0.lo, S1, MASK, $bits(MASK), $bits(REC), $bits(LOCAL), LOCAL, REC.w, REC.t,
                REC2, NAMED.a, NAMED.b, COL.name(), g[0].STEP, g[1].STEP, v);
  end
endmodule
module top;
  localparam p::pair_t TA = p::PA, TB = p::PB;
  leaf #(.W(8), .ID(0)) a ();
  leaf #(.W(16), .ID(1), .COL(p::BLUE)) b ();
  leaf #(.W(4), .ID(2), .S1('{hi: 4'h5, lo: 4'ha})) c ();
  for (genvar i = 0; i < 2; i++) begin : arr
    leaf #(.W(6 + i), .ID(3 + i), .MASK(5)) u ();
  end
  initial begin
    #2 $display("T|top TA=%h.%h TB=%h", TA.a, TA.b, TB);
    $finish;
  end
endmodule
"#;

const EXPECTED: [&str; 6] = [
    "T|top.a W=8 S0=1.2 S1=ff MASK=ff bits=8/10/8 LOCAL=ff REC=03/2 REC2=3ff NAMED=a0.0 COL=GREEN STEP=ff,fe v=002",
    "T|top.b W=16 S0=1.2 S1=ff MASK=ffff bits=16/18/16 LOCAL=ffff REC=0004/2 REC2=3ffff NAMED=a1.0 COL=BLUE STEP=ffff,fffe v=00002",
    "T|top.c W=4 S0=1.2 S1=5a MASK=f bits=4/6/4 LOCAL=f REC=5/2 REC2=3f NAMED=a2.0 COL=GREEN STEP=f,e v=02",
    "T|top.arr[0].u W=6 S0=1.2 S1=ff MASK=05 bits=6/8/6 LOCAL=3f REC=06/2 REC2=ff NAMED=a3.0 COL=GREEN STEP=05,04 v=02",
    "T|top.arr[1].u W=7 S0=1.2 S1=ff MASK=05 bits=7/9/7 LOCAL=7f REC=07/2 REC2=1ff NAMED=a4.0 COL=GREEN STEP=05,04 v=002",
    "T|top TA=11.2 TB=fff",
];

fn lines(text: impl Iterator<Item = String>) -> Vec<String> {
    let mut lines: Vec<String> = text.filter(|l| l.starts_with("T|")).collect();
    lines.sort_unstable();
    lines
}

fn expected() -> Vec<String> {
    lines(EXPECTED.iter().map(|l| l.to_string()))
}

#[test]
fn shared_parameter_types_resolve_per_instance() {
    let sim = simulate(SRC, 10).expect("simulate");
    assert_eq!(
        lines(sim.output.iter().map(|o| o.message.clone())),
        expected()
    );
}

#[test]
fn shared_parameter_types_survive_the_cache_and_an_artifact() {
    let dir = std::env::temp_dir().join(format!("xezim_param_types_721_{}", std::process::id()));
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
        assert_eq!(lines(direct.lines().map(str::to_string)), expected());
    }
    // A cold design cache, then the cached design.
    for _ in 0..2 {
        let cached = run(&["--cache-dir", "cache", "design.sv"], false);
        assert_eq!(lines(cached.lines().map(str::to_string)), expected());
    }
    run(
        &["--compile", "--no-cache", "design.sv", "-o", "design.xez"],
        false,
    );
    let loaded = run(&["--simulate", "design.xez"], false);
    assert_eq!(lines(loaded.lines().map(str::to_string)), expected());
    let _ = std::fs::remove_dir_all(&dir);
}
