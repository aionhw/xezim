//! §13.5.2: an element, bit or part-select write through a `ref` formal
//! lands on the caller's variable, and the matching read inside the callee
//! sees it. Shapes: packed-dimension formals (typedef and `logic`, one and
//! two packed dimensions, automatic and static tasks) and a two-dimensional
//! unpacked array. These ran on the interpreter's task-call path, where the
//! selected write went to a dead local copy while the selected read went to
//! the caller's storage — `a[1] = 9` read back x inside the task and the
//! caller never saw it. One-dimensional unpacked arrays and queues (copy-in
//! plus copy-back) are covered too so that path stays intact.
use std::path::PathBuf;
use std::process::Command;

const DESIGN: &str = r#"
typedef bit [6:0] u7_t;
module top;
  task automatic td1(ref u7_t [1:0] a); a[1] = 9; $display("TD1 in %0d", a[1]); endtask
  task automatic td2(ref u7_t [1:0][1:0] a); a[0][1] = 9; $display("TD2 in %0d", a[0][1]); endtask
  task automatic lg2(ref logic [1:0][1:0][6:0] a); a[0][1] = 9; endtask
  task st1(ref logic [1:0][6:0] a); a[1] = 9; $display("ST1 in %0d", a[1]); endtask
  task automatic part(ref logic [15:0] a); a[7:0] = 8'h5a; a[15] = 1'b1; endtask
  task automatic un2(ref int a[2][2]); a[0][1] = 9; $display("UN2 in %0d", a[0][1]); endtask
  task automatic un1(ref int a[4]); a[1] = 9; endtask
  task automatic q1(ref int q[$]); q.push_back(9); endtask
  task automatic dl(ref u7_t [1:0] a); #1 a[0] = 3; endtask
  u7_t [1:0] v1; u7_t [1:0][1:0] v2; logic [1:0][1:0][6:0] l2; logic [1:0][6:0] s1;
  logic [15:0] p16; int u2[2][2]; int u1[4]; int q[$]; u7_t [1:0] d1;
  initial begin
    td1(v1); $display("TD1 out %0d", v1[1]);
    td2(v2); $display("TD2 out %0d", v2[0][1]);
    lg2(l2); $display("LG2 out %0d", l2[0][1]);
    st1(s1); $display("ST1 out %0d", s1[1]);
    p16 = 0; part(p16); $display("PART out %h", p16);
    un2(u2); $display("UN2 out %0d", u2[0][1]);
    un1(u1); $display("UN1 out %0d", u1[1]);
    q1(q); $display("Q1 out %0d", q.size());
    dl(d1); $display("DL out %0d", d1[0]);
  end
endmodule
"#;

fn run(jit: bool) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ref_formal_element_write");
    std::fs::create_dir_all(&dir).unwrap();
    // One file per variant: the two variants run on parallel test threads
    // and a shared file can be truncated under the other run.
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
        "TD1 in 9", "TD1 out 9", "TD2 in 9", "TD2 out 9", "LG2 out 9", "ST1 in 9", "ST1 out 9",
        "PART out 805a", "UN2 in 9", "UN2 out 9", "UN1 out 9", "Q1 out 1", "DL out 3",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
}

#[test]
fn selected_writes_through_ref_formals_reach_the_caller() {
    check(&run(false));
}

#[test]
fn selected_writes_through_ref_formals_reach_the_caller_jit() {
    check(&run(true));
}
