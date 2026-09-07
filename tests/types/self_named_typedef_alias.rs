//! A typedef that aliases a specialization of a class of the SAME name
//! (`typedef req_t #(.DW(DW)) req_t;`, the snitch mem_test pattern) made the
//! local-type string walk follow `req_t` to itself forever: the simulation
//! spun at time 0 at full CPU and ignored SIGTERM. Must finish promptly.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DESIGN: &str = r#"
class req_t #(int DW = 8);
  logic [DW-1:0] data;
  function new(); data = DW; endfunction
endclass
class driver #(int DW = 16);
  typedef req_t #(.DW(8)) req_t;
  task run();
    req_t r = new;
    string tag = "ok";
    $display("R data=%0d tag=%s", r.data, tag);
  endtask
endclass
module top;
  driver #(.DW(12)) d = new;
  initial begin d.run(); $finish; end
endmodule
"#;

#[test]
fn self_named_class_typedef_alias_does_not_spin() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("self_named_typedef");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    std::fs::write(&src, DESIGN).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "-s", "top", src.to_str().unwrap(), "--no-cache"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("simulation did not finish within 60 s (time-0 spin)");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let output = child.wait_with_output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "run failed:\n{text}");
    assert!(text.contains("R data=8 tag=ok"), "wrong output:\n{text}");
}
