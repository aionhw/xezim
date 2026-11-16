//! §23.6 / §7.4.1: absolute packed selects and awakened processes
//! keep their own declaration scope despite a same-named writer local.
#[test]
fn rooted_reads_and_observers_keep_top_scope() {
    let source = r#"
module writer;
 logic done;
 logic [1:0][255:0] flat='1;
 initial begin
  if(top.flat[1] !== 1'b1) $fatal(1,"absolute scalar inherited child dimensions");
  #5 top.done[0][181]=0;
  #5 top.done[0][181]=1;
 end
endmodule
module observer(output int checks);
 logic done;
 always @(top.done) if($time>0) begin
  if(top.done[0][181] === 1'bx || $isunknown(top.done[0][181])) $fatal(1,"hierarchical select became unknown");
  checks++;
 end
endmodule
module top;
 bit [1:0][255:0] done='1;
 logic [511:0] flat=512'h2;
 int checks;
 writer w(); observer o();
 always @(done) if($time>0) begin
  if($isunknown(done[0][181]) || $isunknown(done)) $fatal(1,"writer scope leaked");
  checks++;
 end
 initial begin
  #11;
  if(checks!=2 || o.checks!=2 || done!='1) $fatal(1,"lost update");
  $display("T|PASS"); $finish;
 end
endmodule
"#;
    let sim = xezim::simulate(source, 1000).expect("simulate");
    assert!(
        sim.output.iter().any(|line| line.message == "T|PASS"),
        "{:?}",
        sim.output
    );
    let dir = std::env::temp_dir().join(format!("xezim_rooted_observer_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(&path, source).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--no-cache")
        .arg(&path)
        .output()
        .expect("run CLI");
    let _ = std::fs::remove_dir_all(dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success() && text.contains("T|PASS"), "{text}");
}
