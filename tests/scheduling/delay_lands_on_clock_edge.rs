//! De facto §4.5 interleave: a process whose `#delay` expires exactly at a
//! clock-toggle time resumes and runs to its NEXT timing control BEFORE the
//! clock toggles. §4.7 leaves the interleave open, but every major
//! implementation runs the resumed process first, so:
//!   * a `@(posedge clk)` reached after the wake catches THIS slot's edge,
//!   * a `clk` read after the wake sees the PRE-toggle value.
//! xezim fired its clock generators before the wheel drain, so a task ending
//! in `#N` that landed on a posedge shifted the caller's whole
//! `repeat(M) @(posedge clk)` by one period (+1000ps on the reporting
//! design's reset sequence). Generators now fire once per timestep AFTER the
//! first wheel drain. Every expectation below is reference-simulator
//! verified, for both the recognized clock-gen shape and a plain
//! wheel-scheduled `forever #5` toggle.

use xezim::simulate;

fn msgs(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.clone())
        .collect()
}

#[test]
fn task_final_delay_landing_on_posedge_is_caught() {
    let out = msgs(
        r#"
module tb_top;
    bit clk;
    initial clk = 0;
    always #5 clk = ~clk;
    task do_delay();
        int wait_time;
        begin
            repeat(4) @(posedge clk);
            wait_time = 40;
            #wait_time
            ;
        end
    endtask
    initial begin
        do_delay();
        repeat(3) @(posedge clk);
        $display("T_%0t", $time);
    end
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "T_95"),
        "repeat after coincident task-return must catch the same-slot edge \
         (95), not skip it (105): {:?}",
        out
    );
}

#[test]
fn coincident_wake_ordering_matrix() {
    let out = msgs(
        r#"
module tb_top;
  int pass=0, fail=0;
  bit clk;
  initial clk = 0;
  always #5 clk = ~clk;
  bit clk2;
  initial begin clk2 = 0; forever #5 clk2 = ~clk2; end
`define CHK(e,m) if (e) pass++; else begin fail++; $display("AFAIL: %s", m); end
  initial begin
    #40; #35;
    `CHK(clk == 1'b0, "A2 pre-toggle read")
    @(posedge clk);
    `CHK($time == 75, "A2b same-slot posedge")
    #35;
    `CHK(clk == 1'b1, "A3 pre-toggle negedge read")
    @(negedge clk);
    `CHK($time == 110, "A3b same-slot negedge")
  end
  initial begin
    #75;
    `CHK(clk2 == 1'b0, "A4 wheel-clock pre-toggle read")
    @(posedge clk2);
    `CHK($time == 75, "A4b same-slot wheel posedge")
  end
  initial begin
    #70; @(posedge clk);
    `CHK($time == 75, "A6 pre-parked waiter")
  end
  initial begin
    #75; #0; @(posedge clk);
    `CHK($time == 85, "A7 post-#0 wait")
  end
  int nposs;
  always @(posedge clk) nposs++;
  initial begin
    #101;
    `CHK(nposs == 10, "A9 posedge count")
  end
  initial begin
    #200;
    if (fail == 0) $display("AUDIT_EDGE_PASS");
    else $display("AUDIT_EDGE_FAIL");
  end
endmodule
"#,
    );
    assert!(
        out.iter().any(|m| m == "AUDIT_EDGE_PASS"),
        "coincident-wake ordering drifted from the reference: {:?}",
        out
    );
}
