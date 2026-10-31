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
