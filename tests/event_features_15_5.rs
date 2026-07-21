//! IEEE 1800-2017 §6.17/§15.5 event-feature fixes from issue #40:
//! `->>` defers to the NBA region, `wait_order` parses and enforces order,
//! event variables are handles (aliasing/merging/null per §15.5.5), and
//! events work as array/queue elements.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn run(src: &str) -> Vec<String> {
    messages(&simulate(src, 10000).expect("sim"))
}

#[test]
fn nonblocking_trigger_defers_to_nba_region() {
    // §15.5.2: `->> e` schedules the trigger in the NBA region — a same-slot
    // `.triggered` read from the active (or #0-inactive) region sees 0.
    let out = run(r#"
module top;
  event e;
  bit early, late;
  initial begin
    fork
      begin ->> e; if (e.triggered) early = 1; end
      begin #0; if (e.triggered) late = 1; end
    join
    #1 $display("E=%b L=%b", early, late);
    $finish;
  end
endmodule
"#);
    assert!(out.iter().any(|m| m == "E=0 L=0"), "output: {:?}", out);
}

#[test]
fn wait_order_pass_and_fail_branches() {
    // §15.5.3: in-order completion runs the pass action; the first
    // out-of-order fire runs the else action.
    let out = run(r#"
module top;
  event a, b, c;
  initial begin
    fork
      begin #10 -> a; #10 -> b; #10 -> c; end
      wait_order(a, b, c) $display("ORD_PASS"); else $display("ORD_FAIL");
    join
    fork
      begin #10 -> a; #10 -> c; #10 -> b; end
      wait_order(a, b, c) $display("ORD2_PASS"); else $display("ORD2_FAIL");
    join
    $finish;
  end
endmodule
"#);
    assert!(out.iter().any(|m| m == "ORD_PASS"), "output: {:?}", out);
    assert!(out.iter().any(|m| m == "ORD2_FAIL"), "output: {:?}", out);
    assert!(!out.iter().any(|m| m == "ORD2_PASS"), "output: {:?}", out);
}

#[test]
fn event_handles_alias_merge_unmerge_null() {
    // §15.5.5: assignment shares the synchronization object; re-assignment
    // to a fresh event un-merges; triggering a null handle is a no-op.
    let out = run(r#"
module top;
  event base;
  initial begin
    begin
      event alias1, alias2, isolated;
      bit chk1, chk2;
      alias1 = base;
      alias2 = alias1;
      fork
        begin #10 -> base; end
        begin #10 if (alias2.triggered) chk1 = 1; end
      join
      #1;
      alias2 = isolated;
      fork
        begin #10 -> base; end
        begin #10 if (alias2.triggered) chk2 = 1; end
      join
      #1 $display("MERGE=%b UNMERGE=%b", chk1, chk2);
      alias1 = null;
      -> alias1; // no-op, must not disturb base
      $display("NULLTRIG_OK");
    end
    $finish;
  end
endmodule
"#);
    assert!(
        out.iter().any(|m| m == "MERGE=1 UNMERGE=0"),
        "output: {:?}",
        out
    );
    assert!(out.iter().any(|m| m == "NULLTRIG_OK"), "output: {:?}", out);
}

#[test]
fn event_arrays_and_queues() {
    // §6.17: events are first-class — array elements trigger and report
    // `.triggered` individually; queue elements keep their handle identity.
    let out = run(r#"
module top;
  initial begin
    begin
      event arr [3];
      event q [$:5];
      bit achk = 0, qchk = 0;
      begin
        event e0, e1, e2;
        arr[0] = e0; arr[1] = e1; arr[2] = e2;
      end
      begin
        event eq0, eq1;
        q.push_back(eq0);
        q.push_back(eq1);
      end
      fork
        begin #10 -> arr[1]; end
        begin #10 if (arr[1].triggered) achk = 1; end
      join
      #1;
      fork
        begin
          event tmp;
          #10;
          tmp = q[0];
          -> tmp;
        end
        begin #10 if (q[0].triggered) qchk = 1; end
      join
      #1 $display("ARR=%b Q=%b", achk, qchk);
    end
    $finish;
  end
endmodule
"#);
    assert!(out.iter().any(|m| m == "ARR=1 Q=1"), "output: {:?}", out);
}
