//! IEEE 1800-2023 §9.3.2, §9.6, §9.7: ended fork children keep their
//! observable terminal status, but need no execution metadata.

fn check_ended_children(body: &str) {
    for n in [20, 200, 2000] {
        let source = format!(
            "module top; real temp=25.0; initial begin
             repeat ({n}) begin {body} end $finish; end endmodule"
        );
        let mut sim = crate::simulate(&source, 20 * n + 10).expect("fork lifetime fixture");
        assert_eq!(sim.error_count, 0);
        assert!(sim.finished);
        assert!(sim.next_pid >= 2 * n as usize);
        assert!(sim.finished_pids.len() + sim.killed_pids.len() >= 2 * n as usize);
        assert!(sim.process_parents.is_empty());
        assert!(sim.process_contexts.is_empty());
        assert!(sim.event_waiters.is_empty());
        assert!(sim.event_queue.is_empty());
        // Only this slot's two children may still be needed by a stall report.
        assert!(sim.process_origin.len() <= 3);
        sim.reset_stall_window();
        // The static initial procedure keeps its origin. Retired fork records
        // must no longer remain in the maps.
        assert!(
            sim.process_origin.len() <= 1,
            "origins after {n} forks: {}",
            sim.process_origin.len()
        );
        assert!(sim.process_scope_hint.is_empty());
        assert!(sim.process_m_scope.is_empty());
        assert!(sim.process_m_label.is_empty());
        assert!(sim.pid_src_file.len() <= 1);
        assert!(sim.fork_spawn_cont.is_empty());
    }
}

#[test]
fn joined_children_release_metadata() {
    if rerun_without_diagnostics("joined_children_release_metadata") {
        return;
    }
    check_ended_children("fork begin #20; end begin #10; end join");
}

#[test]
fn named_disabled_children_release_metadata() {
    if rerun_without_diagnostics("named_disabled_children_release_metadata") {
        return;
    }
    check_ended_children("fork : a begin #20; end begin @(temp); end join_any disable a;");
}

#[test]
fn disable_fork_children_release_metadata() {
    if rerun_without_diagnostics("disable_fork_children_release_metadata") {
        return;
    }
    check_ended_children("fork begin #20; end begin @(temp); end join_any disable fork;");
}

#[test]
fn join_none_disabled_children_release_metadata() {
    if rerun_without_diagnostics("join_none_disabled_children_release_metadata") {
        return;
    }
    check_ended_children("fork : a begin #20; end begin @(temp); end join_none #20; disable a;");
}

#[test]
fn scoped_children_release_metadata_but_live_children_keep_it() {
    if rerun_without_diagnostics("scoped_children_release_metadata_but_live_children_keep_it") {
        return;
    }
    // §23 / §27: inherited instance hints are needed while a child is live.
    let source = r#"
        module worker;
          real temp = 25.0;
          initial begin : work
            repeat (100) begin
              fork begin #2; end begin @(temp); end join_any
              disable fork;
            end
          end
        endmodule
        module top;
          for (genvar i=0; i<2; i++) begin : g
            worker w();
          end
          initial begin #210; $finish; end
        endmodule
    "#;
    let live = crate::simulate(source, 1).expect("live child fixture");
    assert!(!live.process_parents.is_empty());
    for pid in live.process_parents.keys() {
        assert!(live.process_origin.contains_key(pid));
        assert!(live.process_scope_hint.contains_key(pid));
        assert!(live.process_m_label.contains_key(pid));
    }
    let ended = crate::simulate(source, 1000).expect("ended child fixture");
    assert_eq!(ended.error_count, 0);
    assert!(ended.process_parents.is_empty());
    assert!(
        ended.process_origin.len() <= 3,
        "origins={} scopes={} labels={} chains={}",
        ended.process_origin.len(),
        ended.process_scope_hint.len(),
        ended.process_m_label.len(),
        ended.process_m_scope.len()
    );
    assert!(ended.process_scope_hint.len() <= 2);
    assert!(ended.process_m_label.len() <= 2);
    assert!(ended.process_m_scope.is_empty());
}

// Environment switches are process-wide; isolate only these tests instead
// of mutating the environment while other simulator tests run in parallel.
fn rerun_without_diagnostics(name: &str) -> bool {
    let vars = ["XEZIM_X_WARN", "XEZIM_VALUE_TRACE", "XEZIM_TRACE_SCHED"];
    if vars.iter().all(|v| std::env::var_os(v).is_none()) {
        return false;
    }
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        &format!("compiler::simulator::finished_process_lifetime_tests::{name}"),
        "--nocapture",
    ]);
    for var in vars {
        cmd.env_remove(var);
    }
    let out = cmd.output().expect("isolated lifetime test");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"));
    true
}

