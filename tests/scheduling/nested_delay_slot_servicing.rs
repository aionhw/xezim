//! IEEE 1800-2017 §4.4.2.4: the postponed region (`$monitor`, `$strobe`) and
//! the waveform dump belong to EVERY time slot, including slots consumed by a
//! nested event loop.
//!
//! A `#delay` inside an edge block (`always @(posedge tick) begin #500; end`)
//! runs through `exec_statement`, which advances time with its own nested loop
//! (`run_events_until`). That loop ran processes, applied NBAs, settled and
//! checked edges for each slot it crossed — but ran none of the postponed
//! region for them. Anything that changed in one of those slots was invisible
//! to `$monitor` and to the dump until some LATER slot that happened to be
//! serviced, and because the two are serviced by different mechanisms they
//! disagreed with each other as well as with the change itself.
//!
//! Observed on a real design: a reset released at t=100 was reported by
//! `$monitor` at 102 and by the VCD at 112, while an internal trace of the
//! write itself said 100. The dump was the worse of the two — it held no
//! record for the timestamp at all, so 485 changes were re-attributed to a
//! later one. Timing read off such a wave is simply wrong.
//!
//! Here `sig` changes at t=100, inside the window of a `#500` in an edge
//! block. Both views must say 100.

use xezim::simulate;

