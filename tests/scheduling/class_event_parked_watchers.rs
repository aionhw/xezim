//! §15.5 / §9.4.2: processes parked on class-member events are indexed by
//! event and by process. A trigger wakes only the waiters of that event, in
//! the order they parked; a waiter on `@(a or b)` wakes once, by whichever
//! fires first; a disabled waiter never wakes. Watchers that a
//! `fork ... join_any` leaves parked stay parked until their event fires.
//! Expected T| lines are the reference simulator's.
use std::process::Command;

const DESIGN: &str = r#"
class C;
  event rst, beat, other;
endclass

module top;
  C c;
  initial begin
    c = new;
    // Watchers left parked by join_any: they wait on `rst` forever.
    for (int i = 0; i < 4; i++) begin
      fork
        automatic int k = i;
        begin @(c.rst); $display("T|rst %0d @%0t", k, $time); end
        begin @(c.beat); $display("T|beat %0d @%0t", k, $time); end
      join_any
    end
    // One waiter on either event: woken once, by the first trigger.
    fork begin @(c.other or c.rst); $display("T|any @%0t", $time); end join_none
    // A waiter whose process is disabled before the trigger never wakes.
    fork begin : doomed @(c.rst); $display("T|doomed woke"); end join_none
    #1 disable doomed;
    #1 -> c.other;
    #1 -> c.rst;
    #1 -> c.other;
    #1 $display("T|done");
  end
  initial begin
    for (int i = 0; i < 4; i++) begin #0; -> c.beat; end
  end
endmodule
"#;

const EXPECTED: &[&str] = &[
    "T|beat 0 @0",
    "T|beat 1 @0",
    "T|beat 2 @0",
    "T|beat 3 @0",
    "T|any @2",
    "T|rst 0 @3",
    "T|rst 1 @3",
    "T|rst 2 @3",
    "T|rst 3 @3",
    "T|done",
];

fn run(compile: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_class_event_watchers_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(&path, DESIGN).expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_COMPILE_METHODS", compile)
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    text
}

#[test]
fn parked_class_event_watchers_wake_by_event_in_order() {
    for compile in ["0", "1"] {
        let log = run(compile);
        let lines: Vec<&str> = log.lines().filter(|l| l.starts_with("T|")).collect();
        assert_eq!(lines, EXPECTED, "XEZIM_COMPILE_METHODS={compile}:\n{log}");
    }
}
