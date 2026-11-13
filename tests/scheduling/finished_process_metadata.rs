//! IEEE 1800-2023 §9.3.2, §9.6, §9.7, §13.4.4: retiring fork metadata
//! must preserve observable process control and inherited scopes.

fn run(source: &str, expected: &str) {
    let sim = xezim::simulate(source, 10000).expect("process lifetime fixture");
    assert_eq!(sim.error_count, 0, "{:?}", sim.output);
    assert!(sim.output.iter().any(|line| line.message == expected));
}

#[test]
fn retained_handles_survive_later_forks() {
    run(
        include_str!("../issue_cases/finished_process_handles.sv"),
        "HANDLES_PASS completed=201 t=414",
    );
}

#[test]
fn children_keep_function_task_class_and_instance_scopes() {
    run(
        include_str!("../issue_cases/finished_process_scopes.sv"),
        "SCOPES_PASS a=10 b=20",
    );
}

#[test]
fn disabled_children_keep_status_but_cannot_resume() {
    run(
        include_str!("../issue_cases/finished_process_disable.sv"),
        "DISABLE_PASS bad=0",
    );
}

// Diagnostics belong to xezim; the fixtures exercise §9.3.2 / §9.4.1,
// §10.4.2 and Annex D.9 without prescribing a portable diagnostic spelling.
fn diagnostic_fixture(name: &str, extra: &[&str]) -> std::process::Output {
    let dir = std::env::temp_dir().join(format!("xezim_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .env_remove("XEZIM_X_WARN")
        .env_remove("XEZIM_VALUE_TRACE")
        .env_remove("XEZIM_TRACE_SCHED")
        .env("XEZIM_STALL_LIMIT", "1000")
        .args(["--no-cache"])
        .args(extra)
        .arg(format!(
            "{}/tests/issue_cases/{name}.sv",
            env!("CARGO_MANIFEST_DIR")
        ))
        .output()
        .expect("fork diagnostic fixture");
    std::fs::remove_dir_all(dir).unwrap();
    out
}

#[test]
fn ended_child_keeps_same_slot_stall_origin() {
    let out = diagnostic_fixture("finished_process_stall", &[]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("process 1 — fork child at"), "{err}");
}

#[test]
fn ended_child_keeps_deferred_x_write_origin() {
    let out = diagnostic_fixture("finished_process_xdiag", &["--x-warn"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    for signal in ["s0.q", "s0.s"] {
        let warning = err
            .split("[xezim][warning]")
            .find(|part| part.contains(&format!("X on '{signal}'")))
            .unwrap_or_else(|| panic!("missing delayed write warning: {err}"));
        let words = warning.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(words.contains("written by: - fork child at"), "{err}");
    }
}

#[test]
fn restarted_child_keeps_stall_origin() {
    let out = diagnostic_fixture("finished_process_restart", &[]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("restarted from"), "{err}");
    assert!(err.contains("process 1 — fork child at"), "{err}");
}

// Annex D.9: a permanent checkpoint can come from an earlier invocation.
#[test]
fn externally_saved_child_keeps_stall_origin() {
    let dir = std::env::temp_dir().join(format!("xezim_external_restart_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for save in [true, false] {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.current_dir(&dir)
            .env_remove("XEZIM_X_WARN")
            .env_remove("XEZIM_VALUE_TRACE")
            .env_remove("XEZIM_TRACE_SCHED")
            .env("XEZIM_STALL_LIMIT", "1000")
            .arg("--no-cache")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/issue_cases/finished_process_external_restart.sv"
            ));
        if save {
            cmd.args(["--save-at", "5ns,child.ckpt"]);
        }
        let out = cmd.output().expect("external checkpoint fixture");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{err}");
        assert!(dir.join("child.ckpt").exists(), "{err}");
        assert!(err.contains("restarted from"), "{err}");
        assert!(err.contains("process 1 — fork child at"), "{err}");
        // Reset only the external marker, retaining the earlier checkpoint.
        std::fs::remove_file(dir.join("child.flag")).unwrap();
    }
    std::fs::remove_dir_all(dir).unwrap();
}
