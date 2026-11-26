//! IEEE 1800-2023 §20.11/§20.12 assertion control system tasks:
//! `$asserton`/`$assertoff`/`$assertkill`, the action-control tasks and
//! `$assertcontrol` (Lock/Unlock, assertion_type and directive_type masks,
//! levels and list scoping). Expected values come from the reference
//! simulator, except where a test says otherwise.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100_000).expect("simulate failed");
    sim.output
        .iter()
        .filter_map(|o| o.message.find("T|").map(|i| o.message[i..].to_string()))
        .collect()
}

/// On/Off by assertion_type and directive_type over simple immediate, `#0` and
/// `final` deferred assertions and immediate covers; Off leaves queued
/// deferred reports to mature; PassOff/FailOff (and the default `$error`)
/// for immediate actions and, at maturity, for queued deferred reports;
/// VacuousOff/NonvacuousOn on immediates, which never pass vacuously.
#[test]
fn immediate_and_deferred_controls() {
    let src = r#"
module p1b;
  int fi, fo, ff, pi, ci, fa;
  function void ifo(); fo++; endfunction
  function void iff_(); ff++; endfunction
  function void ipi(); pi++; endfunction
  initial begin
    $assertoff;
    assert (0) else fi++;
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    #1 $display("T|1|off fi=%0d fo=%0d ff=%0d", fi, fo, ff);
    $asserton;
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    $assertoff;
    #1 $display("T|2|queued-then-off fo=%0d ff=%0d", fo, ff);
    $asserton;
    $assertcontrol(4, 2);
    assert (0) else fi++;
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    #1 $display("T|4|off-simple fi=%0d fo=%0d ff=%0d", fi, fo, ff);
    $assertcontrol(3);
    $assertcontrol(4, 4);
    assert (0) else fi++;
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    #1 $display("T|5|off-observed fi=%0d fo=%0d ff=%0d", fi, fo, ff);
    $assertcontrol(3);
    $assertcontrol(4, 8);
    assert (0) else fi++;
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    #1 $display("T|6|off-final fi=%0d fo=%0d ff=%0d", fi, fo, ff);
    $assertcontrol(3);
    $assertcontrol(4, 1);
    assert (0) else fi++;
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    #1 $display("T|7|off-concurrent fi=%0d fo=%0d ff=%0d", fi, fo, ff);
    $assertcontrol(3);
    // directive masks
    $assertcontrol(4, 15, 2);
    assert (0) else fi++;
    cover (1) ci++;
    assume (0) else fa++;
    #1 $display("T|8|off-cover fi=%0d ci=%0d fa=%0d", fi, ci, fa);
    $assertcontrol(3);
    $assertcontrol(4, 15, 4);
    assert (0) else fi++;
    cover (1) ci++;
    assume (0) else fa++;
    #1 $display("T|9|off-assume fi=%0d ci=%0d fa=%0d", fi, ci, fa);
    $assertcontrol(3);
    $assertcontrol(4, 15, 1);
    assert (0) else fi++;
    cover (1) ci++;
    assume (0) else fa++;
    #1 $display("T|10|off-assert fi=%0d ci=%0d fa=%0d", fi, ci, fa);
    $assertcontrol(3);
    $assertoff;
    cover (1) ci++;
    $assertkill;
    cover (1) ci++;
    $asserton;
    cover (1) ci++;
    #1 $display("T|11|cover off/kill/on ci=%0d", ci);
    // action control immediate
    $assertpassoff;
    assert (1) pi++; else fi++;
    assert (0) pi++; else fi++;
    cover (1) ci++;
    #1 $display("T|12|passoff pi=%0d fi=%0d ci=%0d", pi, fi, ci);
    $assertpasson;
    $assertfailoff;
    assert (1) pi++; else fi++;
    assert (0) pi++; else fi++;
    assert (0);
    $display("T|13|failoff pi=%0d fi=%0d", pi, fi);
    $assertfailon;
    assert (0);
    $display("T|14|failon default err above");
    // deferred action control
    $assertpassoff;
    assert #0 (1) ipi(); else ifo();
    #1 $display("T|15|passoff deferred pi=%0d", pi);
    $assertpasson;
    assert #0 (1) ipi(); else ifo();
    $assertpassoff;
    #1 $display("T|16|queued-then-passoff deferred pi=%0d", pi);
    $assertpasson;
    // vacuous controls on immediate
    $assertvacuousoff;
    assert (1) pi++;
    #1 $display("T|17|vacuousoff imm pi=%0d", pi);
    $assertpassoff;
    $assertnonvacuouson;
    assert (1) pi++;
    #1 $display("T|18|passoff+nonvacuouson imm pi=%0d", pi);
    $assertpasson;
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|1|off fi=0 fo=0 ff=0",
            "T|2|queued-then-off fo=1 ff=1",
            "T|4|off-simple fi=0 fo=2 ff=2",
            "T|5|off-observed fi=1 fo=2 ff=3",
            "T|6|off-final fi=2 fo=3 ff=3",
            "T|7|off-concurrent fi=3 fo=4 ff=4",
            "T|8|off-cover fi=4 ci=0 fa=1",
            "T|9|off-assume fi=5 ci=1 fa=1",
            "T|10|off-assert fi=5 ci=2 fa=2",
            "T|11|cover off/kill/on ci=3",
            "T|12|passoff pi=0 fi=6 ci=3",
            "T|13|failoff pi=1 fi=6",
            "T|14|failon default err above",
            "T|15|passoff deferred pi=1",
            "T|16|queued-then-passoff deferred pi=1",
            "T|17|vacuousoff imm pi=2",
            "T|18|passoff+nonvacuouson imm pi=3",
        ]
    );
}

