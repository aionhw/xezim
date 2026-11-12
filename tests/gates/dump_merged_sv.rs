//! `--dump-merged-sv <file>`: every source, fully preprocessed (`ifdef
//! branches resolved, macros expanded, `includes inlined), concatenated in
//! parse order into ONE self-contained .sv — a standalone repro for debugging
//! parse/elaboration problems in multi-file `-f` builds.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

#[test]
fn merged_dump_resolves_ifdefs_and_reruns_standalone() {
    let bin = xezim_bin();
    if !bin.exists() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("xezim_merged_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sv"),
        "`define WIDTH 8\n`ifdef USE_A\nmodule picked_a(); endmodule\n`else\n\
         module picked_b(input [`WIDTH-1:0] d); endmodule\n`endif\n",
    )
    .expect("write lib");
    std::fs::write(
        dir.join("tb.sv"),
        "module tb();\n  logic [7:0] x = 8'hA5;\n  picked_b u(.d(x));\n\
         initial begin $display(\"VAL %0h\", x); #1 $finish; end\nendmodule\n",
    )
    .expect("write tb");
    let merged = dir.join("merged.sv");

    let out = Command::new(&bin)
        .args(["--simulate", "-s", "tb", "--dump-merged-sv"])
        .arg(&merged)
        .arg(dir.join("lib.sv"))
        .arg(dir.join("tb.sv"))
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("Wrote merged preprocessed SV"),
        "dump confirmation missing; got:\n{text}"
    );
    assert!(
        text.contains("VAL a5"),
        "original run must still simulate; got:\n{text}"
    );

    let m = std::fs::read_to_string(&merged).expect("merged file written");
    assert!(
        m.contains("picked_b"),
        "`else branch must be selected:\n{m}"
    );
    assert!(
        !m.contains("picked_a"),
        "dead `ifdef branch must be gone:\n{m}"
    );
    assert!(m.contains("[8-1:0]"), "`WIDTH macro must be expanded:\n{m}");
    assert!(
        m.contains("===== file 1/2"),
        "per-file provenance banner:\n{m}"
    );

    // The whole point: the merged artifact re-runs standalone.
    let out2 = Command::new(&bin)
        .args(["--simulate", "-s", "tb"])
        .arg(&merged)
        .output()
        .expect("re-run merged");
    let text2 = format!(
        "{}{}",
        String::from_utf8_lossy(&out2.stdout),
        String::from_utf8_lossy(&out2.stderr)
    );
    assert!(
        text2.contains("VAL a5"),
        "merged file must simulate identically; got:\n{text2}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
