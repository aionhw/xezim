//! A warm design-cache HIT must REPLAY the elaboration diagnostics captured on
//! the cold run — otherwise §6.10 implicit-net warnings, the port-width lint,
//! unresolved-module notes, and width-underflow warnings silently vanish (they
//! are emitted during elaboration, which a cache hit skips). See option (b) of
//! the cache root-cause investigation.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    // Cargo names the binary it built for the integration tests.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xezim"))
}

#[test]
fn warm_cache_replays_elaboration_warnings() {
    let dir = std::env::temp_dir().join(format!("xezim_warmdiag_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("d.v");
    std::fs::write(
        &sv,
        "`timescale 1ns/1ns\n\
         module top;\n\
           reg x = 1;\n\
           assign dangling_net = x;  // §6.10 implicit 1-bit net\n\
           initial begin #1; $display(\"N=%b\", dangling_net); $finish; end\n\
         endmodule\n",
    )
    .expect("write sv");
    let cache = dir.join("cache");

    let run = || {
        // `--verbose`: the "[CACHE] miss/hit" lines asserted below.
        let out = Command::new(xezim_bin())
            .arg(&sv)
            .arg("--verbose")
            .arg("-s")
            .arg("top")
            .arg("--cache-dir")
            .arg(&cache)
            .arg("--max-time")
            .arg("10")
            .output()
            .expect("run xezim");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    // Run 1: cold (cache miss) — warning printed, artifact stored.
    let (o1, e1) = run();
    assert!(e1.contains("CACHE] miss"), "run1 should miss:\n{}", e1);
    assert!(
        e1.contains("implicit 1-bit net"),
        "cold run must emit the warning:\n{}",
        e1
    );

    // Run 2: warm (cache hit) — elaboration skipped, warning must be REPLAYED.
    let (o2, e2) = run();
    assert!(e2.contains("CACHE] hit"), "run2 should hit:\n{}", e2);
    assert!(
        e2.contains("implicit 1-bit net"),
        "warm hit must REPLAY the elaboration warning (option b):\n{}",
        e2
    );
    // Behavior identical across warm/cold.
    assert!(
        o1.contains("N=1") && o2.contains("N=1"),
        "sim output differs"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// §20.3: cached elaboration must not change the scope used to scale `$time`.
#[test]
fn cached_edge_block_keeps_its_time_scope() {
    let dir = std::env::temp_dir().join(format!("xezim_warmtime_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let child = dir.join("unit.sv");
    let top = dir.join("shell.sv");
    std::fs::write(
        &child,
        "module pulse_cell(input phase, output logic mark = 0);\n\
         always @(posedge phase) mark <= ~mark;\n\
         endmodule\n",
    )
    .expect("write child");
    std::fs::write(
        &top,
        "`timescale 10ps/1ps\n\
         module shell; logic phase = 0; wire mark; always #4 phase = ~phase;\n\
         pulse_cell u_cell(.phase(phase), .mark(mark));\n\
         always @(posedge phase) $display(\"STAMP=%0d\", $time);\n\
         initial #20 $finish; endmodule\n",
    )
    .expect("write top");
    let cache = dir.join("cache");

    let run = || {
        let out = Command::new(xezim_bin())
            .args(["--simulate", "-s", "shell", "--cache-dir"])
            .arg(&cache)
            .arg(&child)
            .arg(&top)
            .output()
            .expect("run xezim");
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    let cold = run();
    let warm = run();
    for text in [&cold, &warm] {
        assert!(
            text.contains("STAMP=4"),
            "first edge used the wrong unit:\n{text}"
        );
        assert!(
            text.contains("STAMP=12"),
            "second edge used the wrong unit:\n{text}"
        );
        assert!(
            !text.contains("STAMP=0"),
            "scope leaked across processes:\n{text}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