/// Off lets an attempt in flight finish, Kill aborts it; PassOff/FailOff
/// decide when the attempt finishes; a vacuous success runs the pass action
/// by default, and VacuousOff/NonvacuousOn split the two; `cover
/// property` follows On/Off and the directive and assertion type masks.
#[test]
fn concurrent_controls() {
    let src = r#"
module p2;
  bit clk, a, b, v; int p, f, pv, fv, k1, c1;
  always #5 clk = ~clk;
  // multi-cycle attempt: a |-> ##2 b
  ap: assert property (@(posedge clk) a |-> ##2 b) p++; else f++;
  // vacuous-only: v is 0 -> vacuous pass every clock
  vp: assert property (@(posedge clk) v |-> 1) pv++; else fv++;
  cp: cover property (@(posedge clk) a) c1++;
  initial begin
    a = 1; b = 0;
    @(negedge clk); // t=10: one attempt started at 5
    a = 0;
    $assertoff(0, ap);
    repeat (3) @(negedge clk); // t=40: attempt from 5 completes at 25 (fail) if not aborted
    $display("T|1|off-inflight p=%0d f=%0d", p, f);
    $asserton(0, ap);
    a = 1; @(negedge clk); a = 0; // attempt started at 45
    $assertkill(0, ap);
    repeat (3) @(negedge clk);
    $display("T|2|kill-inflight p=%0d f=%0d", p, f);
    $asserton(0, ap);
    // pass control in flight: start attempt, passoff, attempt completes with pass
    a = 1; b = 1; @(negedge clk); a = 0;
    $assertpassoff(0, ap);
    repeat (3) @(negedge clk);
    $display("T|3|passoff-inflight p=%0d f=%0d", p, f);
    $assertpasson(0, ap);
    b = 0; a = 1; @(negedge clk); a = 0;
    $assertfailoff(0, ap);
    repeat (3) @(negedge clk);
    $display("T|4|failoff-inflight p=%0d f=%0d", p, f);
    $assertfailon(0, ap);
    $display("T|5|vac base pv=%0d", pv);
    $assertvacuousoff(0, vp);
    repeat (2) @(negedge clk);
    $display("T|6|vacuousoff pv=%0d", pv);
    $assertpasson(0, vp);
    repeat (2) @(negedge clk);
    $display("T|7|passon after vacoff pv=%0d", pv);
    $assertpassoff(0, vp);
    $assertnonvacuouson(0, vp);
    repeat (2) @(negedge clk);
    $display("T|8|passoff+nonvacon pv=%0d", pv);
    $assertpasson(0, vp);
    // cover
    a = 1; repeat (2) @(negedge clk);
    $display("T|9|cover base c1=%0d", c1);
    $assertoff(0, cp); repeat (2) @(negedge clk);
    $display("T|10|cover off c1=%0d", c1);
    $asserton(0, cp); $assertcontrol(4, 15, 1); repeat (2) @(negedge clk);
    $display("T|11|assert-only off c1=%0d", c1);
    $assertcontrol(3); $assertcontrol(7, 15, 2); repeat (2) @(negedge clk);
    $display("T|12|cover passoff c1=%0d", c1);
    $assertcontrol(6); $assertcontrol(4, 2); repeat (2) @(negedge clk);
    $display("T|13|immediate-only off c1=%0d", c1);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|1|off-inflight p=0 f=1",
            "T|2|kill-inflight p=0 f=1",
            "T|3|passoff-inflight p=0 f=1",
            "T|4|failoff-inflight p=3 f=1",
            "T|5|vac base pv=16",
            "T|6|vacuousoff pv=16",
            "T|7|passon after vacoff pv=18",
            "T|8|passoff+nonvacon pv=18",
            "T|9|cover base c1=6",
            "T|10|cover off c1=6",
            "T|11|assert-only off c1=8",
            "T|12|cover passoff c1=8",
            "T|13|immediate-only off c1=10",
        ]
    );
}

