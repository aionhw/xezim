//! IEEE 1800-2023 §6.8, §6.18, §6.19, §7.2 (#721): the instances of a module
//! share one copy of each declared type; what the type means stays per
//! instance. Vectors and arrays sized by a parameter, a local typedef, a
//! package struct and enum, and a queue behave by each instance's own
//! parameters, read back after the design is run from a compiled artifact
//! as well. Expected lines are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
package p;
  typedef enum logic [1:0] {RED, GREEN, BLUE} color_e;
  typedef struct packed { logic [7:0] a; logic [3:0] b; } pair_t;
endpackage
module leaf #(parameter int W = 8, parameter int ID = 0) ();
  logic [W-1:0] r0, r1;
  struct packed { logic [3:0] x; logic [W-1:0] y; } inl;
  p::pair_t named;
  p::color_e col;
  typedef logic [W-1:0] word_t;
  word_t lw;
  int unsigned q [$];
  logic [W-1:0] mem [2];
  initial begin
    r0 = '1;
    r1 = r0 >> 1;
    named = '{a: 8'hA0 + ID, b: 4'h3};
    col = p::color_e'(ID % 3);
    lw = '1;
    mem[0] = r1;
    mem[1] = ~r1;
    q.push_back(ID);
    q.push_back(W);
    #1 $display("T|%m W=%0d r0=%h r1=%h bits=%0d/%0d/%0d na=%h nb=%h col=%s lw=%h m0=%h m1=%h q=%0d,%0d",
                W, r0, r1, $bits(r0), $bits(inl), $bits(lw), named.a, named.b, col.name(), lw,
                mem[0], mem[1], q[0], q[1]);
  end
endmodule
module top;
  leaf #(8, 0) a ();
  leaf #(16, 1) b ();
  leaf #(4, 2) c ();
  for (genvar i = 0; i < 2; i++) begin : g
    leaf #(6 + i, 3 + i) u ();
  end
  initial #2 $finish;
endmodule
"#;

const EXPECTED: [&str; 5] = [
    "T|top.a W=8 r0=ff r1=7f bits=8/12/8 na=a0 nb=3 col=RED lw=ff m0=7f m1=80 q=0,8",
    "T|top.b W=16 r0=ffff r1=7fff bits=16/20/16 na=a1 nb=3 col=GREEN lw=ffff m0=7fff m1=8000 q=1,16",
    "T|top.c W=4 r0=f r1=7 bits=4/8/4 na=a2 nb=3 col=BLUE lw=f m0=7 m1=8 q=2,4",
    "T|top.g[0].u W=6 r0=3f r1=1f bits=6/10/6 na=a3 nb=3 col=RED lw=3f m0=1f m1=20 q=3,6",
    "T|top.g[1].u W=7 r0=7f r1=3f bits=7/11/7 na=a4 nb=3 col=GREEN lw=7f m0=3f m1=40 q=4,7",
];

fn lines(text: impl Iterator<Item = String>) -> Vec<String> {
    let mut lines: Vec<String> = text.filter(|l| l.starts_with("T|")).collect();
    lines.sort_unstable();
    lines
}

#[test]
fn shared_declared_types_resolve_per_instance() {
    let sim = simulate(SRC, 10).expect("simulate");
    assert_eq!(
        lines(sim.output.iter().map(|o| o.message.clone())),
        EXPECTED
    );
}

/// The same design from the design cache and from a compiled artifact: a
/// loaded design owns a type per signal and must print the same lines.
#[test]
fn shared_declared_types_survive_the_cache_and_an_artifact() {
    let dir = std::env::temp_dir().join(format!("xezim_decl_types_721_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let src = dir.join("design.sv");
    let art = dir.join("design.xez");
    std::fs::write(&src, SRC).expect("write source");
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run xezim");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let direct = run(&["--no-cache", "design.sv"]);
    assert_eq!(lines(direct.lines().map(str::to_string)), EXPECTED);
    // A cold design cache, then the cached design.
    for _ in 0..2 {
        let cached = run(&["--cache-dir", "cache", "design.sv"]);
        assert_eq!(lines(cached.lines().map(str::to_string)), EXPECTED);
    }
    run(&["--compile", "--no-cache", "design.sv", "-o", "design.xez"]);
    assert!(art.is_file());
    let loaded = run(&["--simulate", "design.xez"]);
    assert_eq!(lines(loaded.lines().map(str::to_string)), EXPECTED);
    let _ = std::fs::remove_dir_all(&dir);
}
