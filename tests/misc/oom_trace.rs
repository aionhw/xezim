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