#[test]
fn declined_children_release_metadata_after_native_stack_unwinds() {
    if rerun_without_diagnostics("declined_children_release_metadata_after_native_stack_unwinds") {
        return;
    }
    // §9.6.2 / §9.7: cancellation can precede a child's first activation.
    let mut sim = crate::simulate(
        r#"
        module top;
          initial begin : parent
            fork begin : child #10; end join
          end
        endmodule
    "#,
        1,
    )
    .unwrap();
    let pid = *sim.process_parents.keys().next().unwrap();
    sim.stall_pid_hits.clear();
    sim.dpi_export_depth = 1;
    sim.release_killed_process_state(pid);
    sim.killed_pids.insert(pid);
    sim.process_parents.remove(&pid);
    sim.reset_stall_window();
    assert!(sim.process_origin.contains_key(&pid));
    sim.dpi_export_depth = 0;
    sim.reset_stall_window();
    assert!(!sim.process_origin.contains_key(&pid));
    assert!(!sim.process_scope_hint.contains_key(&pid));
    assert!(!sim.process_m_scope.contains_key(&pid));
    assert!(!sim.process_m_label.contains_key(&pid));
    assert!(sim.killed_pids.contains(&pid));
}

#[test]
fn restart_in_uncalled_bodies_retains_fork_origins() {
    // Annex D.9 / §13 / §8 / §26: inspect declarations before execution,
    // including subroutines that this particular run does not call.
    for (prefix, items) in [
        ("", "initial if (0) $restart(\"child.ckpt\");"),
        ("", "task restore_child; $restart(\"child.ckpt\"); endtask"),
        (
            "",
            "function void restore_child(); $restart(\"child.ckpt\"); endfunction",
        ),
        (
            "",
            "class C; task restore_child; $restart(\"child.ckpt\"); endtask endclass",
        ),
        (
            "",
            "class C; function void restore_child(); $restart(\"child.ckpt\"); endfunction endclass",
        ),
        (
            "package p; task restore_child; $restart(\"child.ckpt\"); endtask endpackage",
            "import p::*;",
        ),
        (
            "package p; function void restore_child(); $restart(\"child.ckpt\"); endfunction endpackage",
            "import p::*;",
        ),
        (
            "module child; initial if (0) $restart(\"child.ckpt\"); endmodule",
            "child u();",
        ),
        (
            "module child; bit clk = 0; always @(posedge clk) if (0) $restart(\"child.ckpt\"); endmodule",
            "child u();",
        ),
    ] {
        let source = format!(
            "{prefix} module top; {items}
            initial begin repeat (20) fork #2; #1; join #1; $finish; end endmodule"
        );
        let mut sim = crate::simulate(&source, 100).expect("restart declaration fixture");
        assert_eq!(sim.error_count, 0);
        assert!(sim.checkpoint_requested, "{source}");
        sim.reset_stall_window();
        assert_eq!(
            sim.process_origin
                .values()
                .filter(|o| o.1 == super::FORK_CHILD_ORIGIN)
                .count(),
            40
        );
    }
}

#[test]
fn retirement_sweep_preserves_live_children_and_clears_request() {
    if rerun_without_diagnostics("retirement_sweep_preserves_live_children_and_clears_request") {
        return;
    }
    // §9.3.2 / §9.7: a terminal sibling cannot retire a live child's origin.
    let mut sim = crate::simulate(
        "module top; initial begin fork #1; #20; join end endmodule",
        2,
    )
    .unwrap();
    let live = *sim.process_parents.keys().next().unwrap();
    let ended = *sim.finished_pids.iter().next().unwrap();
    // Exercise an actual native-guard decline; ordinary window deferrals
    // no longer request a sweep. Recreate the child's pre-retirement link.
    sim.dpi_export_depth = 1;
    sim.process_parents.insert(ended, 0);
    sim.release_killed_process_state(ended);
    sim.process_parents.remove(&ended);
    assert!(sim.retired_fork_sweep_needed);
    sim.reset_stall_window();
    assert!(sim.retired_fork_sweep_needed);
    assert!(sim.process_origin.contains_key(&ended));
    sim.dpi_export_depth = 0;
    sim.reset_stall_window();
    assert!(!sim.retired_fork_sweep_needed);
    assert!(!sim.process_origin.contains_key(&ended));
    assert!(sim.process_origin.contains_key(&live));
    sim.reset_stall_window();
    assert!(!sim.retired_fork_sweep_needed);
    assert!(sim.process_origin.contains_key(&live));
}

