//! IEEE 1800-2023 §13.3, §13.5, §9.3.2, §9.6: sharing task code
//! preserves invocation storage, output arguments and process control.
fn lines(source: &str) -> Vec<String> {
    let sim = xezim::simulate(source, 1_000_000).expect("task frame fixture");
    assert_eq!(sim.error_count, 0);
    assert!(sim.finished);
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .filter(|s| s.starts_with("T|"))
        .collect()
}
#[test]
fn invocation_storage_and_process_control() {
    assert_eq!(
        lines(include_str!("shared_task_frames.sv")),
        [
            "T|automatic 13 17 103 207",
            "T|static 1",
            "T|static 2",
            "T|static 3",
            "T|recursive 6",
            "T|control 12 0 3",
        ]
    );
}
#[test]
fn resolved_task_targets_stay_separate() {
    assert_eq!(
        lines(include_str!("shared_task_targets.sv")),
        ["T|targets 11 22 33 44"; 3]
    );
}

// Annex D.9 defines optional checkpoint system tasks; §13.3/§9.3.2 semantics
// must hold when calls resume after a saved slot.
#[test]
fn checkpoint_restart_preserves_unshared_task_calls() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("xezim_task_frames_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("test.sv"),
        include_str!("shared_task_checkpoint.sv"),
    )
    .unwrap();
    for restart in [false, true] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.current_dir(&dir).args(["--no-cache", "test.sv"]);
        if restart {
            cmd.arg("+again");
        }
        let out = cmd.output().expect("run checkpoint fixture");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stdout}\n{stderr}");
        assert!(!stderr.contains("Error:"), "{stderr}");
        assert_eq!(
            stdout
                .lines()
                .filter(|s| s.starts_with("T|"))
                .collect::<Vec<_>>(),
            ["T|checkpoint 12"]
        );
        if restart {
            assert!(stderr.contains("restarted"), "{stderr}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fork_children_disable_repeated_task_and_block_activations() {
    assert_eq!(
        lines(include_str!("shared_task_disable.sv")),
        ["T|disable 8 4"]
    );
}

#[test]
fn aggregate_locals_and_concurrent_static_storage() {
    assert_eq!(
        lines(include_str!("shared_task_values.sv")),
        ["T|values 21 75", "T|shared done"]
    );
}

// §13.3, §9.6.2: each call must reach its own cleanup and caller tail.
#[test]
fn distinct_task_call_sites_unwind_to_their_own_caller() {
    assert_eq!(
        lines(include_str!("shared_task_unwind.sv")),
        ["T|unwind 360 12"]
    );
}

// §16.4.2: task completion preserves pending deferred assertion reports.
#[test]
fn deferred_assertions_at_task_exit_survive_body_reuse() {
    assert_eq!(
        lines(include_str!("shared_task_deferred.sv")),
        [
            "T|observed",
            "T|observed",
            "T|observed",
            "T|postponed",
            "T|postponed",
            "T|postponed",
            "T|deferred done",
        ]
    );
}
