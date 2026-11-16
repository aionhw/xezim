//! §23.6 / §7.4.1: absolute packed selects and awakened processes
//! keep their own declaration scope despite a same-named writer local.
#[test]
fn rooted_reads_and_observers_keep_top_scope() {
    let sim = xezim::simulate(
        r#"
module writer;
 logic done;
 initial begin
  #5 top.done[0][181]=0;
  #5 top.done[0][181]=1;
 end
endmodule
module observer;
 int checks;
 always @(top.done) if($time>0) begin
  if($isunknown(top.done[0][181])) $fatal(1,"hierarchical select became unknown");
  checks++;
 end
endmodule
module top;
 bit [1:0][255:0] done='1;
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
"#,
        1000,
    )
    .expect("simulate");
    assert!(
        sim.output.iter().any(|line| line.message == "T|PASS"),
        "{:?}",
        sim.output
    );
}