const SRC: &str = r#"
`timescale 1ns/1ps
module top;
  logic tick = 0;
  logic sig  = 0;

  always #50 tick = ~tick;          // first posedge at t=50

  // The nested-loop path: a delay inside an edge block.
  always @(posedge tick) begin
    #500;
  end

  initial #100 sig = 1;             // real change, inside that window
  initial #600 sig = 0;

  initial $monitor("MON %0t sig=%b", $time, sig);
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
    #900 $finish;
  end
endmodule
"#;

fn run(tag: &str) -> (Vec<String>, Vec<u64>) {
    // Tests in a group share one process, so the path must be per-test or the
    // three runs clobber each other's dump.
    let mut path = std::env::temp_dir();
    path.push(format!("xezim_slot_service_{}_{}.vcd", tag, std::process::id()));
    let _ = std::fs::remove_file(&path);

    let src = SRC.replace("@VCD@", path.to_str().unwrap());
    // Waveform dumping is opt-in (`--wave` on the CLI); this test compares the
    // dump against $monitor, so it needs it on.
    xezim::compiler::simulator::set_wave_enabled(true);
    let sim = simulate(&src, 100_000_000).expect("simulate failed");

    let mons: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("MON "))
        .collect();

    let text = std::fs::read_to_string(&path).expect("VCD not written");
    let stamps: Vec<u64> = text
        .lines()
        .filter_map(|l| l.strip_prefix('#'))
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let _ = std::fs::remove_file(&path);
    (mons, stamps)
}

/// §21.2.3 — `$monitor` reports the change in the slot it happened in, not in
/// whichever later slot the nested loop finally returned to (550 ns here).
#[test]
fn monitor_sees_a_change_made_inside_a_nested_delay_window() {
    let (mons, _) = run("monitor");
    assert!(
        mons.contains(&"MON 100000 sig=1".to_string()),
        "expected the t=100ns change to be reported at 100000ps, got {mons:?}"
    );
}

/// §21.7 — the dump has a record for that timestamp. Previously the VCD went
/// straight from #0 to #550000 and the change was re-dated to 550 ns.
#[test]
fn dump_has_a_record_for_a_slot_inside_a_nested_delay_window() {
    let (_, stamps) = run("record");
    assert!(
        stamps.contains(&100_000),
        "VCD has no #100000 record; timestamps were {stamps:?}"
    );
}

/// The clock kept toggling throughout the delay window, so those slots must be
/// in the dump too — they were ALL missing, not just the one under test.
#[test]
fn dump_keeps_slots_crossed_by_the_nested_loop() {
    let (_, stamps) = run("slots");
    for t in [150_000u64, 200_000, 250_000, 300_000] {
        assert!(
            stamps.contains(&t),
            "VCD lost slot #{t} inside the nested delay window; got {stamps:?}"
        );
    }
}

/// The `$monitor` and the waveform dump must agree with EACH OTHER, at every
/// change, not merely each be "close".
///
/// Two separate holes produced two different lags, which is why the two views
/// disagreed with each other as well as with the change:
///   * slots crossed INSIDE `run_events_until` serviced neither, and
///   * the slot reached by jumping `self.time` to the delay's target serviced
///     `$monitor` but not the dump.
///
/// `b` here changes exactly AT the target of the `#500`, which is the second
/// hole specifically.
const AGREEMENT: &str = r#"
`timescale 1ns/1ps
module top;
  logic tick = 0;
  logic a = 0, b = 0, c = 0;

  always #50 tick = ~tick;
  always @(posedge tick) begin
    #500;                 // nested window, target at 550ns
  end

  initial #100 a = 1;     // inside the window
  initial #550 b = 1;     // exactly AT the target
  initial #600 c = 1;     // after it

  // $time is excluded from change detection, so the watched
  // signals must be in the argument list or the monitor never
  // re-fires after its first print.
  initial $monitor("MON %0t a=%b b=%b c=%b", $time, a, b, c);
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
    #900 $finish;
  end
endmodule
"#;

#[test]
fn monitor_and_dump_agree_on_every_change_time() {
    let mut path = std::env::temp_dir();
    path.push(format!("xezim_slot_agree_{}.vcd", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let src = AGREEMENT.replace("@VCD@", path.to_str().unwrap());
    let sim = simulate(&src, 100_000_000).expect("simulate failed");

    let mon_times: Vec<u64> = sim
        .output
        .iter()
        .filter_map(|o| {
            o.message
                .trim()
                .strip_prefix("MON ")
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|t| t.parse().ok())
        })
        .collect();

    let text = std::fs::read_to_string(&path).expect("VCD not written");
    let stamps: Vec<u64> = text
        .lines()
        .filter_map(|l| l.strip_prefix('#'))
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let _ = std::fs::remove_file(&path);

    assert!(
        mon_times.contains(&550_000),
        "monitor never reported the delay-target slot: {mon_times:?}"
    );
    for t in &mon_times {
        assert!(
            stamps.contains(t),
            "monitor reported t={t} but the dump has no record for it; \
             dump stamps were {stamps:?}"
        );
    }
}

/// The write that follows a `#delay` belongs to the slot the delay resumed in.
///
/// `run_events_until` closes out a slot when it is about to advance past it,
/// but its "this slot has unserviced work" flag started FALSE. A process
/// reaches that loop from a `#delay`, so by then it has already executed the
/// statements after its PREVIOUS delay — at the current time, and with no
/// postponed region yet. Starting false skipped exactly that slot.
///
/// Here `sig = 1` executes at t=100 (a `#50` resuming inside an edge block
/// that fired at t=50). Both `$monitor` and the dump reported it at 150 — the
/// next slot that happened to be serviced — while an internal trace of the
/// write said 100. Every view must say 100.
const WRITE_AFTER_DELAY: &str = r#"
`timescale 1ns/1ps
module top;
  logic tick = 0;
  logic sig  = 0;
  always #50 tick = ~tick;

  always @(posedge tick) begin
    #50  sig = 1;     // resumes and writes at t=100
    #500 sig = 0;     // and at t=600
  end

  initial $monitor("MON %0t sig=%b", $time, sig);
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
    #900 $finish;
  end
endmodule
"#;

#[test]
fn write_after_a_delay_is_reported_in_its_own_slot() {
    let mut path = std::env::temp_dir();
    path.push(format!("xezim_after_delay_{}.vcd", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let src = WRITE_AFTER_DELAY.replace("@VCD@", path.to_str().unwrap());
    let sim = simulate(&src, 100_000_000).expect("simulate failed");

    let mons: Vec<String> = sim
        .output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("MON "))
        .collect();
    let text = std::fs::read_to_string(&path).expect("VCD not written");
    let stamps: Vec<u64> = text
        .lines()
        .filter_map(|l| l.strip_prefix('#'))
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let _ = std::fs::remove_file(&path);

    assert!(
        mons.contains(&"MON 100000 sig=1".to_string()),
        "the t=100ns write must be monitored at 100000ps, got {mons:?}"
    );
    assert!(
        stamps.contains(&100_000),
        "the dump must hold a record at #100000; stamps were {stamps:?}"
    );
    assert!(
        stamps.contains(&600_000),
        "and at #600000 for the second write; stamps were {stamps:?}"
    );
}