/// `levels` counts module instances below each listed scope (generate and
/// named blocks belong to their instance's level; 0 = all levels), with no
/// list it counts from the top; relative and absolute names, several list
/// items and a generate scope. The reference rejects the empty arguments of
/// the last call (`$assertcontrol(4, , , 1, u1)`); xezim gives them their
/// §20.12 defaults.
#[test]
fn levels_and_list_scoping() {
    let src = r#"
module leaf(input bit go);
  int fails;
  always @(go) begin assert (0) else fails++; end
endmodule
module mid(input bit go);
  int fails;
  always @(go) begin assert (0) else fails++; end
  leaf u2(go);
  if (1) begin : g
    int gf;
    always @(go) begin assert (0) else gf++; end
  end
endmodule
module p3;
  bit go; int fails;
  always @(go) begin assert (0) else fails++; end
  mid u1(go);
  leaf u3(go);
  task automatic pulse(string tag);
    int a0, a1, a2, a3, a4;
    a0 = fails; a1 = u1.fails; a2 = u1.u2.fails; a3 = u3.fails; a4 = u1.g.gf;
    #1 go = ~go; #1;
    $display("T|%s|top=%0d u1=%0d u1.u2=%0d u3=%0d u1.g=%0d", tag, fails-a0, u1.fails-a1, u1.u2.fails-a2, u3.fails-a3, u1.g.gf-a4);
    $asserton;
  endtask
  initial begin
    pulse("base");
    $assertoff(1, p3.u1); pulse("l1_u1");
    $assertoff(2, p3.u1); pulse("l2_u1");
    $assertoff(0, p3.u1); pulse("l0_u1");
    $assertoff(0, u1); pulse("rel_u1");
    $assertoff(1, p3); pulse("l1_top");
    $assertoff(2, p3); pulse("l2_top");
    $assertoff(1); pulse("l1_nolist");
    $assertoff(2); pulse("l2_nolist");
    $assertoff(0); pulse("l0_nolist");
    $assertoff(0, u1.u2, u3); pulse("two_items");
    $assertoff(0, p3.u1.g); pulse("gen_scope");
    $assertcontrol(4, 15, 7, 1, u1); pulse("ctl_l1_u1");
    $assertcontrol(4, , , 1, u1); pulse("ctl_empty_args");
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|base|top=1 u1=1 u1.u2=1 u3=1 u1.g=1",
            "T|l1_u1|top=1 u1=0 u1.u2=1 u3=1 u1.g=0",
            "T|l2_u1|top=1 u1=0 u1.u2=0 u3=1 u1.g=0",
            "T|l0_u1|top=1 u1=0 u1.u2=0 u3=1 u1.g=0",
            "T|rel_u1|top=1 u1=0 u1.u2=0 u3=1 u1.g=0",
            "T|l1_top|top=0 u1=1 u1.u2=1 u3=1 u1.g=1",
            "T|l2_top|top=0 u1=0 u1.u2=1 u3=0 u1.g=0",
            "T|l1_nolist|top=0 u1=1 u1.u2=1 u3=1 u1.g=1",
            "T|l2_nolist|top=0 u1=0 u1.u2=1 u3=0 u1.g=0",
            "T|l0_nolist|top=0 u1=0 u1.u2=0 u3=0 u1.g=0",
            "T|two_items|top=1 u1=1 u1.u2=0 u3=0 u1.g=1",
            "T|gen_scope|top=1 u1=1 u1.u2=1 u3=1 u1.g=0",
            "T|ctl_l1_u1|top=1 u1=0 u1.u2=1 u3=1 u1.g=0",
            "T|ctl_empty_args|top=1 u1=0 u1.u2=1 u3=1 u1.g=0",
        ]
    );
}

