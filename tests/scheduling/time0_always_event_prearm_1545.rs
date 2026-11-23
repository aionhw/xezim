//! IEEE 1800-2023 §9.2 / §6.8 (#1545): unconditional event-headed `always`
//! processes arm before `initial` blocks run at time 0 so they catch time-0
//! `initial` assignments, while declaration initializers follow SystemVerilog
//! pre-time-0 semantics (no time-0 event) by default even in `.v` files.

use std::path::PathBuf;
use std::process::Command;
use xezim::simulate;

fn lines_of(src: &str, max_time: u64) -> Vec<String> {
    simulate(src, max_time)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .collect()
}

const FIVE_CLOCKS_SV: &str = r#"
`timescale 1ns/1ns
module t;
  reg c1, c2, c3, c4, c5;
  int n1, n2, n3, n4, n5;
  initial begin n1 = 0; n2 = 0; n3 = 0; n4 = 0; n5 = 0; end
  initial c1 = 1'b0;
  always @(c1) #10 c1 = ~c1;
  always @(c2) #10 c2 = ~c2;
  initial c2 = 1'b0;
  initial c3 = 1'b0;
  always @(c3) #10 c3 <= ~c3;
  initial c4 = 1'b0;
  always begin @(c4) #10 c4 = ~c4; end
  initial c5 = 1'b0;
  always @(*) #10 c5 <= ~c5;
  always @(posedge c1) n1++;
  always @(posedge c2) n2++;
  always @(posedge c3) n3++;
  always @(posedge c4) n4++;
  always @(posedge c5) n5++;
  initial begin
    #2000;
    $display("n1=%0d n2=%0d n3=%0d n4=%0d n5=%0d", n1, n2, n3, n4, n5);
    $finish;
  end
endmodule
"#;

#[test]
fn event_headed_always_processes_prearm_for_initial_writes() {
    assert_eq!(lines_of(FIVE_CLOCKS_SV, 5000), ["n1=1 n2=1 n3=100 n4=1 n5=100"]);
}

#[test]
fn event_headed_always_processes_prearm_under_proc_fsm_and_v_keeps_sv_inits() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("t1545_prearm");
    std::fs::create_dir_all(&dir).unwrap();

    let sv_path = dir.join("t.sv");
    std::fs::write(&sv_path, FIVE_CLOCKS_SV).unwrap();
    let out_fsm = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "t", "--no-cache", sv_path.to_str().unwrap()])
        .env("XEZIM_PROC_FSM", "1")
        .output()
        .unwrap();
    let txt_fsm = format!(
        "{}{}",
        String::from_utf8_lossy(&out_fsm.stdout),
        String::from_utf8_lossy(&out_fsm.stderr)
    );
    assert!(out_fsm.status.success(), "FSM run failed:\n{txt_fsm}");
    assert!(
        txt_fsm.contains("n1=1 n2=1 n3=100 n4=1 n5=100"),
        "expected all 5 clocks under XEZIM_PROC_FSM=1:\n{txt_fsm}"
    );

    let v_src = r#"
`timescale 1ns/1ns
module t;
  reg c1 = 1'b0;
  reg c2 = 1'b0;
  reg c3 = 1'b0;
  reg a = 1'b0;
  reg b = 1'b1;
  reg [3:0] v = 4'd5;
  integer n1 = 0, n2 = 0, n3 = 0;
  time any_a, neg_a, pos_b, any_v;
  always @(*) #10 c1 <= ~c1;
  always @(c2) #10 c2 <= ~c2;
  always @(c3) #10 c3 = ~c3;
  always @(posedge c1) n1 = n1 + 1;
  always @(posedge c2) n2 = n2 + 1;
  always @(posedge c3) n3 = n3 + 1;
  always @(a) any_a = $time;
  always @(negedge a) neg_a = $time;
  always @(posedge b) pos_b = $time;
  always @(v) any_v = $time;
  initial begin
    #2000;
    $display("n1=%0d n2=%0d n3=%0d", n1, n2, n3);
    $display("any_a=%0d neg_a=%0d pos_b=%0d any_v=%0d", any_a, neg_a, pos_b, any_v);
    $finish;
  end
endmodule
"#;
    let v_path = dir.join("t.v");
    std::fs::write(&v_path, v_src).unwrap();
    let out_v = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "t", "--no-cache", v_path.to_str().unwrap()])
        .output()
        .unwrap();
    let txt_v = format!(
        "{}{}",
        String::from_utf8_lossy(&out_v.stdout),
        String::from_utf8_lossy(&out_v.stderr)
    );
    assert!(out_v.status.success(), ".v run failed:\n{txt_v}");
    assert!(
        txt_v.contains("n1=0 n2=0 n3=0") && txt_v.contains("any_a=x neg_a=x pos_b=x any_v=x"),
        "expected SV declaration init semantics on .v:\n{txt_v}"
    );
}

#[test]
fn nested_child_and_negative_guards_for_time0_prearm() {
    let src = r#"
`timescale 1ns/1ns
module child;
  reg cc;
  initial cc = 1'b0;
  always @(cc) begin $display("CHILD_EV"); #5; end
endmodule
module top;
  reg c, d, m, p1, p2;
  int d_hits = 0, m_hits = 0, p1_hits = 0, p2_hits = 0;

  initial begin
    $display("INIT_1");
    c = 1'b0;
    d = 1'b0;
    p1 = 1'b0;
    p2 = 1'b0;
  end

  always begin begin @(c) $display("C_NEST"); end #10; end
  always begin $display("D_BEFORE_EV"); @(d) d_hits = d_hits + 1; #10; end

  always @(m) begin m_hits = m_hits + 1; #10; end
  initial m = 1'b0;
  always @(m) begin m_hits = m_hits + 1; #10; end

  always @(p1 iff (p1 === 0)) begin p1_hits = p1_hits + 1; wait (p1 === 1); end
  always @(p2) begin p2_hits = p2_hits + 1; wait (p2 === 1); end

  child u();

  initial begin
    #25;
    $display("COUNTS d=%0d m=%0d p1=%0d p2=%0d", d_hits, m_hits, p1_hits, p2_hits);
    $finish;
  end
endmodule
"#;
    let out = lines_of(src, 100);
    let init_idx = out
        .iter()
        .position(|l| l == "INIT_1")
        .expect("missing INIT_1");
    let d_before_idx = out
        .iter()
        .position(|l| l == "D_BEFORE_EV")
        .expect("missing D_BEFORE_EV");
    assert!(
        init_idx < d_before_idx,
        "non-event-headed always jumped before INIT_1: {out:?}"
    );
    assert!(out.iter().any(|l| l == "C_NEST"), "{out:?}");
    assert!(out.iter().any(|l| l == "CHILD_EV"), "{out:?}");
    assert!(
        out.iter().any(|l| l == "COUNTS d=0 m=2 p1=0 p2=0"),
        "{out:?}"
    );
}
