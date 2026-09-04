//! §19.3 covergroups declared inside classes. A class-body covergroup
//! implicitly declares a variable of its name; `cg = new` in the constructor
//! instantiates it; `cg.sample()` reads the OBJECT's properties (also when
//! sampled from outside through `obj.cg`); a derived class that redeclares
//! `cg` gets its own coverpoints; constructor formals reach the bins; and
//! `$get_coverage` averages the covergroup types; `option.auto_bin_max` and
//! `cg::type_option` are honoured. None of it worked: the
//! class-body covergroup was never registered, the implicit property did not
//! exist (the handle stayed x), and covergroup / class handles shared one
//! integer namespace, so class object 1 was dispatched as covergroup 1.
//! Expected values are the reference simulator's.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
class base;
  bit [3:0] v;
  covergroup cg; cp: coverpoint v; endgroup
  function new(); cg = new; endfunction
  virtual function void do_sample(); cg.sample(); endfunction
  function real rep(); return cg.get_inst_coverage(); endfunction
endclass
class ext extends base;
  bit [1:0] w;
  covergroup cg; cpw: coverpoint w; endgroup
  function new(); super.new(); cg = new; endfunction
  virtual function void do_sample(); cg.sample(); endfunction
endclass
class withargs;
  covergroup cg with function sample(bit [3:0] x); cpx: coverpoint x; endgroup
  function new(); cg = new; endfunction
endclass
module top;
  bit [3:0] mv;
  covergroup mcg (int lo, int hi); cp: coverpoint mv { bins r = {[lo:hi]}; } endgroup
  mcg m = new(2, 5);
  bit [3:0] ov;
  covergroup ocg; type_option.goal = 80; cp: coverpoint ov { option.auto_bin_max = 2; } endgroup
  ocg o = new;
  initial begin
    base b = new; ext e = new; withargs wa = new;
    b.v = 2; b.do_sample();
    $display("BASE internal=%0.1f external=%0.1f", b.rep(), b.cg.get_inst_coverage());
    b.v = 5; b.cg.sample();
    $display("BASE after-external-sample=%0.1f", b.rep());
    e.w = 1; e.do_sample();
    $display("EXT redeclared=%0.1f", e.cg.get_inst_coverage());
    wa.cg.sample(4); wa.cg.sample(5);
    $display("ARGS sample-args=%0.1f", wa.cg.get_inst_coverage());
    mv = 3; m.sample();
    $display("CTOR arg-bins=%0.1f", m.get_inst_coverage());
    ov = 1; o.sample();
    ocg::type_option.weight = 5;
    $display("OPTS auto_bin_max=%0.1f type_weight=%0d type_goal=%0d", o.get_inst_coverage(), ocg::type_option.weight, ocg::type_option.goal);
    $display("GLOBAL %0.1f", $get_coverage());
    $finish;
  end
endmodule
"#;

fn run(jit: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("class_covergroups");
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
    for want in [
        "BASE internal=6.2 external=6.2",
        "BASE after-external-sample=12.5",
        "EXT redeclared=25.0",
        "ARGS sample-args=12.5",
        "CTOR arg-bins=100.0",
        // §19.7.1: auto_bin_max 2 on a 4-bit point puts value 1 in the low
        // half (50%); type_option reads the body's goal and a scoped write.
        "OPTS auto_bin_max=50.0 type_weight=5 type_goal=80",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
    // Five covergroup types (base::cg 12.5, ext::cg 25, withargs::cg 12.5,
    // mcg 100, ocg 50): $get_coverage is their mean.
    assert!(text.contains("GLOBAL 40.0"), "global coverage:\n{text}");
}

#[test]
fn class_body_covergroups_sample_report_and_derive() {
    check(&run(false));
}

#[test]
fn class_body_covergroups_sample_report_and_derive_jit() {
    check(&run(true));
}