/// A locked assertion ignores every control but Unlock; Lock by scope and by
/// type masks; locking twice needs one Unlock; an out-of-range control_type
/// is ignored.
#[test]
fn lock_and_unlock() {
    let src = r#"
module leaf(input bit go);
  int fails;
  always @(go) begin assert (0) else fails++; end
endmodule
module p4;
  bit go; int fails;
  always @(go) begin assert (0) else fails++; end
  leaf u1(go);
  leaf u3(go);
  task automatic pulse(string tag);
    int a0, a1, a3;
    a0 = fails; a1 = u1.fails; a3 = u3.fails;
    #1 go = ~go; #1;
    $display("T|%s|top=%0d u1=%0d u3=%0d", tag, fails-a0, u1.fails-a1, u3.fails-a3);
  endtask
  initial begin
    $assertcontrol(1, 15, 7, 0, u1); // lock u1
    $assertoff; pulse("lock_u1_then_off");
    $asserton; pulse("on");
    $assertcontrol(2, 15, 7, 0, u1); // unlock u1
    $assertoff; pulse("unlock_then_off");
    $assertcontrol(1); // lock all (off state)
    $asserton; pulse("locked_on_ignored");
    $assertcontrol(2); $asserton; pulse("unlock_on");
    $assertcontrol(1, 1); // lock only concurrent
    $assertoff; pulse("lock_concurrent_off");
    $asserton; $assertcontrol(2);
    $assertcontrol(1, 15, 2); // lock covers only
    $assertoff; pulse("lock_cover_off");
    $asserton; $assertcontrol(2);
    $assertcontrol(1, 15, 7, 0, u1); $assertcontrol(1, 15, 7, 0, u1); $assertcontrol(2, 15, 7, 0, u1);
    $assertoff; pulse("double_lock_single_unlock");
    $asserton;
    $assertcontrol(12); pulse("bad_ctl");
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|lock_u1_then_off|top=0 u1=1 u3=0",
            "T|on|top=1 u1=1 u3=1",
            "T|unlock_then_off|top=0 u1=0 u3=0",
            "T|locked_on_ignored|top=0 u1=0 u3=0",
            "T|unlock_on|top=1 u1=1 u3=1",
            "T|lock_concurrent_off|top=0 u1=0 u3=0",
            "T|lock_cover_off|top=0 u1=0 u3=0",
            "T|double_lock_single_unlock|top=0 u1=0 u3=0",
            "T|bad_ctl|top=1 u1=1 u3=1",
        ]
    );
}

/// An immediate assertion named by its label (absolute, relative, inside a
/// named block), a named block as scope, and Kill aborting the multi-cycle
/// attempts in flight in one instance or of one named concurrent assertion.
#[test]
fn labels_named_blocks_and_kill_in_flight() {
    let src = r#"
module sub(input bit clk, input bit a);
  int f;
  sp: assert property (@(posedge clk) a |-> ##3 0) else f++;
endmodule
module p5;
  bit clk, a, go; int f1, f2, fb, fn;
  always #5 clk = ~clk;
  sub u1(clk, a);
  sub u2(clk, a);
  always @(go) begin
    ia: assert (0) else f1++;
    ib: assert (0) else f2++;
  end
  always @(go) begin : nb
    ic: assert (0) else fn++;
  end
  initial begin
    // name an immediate assertion by label
    $assertoff(0, p5.ia); #1 go = ~go; #1;
    $display("T|1|label-off f1=%0d f2=%0d", f1, f2);
    $asserton;
    $assertoff(0, ia); #1 go = ~go; #1;
    $display("T|2|rel-label-off f1=%0d f2=%0d", f1, f2);
    $asserton;
    $assertoff(0, p5.nb.ic); #1 go = ~go; #1;
    $display("T|3|named-block-label-off fn=%0d", fn);
    $assertoff(0, p5.nb); #1 go = ~go; #1;
    $display("T|4|named-block-scope-off fn=%0d", fn);
    $asserton;
    $assertoff(1, p5); #1 go = ~go; #1;
    $display("T|5|l1-top-named-block fn=%0d f1=%0d", fn, f1);
    $asserton;
    // kill in flight in one instance
    @(negedge clk); a = 1; @(negedge clk); a = 0;
    $assertkill(0, u1);
    $asserton;
    repeat (5) @(negedge clk);
    $display("T|6|kill-u1 u1.f=%0d u2.f=%0d", u1.f, u2.f);
    // kill named concurrent assertion in flight
    a = 1; @(negedge clk); a = 0;
    $assertkill(0, p5.u2.sp);
    repeat (5) @(negedge clk);
    $display("T|7|kill-u2.sp(no on) u1.f=%0d u2.f=%0d", u1.f, u2.f);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|1|label-off f1=0 f2=1",
            "T|2|rel-label-off f1=0 f2=2",
            "T|3|named-block-label-off fn=2",
            "T|4|named-block-scope-off fn=2",
            "T|5|l1-top-named-block fn=2 f1=2",
            "T|6|kill-u1 u1.f=0 u2.f=1",
            "T|7|kill-u2.sp(no on) u1.f=1 u2.f=1",
        ]
    );
}

