use xezim::simulate;
fn check(src: &str) {
    let sim = simulate(src, 100).unwrap();
    let out = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        out.contains("PASS") && !out.contains("Fatal") && !out.contains("null object"),
        "{out}"
    );
}
const INLINE: &str = r#"class transaction;
 rand int resp[];
 constraint c {resp.size()==4;}
endclass
class mapper;
 task copy();
  transaction _target_txn=new();
  transaction target_txn=new();
  _target_txn.resp='{1,2,3,4};
  repeat(4) begin
   if(!target_txn.randomize() with {foreach(resp[i]) resp[i]==_target_txn.resp[i];}) $fatal(1,"randomize failed");
   foreach(target_txn.resp[i]) if(target_txn.resp[i]!=i+1) $fatal(1,"bad copy %0d",i);
  end
  $display("PASS");
 endtask
endclass
module top;
 initial begin mapper m=new(); m.copy(); $finish; end
endmodule
"#;
const FORK: &str = r#"class transaction;
 bit resp_has_data=1;
endclass
class monitor;
 int seen=0;
 function int get_data(transaction probe_txn);
  if(probe_txn==null) $fatal(1,"null probe");
  return probe_txn.resp_has_data;
 endfunction
 task associate(output transaction associated[$]);
  transaction temp[$];
  transaction t=new();
  temp.push_back(t);
  associated=temp;
  foreach(associated[i]) begin
   automatic int ix=i;
   fork begin
    if(associated[ix].resp_has_data) #1;
    seen+=get_data(associated[ix]);
   end join_none
  end
 endtask
endclass
module top;
 initial begin
  monitor m=new();
  transaction q[$];
  m.associate(q);
  #2;
  if(m.seen!=1) $fatal(1,"wrong seen %0d",m.seen);
  $display("PASS");$finish;
 end
endmodule
"#;
#[test]
fn inline_foreach_reads_caller_object_array() {
    check(INLINE);
}
#[test]
fn fork_keeps_output_queue_alive_after_task_returns() {
    check(FORK);
}
#[test]
fn output_queue_survives_until_last_fork_child_finishes() {
    check(
        &FORK
            .replace(
                "temp.push_back(t);",
                "temp.push_back(t); temp.push_back(t);",
            )
            .replace("#1;", "#(ix+1);")
            .replace("#2;", "#3;")
            .replace("m.seen!=1", "m.seen!=2"),
    );
}
#[test]
fn fork_keeps_local_queue_alive_after_task_returns() {
    check(
        &FORK
            .replace(
                "task associate(output transaction associated[$]);",
                "task associate(); transaction associated[$];",
            )
            .replace("m.associate(q)", "m.associate()"),
    );
}

#[test]
fn sibling_forks_share_task_prerequisite_flag() {
    check(
        r#"class driver;
 int done=0;
 task send();
  bit prereq_done=0;
  fork
   begin
    fork
     begin
      fork
       begin #1; prereq_done=1; end
       begin wait(prereq_done==1); done=1; end
      join
     end
     begin #10; end
    join_any
    disable fork;
   end
  join
 endtask
endclass
module top;
 initial begin driver d=new(); d.send(); if(d.done!=1) $fatal(1,"prereq lost");$display("PASS");$finish; end
endmodule
"#,
    );
}

#[test]
fn sibling_observation_does_not_overwrite_later_write() {
    check(
        r#"
class C;
 task run();
  int x=0;
  fork
   begin #1; x=1; end
   begin #2; x=2; end
   begin #1; #2; if(x!=2) $fatal(1,"stale capture %0d",x); end
  join
  if(x!=2) $fatal(1,"parent stale %0d",x);
 endtask
endclass
module top;
 initial begin C c=new(); c.run(); $display("PASS");$finish; end
endmodule
"#,
    );
}

#[test]
fn fork_captures_from_different_loop_blocks_stay_separate() {
    check(
        r#"
class C;
 int seen[2];
 task run();
  for(int i=0;i<2;i++) begin
   automatic int value=i;
   automatic int slot=i;
   fork begin
    #1;
    if(slot==0) value=99;
    #1;
    seen[slot]=value;
   end join_none
   #0;
  end
  #3;
  if(seen[0]!=99 || seen[1]!=1) $fatal(1,"block captures mixed %0d %0d",seen[0],seen[1]);
 endtask
endclass
module top;
 initial begin C c=new(); c.run(); $display("PASS"); $finish; end
endmodule
"#,
    );
}
