//! A single SIGTERM (what `timeout(1)` sends) must end a run even while it is
//! stuck outside the event loop, which is the only place the interrupt flag
//! used to be polled. A `randomize()` retried in a loop against an
//! unsatisfiable constraint spun in the solver, and a long synchronous loop
//! in a function never returned to the scheduler: both ignored SIGTERM, so
//! only SIGKILL stopped them. The solver now polls the flag, and an
//! interrupt the run does not take within a few seconds ends the process by
//! the signal's default action. Process handling only: no simulation output
//! is compared.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

/// Start `src`, wait until it has printed `start` (flushed by `$fflush`),
/// send one SIGTERM and return how long the process took to exit afterwards.
fn term_after_start(src: &str, tag: &str) -> Duration {
    let base = format!("/tmp/sigterm_{tag}_{}", std::process::id());
    let path = format!("{base}.sv");
    let out_path = format!("{base}.out");
    std::fs::write(&path, src).unwrap();
    let out = std::fs::File::create(&out_path).unwrap();
    let mut child = Command::new(xezim())
        .arg(&path)
        .stdout(Stdio::from(out))
        .stderr(Stdio::null())
        .spawn()
        .expect("run xezim");
    let t_start = Instant::now();
    while !std::fs::read_to_string(&out_path)
        .unwrap_or_default()
        .contains("start")
        && t_start.elapsed() < Duration::from_secs(60)
        && child.try_wait().expect("wait").is_none()
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let t0 = Instant::now();
    loop {
        if child.try_wait().expect("wait").is_some() {
            break;
        }
        if t0.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out_path);
    t0.elapsed()
}

#[test]
fn sigterm_stops_a_spinning_randomize() {
    let d = term_after_start(
        r#"
class c;
  rand bit [15:0] x;
  rand bit [15:0] y;
  constraint k { x * y == 32'd7; x > 1; y > 1; }
endclass
module top;
  c o;
  int n;
  initial begin
    o = new;
    $display("start");
    $fflush;
    while (!o.randomize()) n++;
    $display("done %0d", n);
  end
endmodule
"#,
        "rand",
    );
    assert!(d < Duration::from_secs(3), "took {d:?} to stop");
}

#[test]
fn sigterm_ends_a_run_stuck_in_a_function() {
    let d = term_after_start(
        r#"
module top;
  int x;
  function void spin();
    for (int i = 0; i < 100000; i++)
      for (int j = 0; j < 100000; j++)
        x = x + 1;
  endfunction
  initial begin
    $display("start");
    $fflush;
    spin();
    $display("done %0d", x);
  end
endmodule
"#,
        "loop",
    );
    assert!(d < Duration::from_secs(15), "took {d:?} to stop");
}