/// A procedural concurrent assertion: Off starts no attempt from the
/// instances its `always` queues; the attempt in flight completes.
#[test]
fn procedural_concurrent_assertion() {
    let src = r#"
module p6;
  bit clk, a; int p, f, n;
  always #5 clk = ~clk;
  // procedural concurrent assertion in an always
  always @(posedge clk) begin
    n++;
    pa: assert property (a |-> ##1 a) p++; else f++;
  end
  initial begin
    a = 1; repeat (3) @(negedge clk);
    $display("T|1|base p=%0d f=%0d", p, f);
    $assertoff; repeat (3) @(negedge clk);
    $display("T|2|off p=%0d f=%0d", p, f);
    $asserton; repeat (2) @(negedge clk);
    a = 0; @(negedge clk);
    $display("T|3|on p=%0d f=%0d", p, f);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        ["T|1|base p=2 f=0", "T|2|off p=3 f=0", "T|3|on p=5 f=1",]
    );
}

/// An Off issued at the clock edge stops that edge's attempt (assertions are
/// evaluated after the Active region); `expect` ignores the controls, as in
/// the reference.
#[test]
fn off_in_observed_slot_and_expect() {
    let src = r#"
module p7;
  bit clk, a; int f, e;
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) a) else f++;
  initial begin
    // $assertoff at the same posedge as attempt start (race), from an always @(posedge)
    repeat (2) @(negedge clk);
    $display("T|1|base f=%0d", f);
    @(posedge clk) $assertoff;
    @(negedge clk);
    $display("T|2|off-at-posedge f=%0d", f);
    $asserton;
    // expect statement control
    fork
      begin expect (@(posedge clk) a) else e++; end
    join_none
    $assertcontrol(4, 16);
    @(negedge clk); @(negedge clk);
    $display("T|3|expect-off e=%0d", e);
    $assertcontrol(3, 255);
    $assertcontrol(4, 255);
    begin expect (@(posedge clk) a) else e++; end
    $display("T|4|expect-off-255 e=%0d", e);
    $assertcontrol(3, 255);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|1|base f=2",
            "T|2|off-at-posedge f=2",
            "T|3|expect-off e=1",
            "T|4|expect-off-255 e=2",
        ]
    );
}

