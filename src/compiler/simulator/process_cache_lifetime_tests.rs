#[test]
fn completed_fork_children_release_execution_caches() {
    let sim = crate::simulate(
        r#"
        module top;
          int count;
          initial begin
            for (int n=0; n<200; n++) begin
              fork begin
                for (int i=0; i<8; i++) count++;
                #1;
              end join
            end
            if (count != 1600) $fatal(1, "child execution changed");
            $finish;
          end
        endmodule
    "#,
        1000,
    )
    .expect("fork cache lifetime fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim.finished_pids.len() >= 200);
    assert!(sim
        .process_loop_bytecode
        .keys()
        .all(|(p, _, _)| !sim.finished_pids.contains(p)));
    assert!(sim
        .process_cond_bytecode
        .keys()
        .all(|(p, _, _)| !sim.finished_pids.contains(p)));
    assert!(sim
        .forever_cont_cache
        .keys()
        .all(|(p, _, _, _, _)| !sim.finished_pids.contains(p)));
    assert!(sim
        .forever_sens_cache
        .keys()
        .all(|(p, _)| !sim.finished_pids.contains(p)));
}

#[test]
fn killed_process_releases_caches_but_live_process_keeps_them() {
    let source = r#"
        module top;
          process victim;
          int count;
          initial begin
            fork begin
              victim = process::self();
              forever begin
                for (int i=0; i<8; i++) count++;
                #1;
              end
            end join_none
            #5; victim.kill();
            #2;
            if (count != 40) $fatal(1, "killed process continued");
            $finish;
          end
        endmodule
    "#;
    let live = crate::simulate(source, 3).expect("live process cache fixture");
    assert!(live.killed_pids.is_empty());
    assert!(!live.forever_cont_cache.is_empty());
    let killed = crate::simulate(source, 100).expect("killed process cache fixture");
    assert_eq!(killed.error_count, 0);
    assert!(!killed.killed_pids.is_empty());
    assert!(killed
        .process_loop_bytecode
        .keys()
        .all(|(p, _, _)| !killed.killed_pids.contains(p)));
    assert!(killed
        .process_cond_bytecode
        .keys()
        .all(|(p, _, _)| !killed.killed_pids.contains(p)));
    assert!(killed
        .forever_cont_cache
        .keys()
        .all(|(p, _, _, _, _)| !killed.killed_pids.contains(p)));
    assert!(killed
        .forever_sens_cache
        .keys()
        .all(|(p, _)| !killed.killed_pids.contains(p)));
}

#[test]
fn disabled_fork_children_release_execution_caches() {
    let sim = crate::simulate(
        r#"
        module top;
          int count;
          initial begin
            for (int n=0; n<50; n++) begin
              fork
                forever begin
                  for (int i=0; i<8; i++) count++;
                  #1;
                end
              join_none
              #2;
              disable fork;
            end
            if (count != 800) $fatal(1, "disabled children ran on: %0d", count);
            $finish;
          end
        endmodule
    "#,
        1000,
    )
    .expect("disable fork cache fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim.killed_pids.len() >= 50);
    assert!(sim
        .process_loop_bytecode
        .keys()
        .all(|(p, _, _)| !sim.killed_pids.contains(p)));
    assert!(sim
        .forever_cont_cache
        .keys()
        .all(|(p, _, _, _, _)| !sim.killed_pids.contains(p)));
    assert!(sim
        .forever_sens_cache
        .keys()
        .all(|(p, _)| !sim.killed_pids.contains(p)));
    assert!(sim.exec_cache_pids.iter().all(|p| !sim.killed_pids.contains(p)));
}

#[test]
fn suspended_repeat_does_not_retain_a_body_per_iteration() {
    fn run(n: usize) -> usize {
        let source = r#"
            module top;
              int count;
              initial begin
                repeat (ITERATIONS) begin
                  fork begin
                    for (int i=0; i<8; i++) count++;
                    #1;
                  end join
                end
                if (count != ITERATIONS*8) $fatal(1, "repeat count changed");
                $finish;
              end
            endmodule
        "#
        .replace("ITERATIONS", &n.to_string());
        let sim = crate::simulate(&source, (n + 10) as u64).expect("repeat cache fixture");
        assert_eq!(sim.error_count, 0);
        assert!(sim.finished);
        sim.blocking_cont_cache.len()
    }
    let small = run(20);
    let large = run(2000);
    assert!(
        large <= small + 4,
        "repeat retains iteration bodies: small={small}, large={large}"
    );
}

#[test]
fn suspended_repeat_keeps_nested_and_concurrent_counters_separate() {
    let sim = crate::simulate(r#"
        module top;
          int evaluations;
          int completed;
          function int limit(); evaluations++; return 3; endfunction
          task automatic work(input int id);
            int seen = 0;
            repeat (limit()) begin
              repeat (2) begin
                #1; seen++;
                if (seen % 2) continue;
              end
              if (seen == 4) break;
            end
            if (seen != 4) $fatal(1,"nested repeat count changed");
            repeat (2) begin #1; continue; end
            completed++;
          endtask
          initial begin
            fork work(0); work(1); join
            if (evaluations != 2 || completed != 2) $fatal(1,"count reevaluated or continue leaked");
            $display("REPEAT_PASS evaluations=%0d completed=%0d",evaluations,completed);
            $finish;
          end
        endmodule
    "#, 100).expect("nested/concurrent repeat fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim
        .output
        .iter()
        .any(|o| o.message.contains("REPEAT_PASS evaluations=2 completed=2")));
}

#[test]
fn disabled_fork_children_release_captured_collections() {
    let sim = crate::simulate(
        r#"
        class worker;
          task step(int n);
            int q[$];
            for (int i = 0; i < 4; i++) q.push_back(i + n);
            fork
              begin #10; if (q.size() != 4) $fatal(1, "capture lost"); end
              begin #1; end
            join_any
            disable fork;
          endtask
        endclass
        module top;
          initial begin
            worker w = new();
            for (int n = 0; n < 20; n++) w.step(n);
            $finish;
          end
        endmodule
    "#,
        1000,
    )
    .expect("disable fork capture fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim.fork_collection_captures.is_empty());
    assert!(sim.fork_capture_refs.is_empty());
    assert!(sim.deferred_collection_cleanup.is_empty());
    assert!(sim.fork_local_scopes.is_empty());
    assert!(sim.fork_gen_pids.is_empty());
    assert!(sim.fork_pid_gens.is_empty());
}

/// IEEE 1800-2023 §9.3.2, §13.3: repeated task calls share code while
/// each invocation still waits for its own fork children.
#[test]
fn blocking_task_calls_share_continuation_frames() {
    fn run(n: usize) -> (usize, usize) {
        let source = r#"
            module top;
              int count;
              task automatic work();
                fork
                  begin #2; count++; end
                  begin #1; end
                join
              endtask
              initial begin
                repeat (ITERATIONS) work();
                if (count != ITERATIONS) $fatal(1, "task count changed");
                $finish;
              end
            endmodule
        "#
        .replace("ITERATIONS", &n.to_string());
        let sim = crate::simulate(&source, (2 * n + 10) as u64).expect("task cache fixture");
        assert_eq!(sim.error_count, 0);
        assert!(sim.finished);
        (sim.blocking_cont_cache.len(), sim.task_frame_cache.len())
    }
    let small = run(20);
    let large = run(2000);
    assert_eq!(large, small, "task calls retain continuation frames");
    assert_eq!(large.1, 1, "one body per reused call site");
}

/// IEEE 1800-2023 §13.3, §9.3.2, §9.4.1: one-shot blocking calls
/// retain no shared body; only reused sites of tasks containing forks retain one.
#[test]
fn blocking_task_call_sites_share_only_after_reuse() {
    for target in ["bare", "module", "interface"] {
        for fork in [false, true] {
            for sites in [3, 30] {
                for reuse in [false, true] {
                    let body = if fork {
                        "fork begin #2; end begin #1; end join count++;"
                    } else {
                        "#2; count++;"
                    };
                    let task = format!("int count; task automatic work(); {body} endtask");
                    let (prefix, items, name, count) = if target == "bare" {
                        (String::new(), task, "work", "count")
                    } else {
                        (
                            format!("{target} worker; {task} end{target}"),
                            "worker w();".to_string(),
                            "w.work",
                            "w.count",
                        )
                    };
                    let mut calls = format!("{name}();\n").repeat(sites);
                    if reuse {
                        calls.push_str(&format!("repeat (3) {name}();"));
                    }
                    let total = sites + if reuse { 3 } else { 0 };
                    let source = format!(
                        "{prefix} module top; {items} initial begin {calls}
                         if ({count} != {total}) $fatal(1, \"task count changed\");
                         $finish; end endmodule"
                    );
                    let sim = crate::simulate(&source, 100).expect("task call sites fixture");
                    assert_eq!(sim.error_count, 0);
                    assert!(sim.finished);
                    assert_eq!(sim.task_frame_eligibility.len(), 1);
                    assert_eq!(sim.task_frame_eligibility.values().next().unwrap().1, fork);
                    assert_eq!(
                        sim.task_frame_cache.len(),
                        if fork { sites + usize::from(reuse) } else { 0 }
                    );
                    let bodies = sim
                        .task_frame_cache
                        .values()
                        .filter(|(_, f)| f.is_some())
                        .count();
                    assert_eq!(
                        bodies,
                        usize::from(fork && reuse),
                        "{target}: {sites} sites"
                    );
                    assert_eq!(
                        sim.blocking_cont_cache.len(),
                        if fork {
                            4 * (sites + if reuse { 2 } else { 0 })
                        } else {
                            0
                        }
                    );
                }
            }
        }
    }
}

/// Annex D.9, §13.3/§13.4, §8, §25, §26: even an uncalled routine or a dead
/// branch can contain an optional checkpoint system task. All module task invocations
/// must then retain the original per-call statement owners.
#[test]
fn checkpoint_designs_keep_task_continuation_frames() {
    let contexts = [
        ("", "initial if (0) $save(\"unused.ckpt\");"),
        ("", "task unused(); $incsave(\"unused.ckpt\"); endtask"),
        (
            "",
            "function void unused(); $restart(\"unused.ckpt\"); endfunction",
        ),
        (
            "",
            "class C; task unused(); $save(\"unused.ckpt\"); endtask endclass",
        ),
        (
            "",
            "class C; function void unused(); $incsave(\"unused.ckpt\"); endfunction endclass",
        ),
        (
            "package p; task unused(); $restart(\"unused.ckpt\"); endtask endpackage",
            "",
        ),
        (
            "package p; function void unused(); $save(\"unused.ckpt\"); endfunction endpackage",
            "",
        ),
        (
            "module child; initial if (0) $incsave(\"unused.ckpt\"); endmodule",
            "child c();",
        ),
        (
            "module child; task unused(); $restart(\"unused.ckpt\"); endtask endmodule",
            "child c();",
        ),
        (
            "interface bus; initial if (0) $save(\"unused.ckpt\"); endinterface",
            "bus b();",
        ),
        (
            "interface bus; task unused(); $incsave(\"unused.ckpt\"); endtask endinterface",
            "bus b();",
        ),
    ];
    for (prefix, items) in contexts {
        let run = |n| {
            let source = format!(
                r#"
                {prefix}
                module top;
                  {items}
                  int count;
                  task automatic work();
                    fork begin #2; count++; end begin #1; end join
                  endtask
                  initial begin
                    repeat ({n}) work();
                    if (count != {n}) $fatal(1, "task count changed");
                    $finish;
                  end
                endmodule
            "#
            );
            let sim = crate::simulate(&source, 10000).expect("checkpoint guard fixture");
            assert_eq!(sim.error_count, 0, "{prefix} {items}");
            assert!(sim.finished);
            assert!(!sim.share_task_frames, "missed {prefix} {items}");
            assert!(sim.task_frame_cache.is_empty());
        };
        // Sharing stays off; the size of main's own continuation cache is not pinned.
        run(20);
        run(2000);
    }
}

/// §5.9: text mentioning a system task is not a call to that task.
#[test]
fn checkpoint_names_in_strings_do_not_disable_task_sharing() {
    let sim = crate::simulate(
        r#"
        module top;
          string names = "$save $incsave $restart";
          task automatic work(); fork begin #1; end begin #2; end join endtask
          initial begin repeat (20) work(); $finish; end
        endmodule
    "#,
        100,
    )
    .expect("checkpoint text fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim.finished);
    assert!(sim.share_task_frames);
    assert_eq!(sim.task_frame_cache.len(), 1);
    assert_eq!(sim.blocking_cont_cache.len(), 8);
}

/// §13.3, §23.6, §25: the resolved hierarchical entry uses the same
/// checkpoint eligibility as a bare module task call.
#[test]
fn hierarchical_task_frames_obey_checkpoint_guard() {
    for kind in ["module", "interface"] {
        for checkpoint in [false, true] {
            for n in [20, 2000] {
                let guard = if checkpoint {
                    "initial if (0) $restart(\"unused.ckpt\");"
                } else {
                    ""
                };
                let source = format!(
                    r#"
                    {kind} worker;
                      int count;
                      task automatic work();
                        fork begin #2; count++; end begin #1; end join
                      endtask
                    end{kind}
                    module top;
                      worker w();
                      {guard}
                      initial begin
                        repeat ({n}) w.work();
                        if (w.count != {n}) $fatal(1, "hierarchical task count changed");
                        $finish;
                      end
                    endmodule
                "#
                );
                let sim = crate::simulate(&source, 10000).expect("hierarchical frame fixture");
                assert_eq!(sim.error_count, 0);
                assert!(sim.finished);
                assert_eq!(sim.share_task_frames, !checkpoint);
                assert_eq!(sim.task_frame_cache.len(), usize::from(!checkpoint));
                assert_eq!(
                    sim.blocking_cont_cache.len(),
                    if checkpoint { 4 * n } else { 8 }
                );
            }
        }
    }
}

/// IEEE 1800-2023 §9.3–§9.4, §12.4–§12.7, §15.5.4, §16.3, §18.16–§18.17:
/// eligibility visits every nested statement arm, even unexecuted syntax.
/// Nested calls/waits need an owner too; flat task calls stay covered by
/// `blocking_task_call_sites_share_only_after_reuse` without added frames.
#[test]
fn blocking_task_frame_eligibility_visits_nested_statements() {
    let nested = [
        "fork #1; join",
        "begin : nested fork #1; join end",
        "if (1) fork #1; join",
        "if (0) ; else fork #1; join",
        "case (0) 0: fork #1; join endcase",
        "randcase 1: fork #1; join endcase",
        "for (int i = 0; i < 1; i++) fork #1; join",
        "foreach (a[i]) fork #1; join",
        "while (0) fork #1; join",
        "do fork #1; join while (0);",
        "repeat (1) fork #1; join",
        "forever fork #1; join",
        "#1 fork #1; join",
        "@(e) fork #1; join",
        "wait (1) fork #1; join",
        "assert (1) fork #1; join",
        "assert (0) begin end else fork #1; join",
        "wait_order (e, f) fork #1; join",
        "wait_order (e, f) begin end else fork #1; join",
        "randsequence (s) s: { fork #1; join }; endsequence",
    ];
    for (body, eligible) in nested.into_iter().map(|body| (body, true)).chain([
        ("child();", true),
        ("$display(\"fork\");", false),
        ("wait fork; disable fork;", true),
        ("#1; #1;", true),
    ]) {
        let source = format!(
            "module top;
               event e, f; int a[1]; int count;
               task child(); fork #1; join endtask
               task automatic work(); #1; if (0) begin {body} end count++; endtask
               initial begin
                 repeat (3) work();
                 if (count != 3) $fatal(1, \"task count changed\");
                 $finish;
               end
             endmodule"
        );
        let sim = crate::simulate(&source, 100).expect("nested fork eligibility fixture");
        assert_eq!(sim.error_count, 0, "{body}");
        assert!(sim.finished, "{body}");
        assert_eq!(sim.task_frame_eligibility.len(), 1, "{body}");
        assert_eq!(
            sim.task_frame_eligibility.values().next().unwrap().1,
            eligible,
            "{body}"
        );
        assert_eq!(sim.task_frame_cache.len(), usize::from(eligible), "{body}");
        assert!(
            sim.task_frame_cache
                .values()
                .all(|(_, frame)| frame.is_some()),
            "{body}"
        );
    }
}

/// IEEE 1800-2023 §13.3, §9.6.2, §16.4.2: control-flow guards must exercise
/// promoted sites even when sharing is restricted to tasks containing forks.
#[test]
fn blocking_task_control_fixtures_reach_shared_bodies() {
    for (source, declarations, sites, output) in [
        (
            include_str!("../../../tests/scheduling/shared_task_unwind.sv"),
            1,
            4,
            "T|unwind 360 12",
        ),
        (
            include_str!("../../../tests/scheduling/shared_task_deferred.sv"),
            2,
            2,
            "T|deferred done",
        ),
    ] {
        let sim = crate::simulate(source, 1_000_000).expect("shared control fixture");
        assert_eq!(sim.error_count, 0);
        assert!(sim.finished);
        assert!(sim.output.iter().any(|o| o.message == output));
        assert_eq!(sim.task_frame_eligibility.len(), declarations);
        assert_eq!(sim.task_frame_cache.len(), sites);
        assert!(
            sim.task_frame_cache
                .values()
                .all(|(_, frame)| frame.is_some())
        );
    }
}

/// #1127 / §9.3.1 / §13.3: a nested timed block has the same owner lifetime
/// requirement as a fork, while each call still runs its own activation.
#[test]
fn nested_timed_task_blocks_share_continuation_owners() {
    let run = |calls: u64| {
        let source = format!(
            "module top; int count; task automatic tk(); begin #2; #1; count++; end endtask initial begin repeat ({calls}) tk(); if(count!={calls}) $fatal(1); $finish; end endmodule"
        );
        let sim = crate::simulate(&source, calls * 3 + 10).unwrap();
        assert_eq!(sim.error_count, 0);
        assert!(sim.finished);
        assert_eq!(sim.task_frame_cache.len(), 1);
        sim.blocking_cont_cache.len()
    };
    assert_eq!(run(20), run(2000));
}
