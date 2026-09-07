//! Concurrent assertions: (1) an `assert property` inside an INLINED
//! instance — a sub-module or an interface — registers a site and fires
//! (it used to vanish: no site, no failure, ever); (2) a `cover property`
//! is tallied as cover, a miss is not a failure; (3) a sequence consequent
//! (`a |-> a ##1 b ##1 c`) samples its leading term at the implication
//! tick and walks the rest cycle by cycle (the chain used to be read as one
//! delay count); (4) a NAMED sequence instance in a consequent expands (an
//! unclocked `sequence s; ... endsequence` body was never recorded).
//! Every expected outcome here is the reference simulator's.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
interface ifc(input bit clk);
  bit req, ack;
  ap: assert property (@(posedge clk) req |=> ack) else $display("IFACE-FAIL t=%0t", $time);
endinterface
module sub(input bit clk, input bit req, input bit ack);
  ap: assert property (@(posedge clk) req |=> ack) else $display("SUBMOD-FAIL t=%0t", $time);
endmodule
module top;
  bit clk = 0;
  bit a, b, c, req, ack, cv;
  always #5 clk = ~clk;
  ifc i(clk);
  sub u(clk, req, ack);
  sequence s; a ##1 b ##1 c; endsequence
  sequence s2; b ##1 !b; endsequence
  seq_inline: assert property (@(posedge clk) a |-> a ##1 b ##1 c) else $display("INLINE-FAIL t=%0t", $time);
  seq_named:  assert property (@(posedge clk) a |-> s) else $display("NAMED-FAIL t=%0t", $time);
  seq_next:   assert property (@(posedge clk) a |=> s2) else $display("NEXT-FAIL t=%0t", $time);
  cov: cover property (@(posedge clk) cv) $display("COVER-HIT t=%0t", $time);
  initial begin
    @(posedge clk); a = 1; req = 1; i.req = 1;
    @(posedge clk); b = 1; a = 0; req = 0; i.req = 0; cv = 1;
    @(posedge clk); b = 0; c = 1; cv = 0;
    @(posedge clk); c = 0;
    #20 $display("DONE");
    $finish;
  end
endmodule
"#;

fn run(jit: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sva_instances_and_sequences");
    std::fs::create_dir_all(&dir).unwrap();
    let sv = dir.join(if jit { "t_jit.sv" } else { "t_default.sv" });
    std::fs::write(&sv, DESIGN).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--simulate", "-s", "top", "--no-cache", sv.to_str().unwrap()]);
    if jit {
        cmd.env("XEZIM_JIT", "1");
    }
    let output = cmd.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    text
}

fn check(text: &str) {
    // The instance assertions fire once each: req at the t=15 tick, ack never.
    assert!(text.contains("IFACE-FAIL t=25"), "interface assertion did not fire:\n{text}");
    assert!(text.contains("SUBMOD-FAIL t=25"), "sub-module assertion did not fire:\n{text}");
    assert_eq!(text.matches("IFACE-FAIL").count(), 1, "interface assertion count:\n{text}");
    // The three sequence properties all pass on this stimulus.
    for bad in ["INLINE-FAIL", "NAMED-FAIL", "NEXT-FAIL"] {
        assert!(!text.contains(bad), "unexpected `{bad}`:\n{text}");
    }
    assert!(text.contains("COVER-HIT t=25"), "cover property did not report its hit:\n{text}");
    // Six sites: 2 instance asserts + 3 sequence asserts + 1 cover; the cover
    // is counted as cover, and its misses are not failures.
    assert!(text.contains("assertions: 6 sites (assert=5, assume=0, cover=1)"), "site summary:\n{text}");
    assert!(text.contains("DONE"), "did not finish:\n{text}");
}

#[test]
fn inlined_instance_assertions_sequences_and_cover() {
    check(&run(false));
}

#[test]
fn inlined_instance_assertions_sequences_and_cover_jit() {
    check(&run(true));
}