/// Sequences whose later calls override earlier ones (scoped by global,
/// deeper levels by shallower, type masks that do not cover each other, a
/// toggle loop, Unlock without Lock, Lock then a global Unlock).
#[test]
fn log_compaction_cases() {
    let src = r#"
module leaf(input bit go);
  int fails;
  always @(go) begin assert (0) else fails++; end
endmodule
module mid(input bit go);
  int fails;
  always @(go) begin assert (0) else fails++; end
  leaf u2(go);
endmodule
module p11;
  bit go; int fails, n;
  always @(go) begin assert (0) else fails++; end
  mid u1(go);
  task automatic pulse(string tag);
    int a0, a1, a2;
    a0 = fails; a1 = u1.fails; a2 = u1.u2.fails;
    #1 go = ~go; #1;
    $display("T|%s|top=%0d u1=%0d u1.u2=%0d", tag, fails-a0, u1.fails-a1, u1.u2.fails-a2);
  endtask
  initial begin
    $assertoff(0, u1); $asserton; pulse("scoped_off_global_on");
    $assertoff(1); $asserton(2); pulse("off1_on2");
    $assertoff(2); $asserton(1); pulse("off2_on1");
    $asserton;
    $assertcontrol(4, 2); $assertcontrol(3, 1); pulse("imm_off_conc_on");
    $asserton;
    for (n = 0; n < 1000; n++) begin $assertoff(0, u1.u2); $asserton(0, u1.u2); $assertoff(1, u1); end
    pulse("loop");
    $asserton;
    $assertoff(0, u1); $assertcontrol(2); $asserton(1, u1); pulse("unlock_noop_on1");
    $asserton;
    $assertcontrol(1, 15, 7, 0, u1.u2); $assertoff; $assertcontrol(2); $asserton(1); pulse("lock_then_unlock_all");
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|scoped_off_global_on|top=1 u1=1 u1.u2=1",
            "T|off1_on2|top=1 u1=1 u1.u2=1",
            "T|off2_on1|top=1 u1=0 u1.u2=1",
            "T|imm_off_conc_on|top=0 u1=0 u1.u2=0",
            "T|loop|top=1 u1=0 u1.u2=1",
            "T|unlock_noop_on1|top=1 u1=1 u1.u2=0",
            "T|lock_then_unlock_all|top=1 u1=0 u1.u2=1",
        ]
    );
}

/// §20.12 Kill flushes the queued reports of deferred assertions it selects
/// (Off does not, see above); a Kill of another scope or of the other
/// deferred kind leaves them. The reference matures them anyway; xezim
/// follows the clause.
#[test]
fn kill_flushes_queued_deferred_reports() {
    let src = r#"
module sub; endmodule
module p8;
  int fo, ff;
  sub u1();
  function void ifo(); fo++; endfunction
  function void iff_(); ff++; endfunction
  initial begin
    assert #0 (0) else ifo();
    assert final (0) else iff_();
    $assertkill;
    #1 $display("T|1|queued-then-kill fo=%0d ff=%0d", fo, ff);
    $asserton;
    assert #0 (0) else ifo();
    $assertkill(0, p8.u1);
    #1 $display("T|2|kill-other-scope fo=%0d", fo);
    $asserton;
    assert #0 (0) else ifo();
    $assertcontrol(5, 8);
    #1 $display("T|3|kill-final-only fo=%0d", fo);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|1|queued-then-kill fo=0 ff=0",
            "T|2|kill-other-scope fo=1",
            "T|3|kill-final-only fo=2",
        ]
    );
}

/// PassOn enables pass actions only: a failing concurrent assertion still
/// runs its fail action (or none under FailOff). The reference runs the PASS
/// action of every failure of an assertion after `$assertpasson`, which no
/// clause supports; xezim keeps the fail action.
#[test]
fn pass_on_leaves_failures_failing() {
    let src = r#"
module p10;
  bit clk, a; int fails, passes, f2, p2;
  always #5 clk = ~clk;
  ap: assert property (@(posedge clk) a) passes++; else fails++;
  bp: assert property (@(posedge clk) a) p2++; else f2++;
  initial begin
    a = 0;
    repeat (2) @(negedge clk); $display("T|0|passes=%0d fails=%0d", passes, fails);
    $assertpasson(0, ap); repeat (2) @(negedge clk); $display("T|1|passon(nothing before) passes=%0d fails=%0d", passes, fails);
    $assertpassoff(0, ap); $assertpasson(0, ap); repeat (2) @(negedge clk); $display("T|2|passoff+passon passes=%0d fails=%0d", passes, fails);
    $assertpassoff(0, bp); repeat (2) @(negedge clk); $display("T|3|bp passoff p2=%0d f2=%0d", p2, f2);
    $assertnonvacuouson(0, bp); repeat (2) @(negedge clk); $display("T|4|bp nonvacon p2=%0d f2=%0d", p2, f2);
    $assertfailoff(0, bp); repeat (2) @(negedge clk); $display("T|5|bp failoff p2=%0d f2=%0d", p2, f2);
    $finish;
  end
endmodule
"#;
    assert_eq!(
        t_lines(src),
        [
            "T|0|passes=0 fails=2",
            "T|1|passon(nothing before) passes=0 fails=4",
            "T|2|passoff+passon passes=0 fails=6",
            "T|3|bp passoff p2=0 f2=8",
            "T|4|bp nonvacon p2=0 f2=10",
            "T|5|bp failoff p2=0 f2=10",
        ]
    );
}
