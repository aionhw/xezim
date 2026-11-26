//! Ctrl-C must finalize the waveform, not abandon it.
//!
//! xezim installed no signal handler, so an interrupted run died where it
//! stood. For VCD and XTrace that costs the tail; for FST it costs
//! EVERYTHING — value changes accumulate in an in-memory block that only
//! `fst_finish` writes, so an interrupted 17MB run left a 577-byte file
//! holding a header, hierarchy and geometry and no data at all.
//!
//! SIGINT/SIGTERM now set a flag that the event loop polls, so the run leaves
//! through its normal exit and reaches `vcd_finish`/`xtrace_finish`/
//! `fst_finish`. A second signal restores the default disposition and
//! re-raises, so an interrupt is never swallowed. The run then exits 128 +
//! the signal number (130 here), not 0.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Long enough that the interrupt lands mid-run on any machine.
const SRC: &str = r#"
`timescale 1ns/1ps
module top;
  logic clk = 0;
  logic [31:0] c = 0;
  always #5 clk = ~clk;
  always @(posedge clk) c <= c + 1;
  initial begin
    $dumpfile("@VCD@");
    $dumpvars(0, top);
  end
  initial #2000000000 $finish;   // far longer than the test will run
endmodule
"#;

#[test]
fn sigint_finalizes_fst_and_vcd() {
    let pid = std::process::id();
    let dir = std::env::temp_dir();
    let sv = dir.join(format!("xezim_int_{pid}.sv"));
    let vcd = dir.join(format!("xezim_int_{pid}.vcd"));
    let fst = dir.join(format!("xezim_int_{pid}.fst"));
    for p in [&sv, &vcd, &fst] {
        let _ = std::fs::remove_file(p);
    }
    let mut f = std::fs::File::create(&sv).unwrap();
    f.write_all(SRC.replace("@VCD@", vcd.to_str().unwrap()).as_bytes())
        .unwrap();
    drop(f);

    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "top", "--fst"])
        .arg(&fst)
        .arg(&sv)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn xezim");

    // Let it accumulate some waveform, then interrupt.
    std::thread::sleep(Duration::from_millis(1500));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
    let status = child.wait().expect("wait");

    let fst_len = std::fs::metadata(&fst).map(|m| m.len()).unwrap_or(0);
    let vcd_text = std::fs::read_to_string(&vcd).unwrap_or_default();
    for p in [&sv, &vcd, &fst] {
        let _ = std::fs::remove_file(p);
    }

    // Through the normal exit, which reports the interrupt: 128 + SIGINT.
    assert_eq!(
        status.code(),
        Some(130),
        "an interrupted run should leave through its normal exit with status 130"
    );

    // The FST must carry a value-change block, not just a header. A dump that
    // lost its data is a few hundred bytes; one with data is far larger.
    assert!(
        fst_len > 2_000,
        "FST looks like header-only after SIGINT ({fst_len} bytes) — \
         the value-change block was never written"
    );

    // And the VCD must hold real time records, not just its t=0 snapshot.
    let stamps = vcd_text.lines().filter(|l| l.starts_with('#')).count();
    assert!(
        stamps > 10,
        "VCD holds only {stamps} time records after SIGINT"
    );
}
