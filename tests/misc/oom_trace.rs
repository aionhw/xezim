//! Memory tracing identifies source work and preserves the command's result.

use std::process::{Command, Output};

#[test]
fn memory_trace_reports_files_instances_and_growth_without_changing_output() {
    let dir = std::env::temp_dir().join(format!("xezim_oom_cli_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("top\"é.sv");
    std::fs::write(
        dir.join("leaf.svh"),
        "\nmodule leaf; logic [7:0] data; initial data=7; endmodule\n",
    )
    .unwrap();
    std::fs::write(&source, "`include \"leaf.svh\"\nmodule top; leaf u(); logic [7:0] mem[4]; logic [7:0] matrix[2][3]; logic cube[2][2][2]; initial begin #1; $display(\"PASS memory trace\"); $finish; end endmodule\n").unwrap();
    let run = |trace: Option<&std::ffi::OsStr>, compile: bool| -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command
            .args([
                if compile { "--compile" } else { "--simulate" },
                "--no-cache",
                "--error-exit",
                "-s",
                "top",
            ])
            .arg(&source)
            .env_remove("XEZIM_OOM_TRACE");
        if let Some(trace) = trace {
            command.env("XEZIM_OOM_TRACE", trace);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let baseline = run(None, false);
    let path = dir.join("memory.jsonl");
    let traced = run(Some(path.as_os_str()), false);
    assert_eq!(baseline.stdout, traced.stdout);
    assert_eq!(baseline.stderr, traced.stderr);
    let records: Vec<serde_json::Value> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.iter().any(|r| {
        r["source"]
            .as_str()
            .is_some_and(|s| s.contains("leaf.svh:2"))
            && r["definition"] == "leaf"
    }));
    assert!(records.iter().any(|r| {
        r["source"]
            .as_str()
            .is_some_and(|s| s.contains("top\"é.sv:2"))
            && r["instance"] == "u"
    }));
    let model = records
        .iter()
        .find(|r| r["phase"] == "elaboration complete")
        .unwrap();
    assert!(
        model["counts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pair| pair[0] == "signals" && pair[1].as_u64().unwrap() > 0)
    );
    assert!(
        records
            .iter()
            .any(|r| r["phase"] == "retain instance context")
    );
    for (name, cells) in [("mem", 4), ("matrix", 6), ("cube", 8)] {
        assert!(records.iter().any(|r| r["phase"] == "allocate array"
            && r["instance"] == name
            && r["counts"][0][1] == cells));
    }
    let disabled = run(Some(std::ffi::OsStr::new("0")), false);
    assert_eq!(baseline.stdout, disabled.stdout);
    assert_eq!(baseline.stderr, disabled.stderr);
    let compiled = run(Some(std::ffi::OsStr::new("1")), true);
    assert!(
        String::from_utf8_lossy(&compiled.stderr)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .any(|r| r["phase"] == "elaboration complete")
    );
    let bad = run(Some(dir.join("missing/trace.jsonl").as_os_str()), false);
    assert_eq!(baseline.stdout, bad.stdout);
    assert!(String::from_utf8_lossy(&bad.stderr).contains("[oom-trace] cannot open trace"));
    std::fs::remove_dir_all(dir).unwrap();
}

/// One redacted `--compile` trace of `files`, as records plus the raw text.
fn redacted_trace(tag: &str, files: &[(&str, &str)], expect_ok: bool) -> (Vec<serde_json::Value>, String) {
    let dir = std::env::temp_dir().join(format!("xezim_oom_sum_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in files {
        std::fs::write(dir.join(name), text).unwrap();
    }
    let trace = dir.join("trace.jsonl");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--compile", "--no-cache", "-s", "top", "-o"])
        .arg(dir.join("top.xbc"))
        .arg(dir.join(files[0].0))
        .env("XEZIM_OOM_TRACE", &trace)
        .env("XEZIM_OOM_TRACE_REDACT", "1")
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        expect_ok,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&trace).unwrap();
    let records = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (records, text)
}

fn count(record: &serde_json::Value, name: &str) -> Option<u64> {
    record["counts"]
        .as_array()?
        .iter()
        .find(|c| c[0] == name)
        .and_then(|c| c[1].as_u64())
}

#[test]
fn memory_trace_summarizes_phases_definitions_binds_and_includes() {
    let (records, text) = redacted_trace(
        "ok",
        &[
            (
                "top.sv",
                "`include \"zq_leaf.svh\"\nmodule zq_chk(input logic c); endmodule\nmodule top; logic c; zq_leaf u0(.c(c)); zq_leaf u1(.c(c)); initial $finish; endmodule\nbind top.u0 zq_chk u_chk(.c(c));\n",
            ),
            (
                "zq_leaf.svh",
                "module zq_leaf(input logic c); logic [7:0] d; always_comb d = {8{c}}; endmodule\n",
            ),
        ],
        true,
    );
    let phase = |name: &str| records.iter().find(|r| r["event"] == "checkpoint" && r["phase"] == name);
    let environment = phase("environment").expect("environment record");
    if cfg!(target_os = "linux") {
        assert!(count(environment, "mem_total_kib").unwrap() > 0);
    }
    assert!(count(environment, "cgroup_limit_kib").is_some());
    let discovery = phase("bind.discovery_pass").expect("discovery record");
    assert_eq!(count(discovery, "unit_binds"), Some(1));
    assert_eq!(count(discovery, "binds_to_instance_path"), Some(1));
    assert_eq!(count(discovery, "defparams"), Some(0));
    let rebuild = phase("bind.rebuild").expect("rebuild record");
    assert_eq!(count(rebuild, "resolved_bind_targets"), Some(1));
    assert!(phase("bind.single_pass").is_none());
    let includes = phase("include file access").expect("include record");
    assert_eq!(count(includes, "include_directives"), Some(1));
    assert!(count(includes, "include_path_probes").unwrap() >= 1);
    assert!(count(includes, "include_file_reads").unwrap() >= 1);
    // Two leaves and the bound checker, in each of the two elaborations.
    let instances = records
        .iter()
        .find(|r| r["event"] == "phase_summary" && r["phase"] == "inline instance")
        .expect("instance phase summary");
    assert_eq!(count(instances, "scopes"), Some(6));
    let definitions: Vec<_> = records
        .iter()
        .filter(|r| r["event"] == "definition_summary")
        .collect();
    assert!(!definitions.is_empty());
    for (rank, record) in definitions.iter().enumerate() {
        assert_eq!(count(record, "rank"), Some(rank as u64 + 1));
        assert!(record["definition"].as_str().unwrap().starts_with("def#"));
    }
    assert!(definitions.windows(2).all(|pair| {
        count(pair[0], "self_grow_kib") >= count(pair[1], "self_grow_kib")
    }));
    assert_eq!(records.last().unwrap()["event"], "finish");
    assert!(!text.contains("zq_"), "{text}");
}

#[test]
fn memory_trace_records_the_class_of_a_failure_without_names() {
    let (records, text) = redacted_trace(
        "err",
        &[(
            "top.sv",
            "module zq_leaf; initial zq_missing = 1; endmodule\nmodule top; zq_leaf u(); endmodule\n",
        )],
        false,
    );
    let error = records
        .iter()
        .find(|r| r["event"] == "error")
        .expect("error record");
    let phase = error["phase"].as_str().unwrap();
    assert!(phase.ends_with(": Undeclared identifier"), "{phase}");
    assert!(!phase.starts_with(": "), "{phase}");
    assert!(!text.contains("zq_"), "{text}");
}
