//! `--artifact-compression <none|1-22>`: the `-o` artifact can be written
//! raw (bincode, fastest load) or zstd-compressed at a chosen level; the
//! reader sniffs the zstd frame magic after the XEZIM header, so both kinds
//! load transparently.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

#[test]
fn artifact_modes_round_trip() {
    let bin = xezim_bin();
    if !bin.exists() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("xezim_artc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("t.sv"),
        "module tb;\n  logic [7:0] v = 8'hA7;\n  initial begin #1 $display(\"VAL %h\", v); $finish; end\nendmodule\n",
    )
    .expect("write");

    let compile = |out: &str, extra: &[&str]| {
        let mut c = Command::new(&bin);
        c.arg("--compile")
            .arg(dir.join("t.sv"))
            .args(["-s", "tb", "-o"])
            .arg(dir.join(out));
        c.args(extra);
        let o = c.output().expect("compile");
        assert!(
            String::from_utf8_lossy(&o.stdout).contains("Wrote compiled artifact"),
            "compile {out} failed:\n{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
    };
    compile("b_default", &[]);
    compile("b_none", &["--artifact-compression", "none"]);
    compile("b_19", &["--artifact-compression=19"]);

    let sz = |n: &str| std::fs::metadata(dir.join(n)).map(|m| m.len()).unwrap_or(0);
    assert!(
        sz("b_none") > sz("b_default"),
        "raw must be larger than zstd"
    );

    for n in ["b_default", "b_none", "b_19"] {
        // `--verbose`: "Loaded compiled:" is part of the run banner.
        let o = Command::new(&bin)
            .arg("--simulate")
            .arg("--verbose")
            .arg(dir.join(n))
            .output()
            .expect("simulate");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        let stdout = String::from_utf8_lossy(&o.stdout);
        let mut lines = stdout.lines();
        let expected_version = format!("=== xezim {} ===", env!("CARGO_PKG_VERSION"));
        let expected_commit = format!(
            "git {} ({})",
            env!("XEZIM_GIT_HASH"),
            env!("XEZIM_GIT_DATE")
        );
        assert_eq!(lines.next(), Some(expected_version.as_str()), "{n}: {text}");
        assert_eq!(lines.next(), Some(expected_commit.as_str()), "{n}: {text}");
        assert!(
            text.contains("Loaded compiled:"),
            "{n}: fast path; got:\n{text}"
        );
        assert!(text.contains("VAL a7"), "{n}: sim output; got:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// §7.5/§7.8: loading an artifact must preserve a dynamic array element's
/// associative type, including a second associative dimension and key types.
#[test]
fn dynamic_outer_associative_properties_survive_artifact_round_trip() {
    let bin = env!("CARGO_BIN_EXE_xezim");
    let dir = std::env::temp_dir().join(format!(
        "xezim_dynamic_assoc_artifact_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("design.sv");
    std::fs::write(&source, r#"
class row_map;
  int values[][string];
endclass
class nested_map;
  int values[][string][int];
endclass
module top;
  initial begin
    row_map rows; nested_map nested;
    rows=new; nested=new;
    rows.values=new[2]; nested.values=new[1];
    rows.values[1]["A"]=7;
    nested.values[0]["B"][3]=11;
    $display("T|one %0d %0d %0d %0d", rows.values[0].size(), rows.values[1].size(), rows.values[1].exists("A"), rows.values[1]["A"]);
    $display("T|two %0d %0d %0d", nested.values[0].size(), nested.values[0]["B"].size(), nested.values[0]["B"][3]);
  end
endmodule
"#).unwrap();
    for mode in ["none", "3"] {
        let artifact = dir.join(format!("design_{mode}.xez"));
        let compile = Command::new(bin)
            .args([
                "--compile",
                "--no-cache",
                "-s",
                "top",
                "--artifact-compression",
                mode,
            ])
            .arg(&source)
            .arg("-o")
            .arg(&artifact)
            .output()
            .unwrap();
        assert!(
            compile.status.success() && artifact.exists(),
            "compile: {}{}",
            String::from_utf8_lossy(&compile.stdout),
            String::from_utf8_lossy(&compile.stderr)
        );
        let loaded = Command::new(bin)
            .args(["--simulate", "--verbose"])
            .arg(&artifact)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&loaded.stdout);
        let stderr = String::from_utf8_lossy(&loaded.stderr);
        assert!(loaded.status.success(), "load: {stdout}{stderr}");
        assert!(
            format!("{stdout}{stderr}").contains("Loaded compiled:"),
            "artifact was not loaded: {stdout}{stderr}"
        );
        let lines: Vec<_> = stdout
            .lines()
            .filter(|line| line.starts_with("T|"))
            .collect();
        assert_eq!(
            lines,
            ["T|one 0 1 1 7", "T|two 1 1 11"],
            "{mode}: {stdout}{stderr}"
        );
    }
    // The previous serialized class layout must be rejected at the header,
    // before the reader can interpret its payload with missing dimensions.
    let current = xezim::XEZIM_BYTECODE_MAGIC[7];
    for mode in ["none", "3"] {
        for version in [current.wrapping_sub(1), current.wrapping_add(1)] {
            let mut bytes = std::fs::read(dir.join(format!("design_{mode}.xez"))).unwrap();
            bytes[7] = version;
            let stale = dir.join(format!("stale_{mode}_{version}.xez"));
            std::fs::write(&stale, bytes).unwrap();
            let output = Command::new(bin)
                .arg("--simulate")
                .arg(stale)
                .output()
                .unwrap();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                output.status.code(),
                Some(1),
                "incompatible artifact did not exit cleanly: {text}"
            );
            assert!(
                text.contains("incompatible xezim artifact version")
                    && text.contains("recompile with current xezim"),
                "{text}"
            );
            assert!(
                !text.contains("T|") && !text.contains("panicked"),
                "incompatible artifact reached execution or parsing: {text}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