#[test]
fn ordinary_retirement_work_is_independent_of_static_origins() {
    if rerun_without_diagnostics("ordinary_retirement_work_is_independent_of_static_origins") {
        return;
    }
    // §9.3.2 / §9.4.1: one completed child per timestamp, alongside static
    // delayed procedures. Count reset entries, not elapsed time or RSS.
    for unrelated in [100, 1000, 10000] {
        let source = format!(
            "module top; {} initial begin
             repeat (200) fork #1; join #1; $finish; end endmodule",
            "initial #1000000; ".repeat(unrelated)
        );
        let mut sim = crate::simulate(&source, 1000).unwrap();
        assert_eq!(sim.error_count, 0);
        assert!(sim.finished);
        sim.reset_stall_window();
        assert_eq!(
            sim.retired_fork_reset_visits, 200,
            "{unrelated} static origins"
        );
        assert_eq!(sim.process_origin.len(), unrelated + 1);
        assert!(sim.retired_fork_pids.is_empty());
        assert!(!sim.retired_fork_sweep_needed);
        sim.reset_stall_window();
        assert_eq!(sim.retired_fork_reset_visits, 200);
    }
}

#[test]
fn native_sweep_shrinks_tables_then_ordinary_retirements_visit_only_queued_ids() {
    if rerun_without_diagnostics(
        "native_sweep_shrinks_tables_then_ordinary_retirements_visit_only_queued_ids",
    ) {
        return;
    }
    // §9.6.2 / §9.7: synthetic cancellation before activation lets us vary
    // the native and scheduler guards without relying on a C stack's timing.
    let mut sim = crate::simulate("module top; initial #1000000; endmodule", 1).unwrap();
    let span = sim.process_origin.values().next().unwrap().0;
    let add_child = |sim: &mut super::Simulator, pid| {
        sim.process_parents.insert(pid, 0);
        sim.process_origin
            .insert(pid, (span, super::FORK_CHILD_ORIGIN));
        sim.process_scope_hint.insert(pid, "top".into());
        sim.process_m_scope.insert(pid, vec!["child".into()]);
        sim.process_m_label.insert(pid, "child".into());
    };
    sim.dpi_export_depth = 1;
    for pid in 1..=10000 {
        add_child(&mut sim, pid);
        sim.release_killed_process_state(pid);
        sim.killed_pids.insert(pid);
        sim.process_parents.remove(&pid);
        sim.reset_stall_window();
    }
    assert!(sim.retired_fork_pids.is_empty());
    assert!(sim.retired_fork_sweep_needed);
    assert_eq!(sim.retired_fork_reset_visits, 0);
    let capacities = (
        sim.process_origin.capacity(),
        sim.process_scope_hint.capacity(),
        sim.process_m_scope.capacity(),
        sim.process_m_label.capacity(),
    );
    sim.dpi_export_depth = 0;
    sim.proc_depth = 2;
    sim.reset_stall_window();
    assert!(sim.retired_fork_sweep_needed);
    assert_eq!(sim.retired_fork_reset_visits, 0);
    sim.proc_depth = 0;
    sim.reset_stall_window();
    assert_eq!(sim.retired_fork_reset_visits, 10001);
    assert!(!sim.retired_fork_sweep_needed);
    assert_eq!(sim.process_origin.len(), 1);
    assert!(sim.process_scope_hint.is_empty());
    assert!(sim.process_m_scope.is_empty());
    assert!(sim.process_m_label.is_empty());
    // Relative capacity checks verify the requested shrink, without pinning
    // a hash table implementation's exact capacity or allocation policy.
    assert!(sim.process_origin.capacity() < capacities.0);
    assert!(sim.process_scope_hint.capacity() < capacities.1);
    assert!(sim.process_m_scope.capacity() < capacities.2);
    assert!(sim.process_m_label.capacity() < capacities.3);

    for i in 0..300 {
        let pid = 10001 + i;
        add_child(&mut sim, pid);
        match i % 3 {
            0 => sim.current_pid = pid,
            1 => {
                sim.stall_pid_hits.insert(pid, 1);
            }
            _ => sim.proc_depth = 2,
        }
        sim.release_killed_process_state(pid);
        sim.killed_pids.insert(pid);
        sim.process_parents.remove(&pid);
        assert!(!sim.retired_fork_sweep_needed);
        assert_eq!(sim.retired_fork_pids.len(), 1);
        if sim.proc_depth != 0 {
            sim.reset_stall_window();
            assert_eq!(sim.retired_fork_pids.len(), 1);
        }
        sim.current_pid = 0;
        sim.proc_depth = 0;
        sim.reset_stall_window();
        assert_eq!(sim.retired_fork_reset_visits, 10002 + i);
        assert_eq!(sim.process_origin.len(), 1);
        assert!(sim.retired_fork_pids.is_empty());
    }
    sim.reset_stall_window();
    assert_eq!(sim.retired_fork_reset_visits, 10301);
}
