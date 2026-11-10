//! §4.4 / §9.7.4: a process that re-arms itself only through `#0`, `wait`,
//! mailboxes or semaphores, and performs a FINITE number of rounds, finishes
//! inside the timestamp it started in, however many rounds that is. The
//! zero-delay stall guard must not split such a phase across timestamps; it
//! includes suspended loop control and every data width when checking
//! progress. Setting `XEZIM_STALL_LIMIT` explicitly opts in to a hard
//! per-process cap instead.

use std::process::Command;

fn run(name: &str, src: &str, limit: Option<&str>) -> String {
    let path = std::env::temp_dir().join(format!("xezim_{name}_{}.sv", std::process::id()));
    std::fs::write(&path, src).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args(["--no-cache", "--simulate", "-s", "top"])
        .arg(&path);
    match limit {
        Some(l) => cmd.env("XEZIM_STALL_LIMIT", l),
        None => cmd.env_remove("XEZIM_STALL_LIMIT"),
    };
    let out = cmd.output().unwrap();
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// `repeat (N) #0` with a counter, sampled one time unit later while a clock
/// keeps a future event scheduled (so the guard could defer instead of abort).
const FINITE_ZERO_LOOP: &str = r#"
module top;
  localparam int N = 250000;
  int spin, sampled;
  bit clk, done;
  initial begin
    fork
      forever #5 clk = ~clk;
      begin repeat (N) begin #0; spin++; end done = 1; end
    join_none
    #1 sampled = spin;
    $display("RESULT sampled=%0d done=%0b", sampled, done);
    $finish;
  end
endmodule
"#;

/// Two processes handing a request/ack pair back and forth with blocking
/// writes and `wait`: every round is a zero-delay re-arm.
const FINITE_WAIT_HANDSHAKE: &str = r#"
module top;
  localparam int N = 120000;
  bit req, ack, clk, done;
  int a, sampled;
  initial begin
    fork
      forever #5 clk = ~clk;
      begin
        repeat (N) begin
          wait (ack == 1'b0); req = 1'b1;
          wait (ack == 1'b1); req = 1'b0;
          a++;
        end
        done = 1;
      end
      begin
        repeat (N) begin
          wait (req == 1'b1); ack = 1'b1;
          wait (req == 1'b0); ack = 1'b0;
        end
      end
    join_none
    #1 sampled = a;
    $display("RESULT sampled=%0d done=%0b", sampled, done);
    $finish;
  end
endmodule
"#;

#[test]
fn finite_zero_delay_loop_stays_in_its_timestamp() {
    let text = run("fin_zero_loop", FINITE_ZERO_LOOP, None);
    assert!(text.contains("RESULT sampled=250000 done=1"), "{text}");
    assert!(!text.contains("no time progress"), "{text}");
    assert!(!text.contains("STALLED"), "{text}");
}

#[test]
fn finite_wait_handshake_stays_in_its_timestamp() {
    let text = run("fin_wait_hs", FINITE_WAIT_HANDSHAKE, None);
    assert!(text.contains("RESULT sampled=120000 done=1"), "{text}");
    assert!(!text.contains("no time progress"), "{text}");
    assert!(!text.contains("STALLED"), "{text}");
}

#[test]
fn explicit_stall_limit_is_a_hard_cap() {
    // Opt-in strict mode: the same finite loop is cut off at the limit.
    let text = run("fin_zero_strict", FINITE_ZERO_LOOP, Some("1000"));
    assert!(!text.contains("RESULT sampled=250000"), "{text}");
    assert!(
        text.contains("no time progress") || text.contains("STALLED"),
        "{text}"
    );
}

#[test]
fn stateless_zero_delay_livelock_is_still_reported() {
    // Nothing ever changes, so even the progress-aware default must trip.
    let text = run(
        "live_no_state",
        "module top; initial forever #0; initial #5 $display(\"LATE\"); endmodule",
        None,
    );
    assert!(text.contains("STALLED"), "{text}");
}

/// Edge / event / NBA relays: each needs more same-timestamp rounds than the
/// fixed caps on the waiter-drain loops and the NBA cascade (10_000 passes,
/// 8 generations), and each round changes state, so it must complete.
fn relay(decls: &str, procs: &str, count: &str, n: u32) -> String {
    format!(
        r#"
module top;
  localparam int N = {n};
  bit clk;
  {decls}
  initial begin
    fork
      forever #5 clk = ~clk;
      {procs}
    join_none
    #1 $display("RESULT %0d/%0d", {count}, N);
    $finish;
  end
endmodule
"#
    )
}

fn assert_relay(name: &str, src: String, n: u32) {
    let text = run(name, &src, None);
    assert!(text.contains(&format!("RESULT {n}/{n}")), "{name}: {text}");
    assert!(!text.contains("no time progress"), "{name}: {text}");
    assert!(!text.contains("STALLED"), "{name}: {text}");
}

#[test]
fn blocking_event_ping_pong_stays_in_its_timestamp() {
    assert_relay(
        "ev_ping_pong",
        relay(
            "event req, ack; int a;",
            "begin #0; repeat (N) begin -> req; @(ack); a++; end end \
             begin repeat (N) begin @(req); -> ack; end end",
            "a",
            30000,
        ),
        30000,
    );
}

#[test]
fn nonblocking_event_ping_pong_stays_in_its_timestamp() {
    // §15.5.2: `->>` flushes with the NBA region of the SAME timestamp.
    assert_relay(
        "nb_ev_ping_pong",
        relay(
            "event req, ack; int a;",
            "begin repeat (N) begin ->> req; @(ack); a++; end end \
             begin repeat (N) begin @(req); ->> ack; end end",
            "a",
            30000,
        ),
        30000,
    );
    assert_relay(
        "mixed_ev_ping_pong",
        relay(
            "event req, ack; int a;",
            "begin #0; repeat (N) begin -> req; @(ack); a++; end end \
             begin repeat (N) begin @(req); ->> ack; end end",
            "a",
            30000,
        ),
        30000,
    );
}

#[test]
fn edge_and_value_change_relays_stay_in_their_timestamp() {
    assert_relay(
        "var_ping_pong",
        relay(
            "bit go, rdy; int a;",
            "begin #0; repeat (N) begin go = ~go; @(rdy); a++; end end \
             begin repeat (N) begin @(go); rdy = ~rdy; end end",
            "a",
            30000,
        ),
        30000,
    );
    assert_relay(
        "posedge_negedge_relay",
        relay(
            "bit req, ack; int a;",
            "begin #0; repeat (N) begin req = 1; @(posedge ack); req = 0; @(negedge ack); a++; end end \
             begin repeat (N) begin @(posedge req); ack = 1; @(negedge req); ack = 0; end end",
            "a",
            30000,
        ),
        30000,
    );
}

#[test]
fn nba_driven_relays_stay_in_their_timestamp() {
    assert_relay(
        "nba_wait_relay",
        relay(
            "bit req, ack; int a;",
            "begin repeat (N) begin wait (ack == 0); req <= 1; wait (ack == 1); req <= 0; a++; end end \
             begin repeat (N) begin wait (req == 1); ack <= 1; wait (req == 0); ack <= 0; end end",
            "a",
            30000,
        ),
        30000,
    );
    // Past the 8-generation cascade limit; the state keeps changing.
    assert_relay(
        "nba_edge_relay",
        relay(
            "bit go, rdy; int a;",
            "begin #0; repeat (N) begin go <= ~go; @(rdy); a++; end end \
             begin repeat (N) begin @(go); rdy <= ~rdy; end end",
            "a",
            30000,
        ),
        30000,
    );
}

#[test]
fn explicit_cascade_limit_is_a_hard_cap() {
    let src = relay(
        "bit go, rdy; int a;",
        "begin #0; repeat (N) begin go <= ~go; @(rdy); a++; end end \
         begin repeat (N) begin @(go); rdy <= ~rdy; end end",
        "a",
        30000,
    );
    let path = std::env::temp_dir().join(format!("xezim_casc_cap_{}.sv", std::process::id()));
    std::fs::write(&path, &src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "--simulate", "-s", "top"])
        .arg(&path)
        .env("XEZIM_CASCADE_LIMIT", "8")
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("did not converge"), "{text}");
}

#[test]
fn edge_exec_rescan_ping_pong_stays_in_its_timestamp() {
    const SRC: &str = r#"
module top;
  localparam int N = 250000;
  reg a = 0, b = 0;
  int count = 0;
  always @(a) begin
    b = ~b;
    count++;
  end
  always @(b) begin
    if (count < N) a = ~a;
  end
  initial begin
    #0 a = 1;
    #1;
    $display("RESULT count=%0d t=%0t", count, $time);
    $finish;
  end
endmodule
"#;
    let text = run("edge_exec_rescan", SRC, None);
    assert!(text.contains("RESULT count=250000 t=1"), "{text}");
    assert!(!text.contains("STALLED"), "{text}");

    // Strict mode cut off at limit
    let strict_text = run("edge_exec_rescan_strict", SRC, Some("1000"));
    assert!(
        !strict_text.contains("RESULT count=250000"),
        "{strict_text}"
    );
    assert!(strict_text.contains("STALLED"), "{strict_text}");
}

#[test]
fn timing_free_forever_with_break_stays_in_its_timestamp() {
    const SRC: &str = r#"
module top;
  int i = 0;
  initial begin
    forever begin
      i++;
      if (i == 250000) break;
    end
    #1;
    $display("RESULT i=%0d t=%0t", i, $time);
    $finish;
  end
endmodule
"#;
    let text = run("forever_break", SRC, None);
    assert!(text.contains("RESULT i=250000 t=1"), "{text}");
    assert!(!text.contains("STALLED"), "{text}");

    // Strict mode cut off at limit
    let strict_text = run("forever_break_strict", SRC, Some("1000"));
    assert!(!strict_text.contains("RESULT i=250000"), "{strict_text}");
    assert!(strict_text.contains("STALLED"), "{strict_text}");
}

#[test]
fn empty_finite_repeat_keeps_its_hidden_continuation_counter() {
    let source = r#"
module top;
 initial begin
   repeat(200001) #0;
   $display("RESULT empty=1 t=%0t",$time);
 end
 initial #1 $finish;
endmodule
"#;
    let first = run("empty_finite_repeat", source, None);
    let second = run("empty_finite_repeat", source, None);
    assert!(first.contains("RESULT empty=1 t=0"), "{first}");
    assert!(!first.contains("STALLED"), "{first}");
    assert_eq!(first, second, "equal seeds must preserve scheduling");
}

#[test]
fn wide_progress_above_bit_63_keeps_an_nba_relay_running() {
    let source = r#"
module top;
 bit [127:0] word_value=0;
 always @(word_value) if(word_value[127:64]<30000)
   word_value<=word_value+128'h1_0000000000000000;
 initial #0 word_value=128'h1_0000000000000000;
 initial begin #1; $display("RESULT wide=%0d",word_value[127:64]); $finish; end
endmodule
"#;
    let text = run("wide_phase_progress", source, None);
    assert!(text.contains("RESULT wide=30000"), "{text}");
    assert!(!text.contains("STALLED"), "{text}");
}

#[test]
fn local_collection_progress_keeps_a_finite_phase_running() {
    let source = r#"
module top;
 task automatic phase();
   int values[$];
   while(values.size()<200001) begin #0; values.push_back(1); end
   $display("RESULT queue=%0d t=%0t",values.size(),$time);
 endtask
 initial phase();
 initial #1 $finish;
endmodule
"#;
    let text = run("local_collection_progress", source, None);
    assert!(text.contains("RESULT queue=200001 t=0"), "{text}");
    assert!(!text.contains("STALLED"), "{text}");
}

#[test]
fn identical_signal_and_property_writes_do_not_mask_livelock() {
    for (name, source) in [
        (
            "same_signal_write",
            "module top; bit flag; initial forever begin #0; flag=0; end endmodule",
        ),
        (
            "same_property_write",
            "class C; int value; function void clear(); value=0; endfunction endclass module top; C c; initial begin c=new; forever begin #0; c.clear(); end end endmodule",
        ),
    ] {
        let text = run(name, source, None);
        assert!(text.contains("STALLED"), "{text}");
    }
}
