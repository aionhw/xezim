//! A warm design-cache HIT must REPLAY the elaboration diagnostics captured on
//! the cold run — otherwise §6.10 implicit-net warnings, the port-width lint,
//! unresolved-module notes, and width-underflow warnings silently vanish (they
//! are emitted during elaboration, which a cache hit skips). See option (b) of
//! the cache root-cause investigation.

use std::process::Command;

fn xezim_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim")
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
        let out = Command::new(xezim_bin())
            .arg(&sv)
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
    assert!(o1.contains("N=1") && o2.contains("N=1"), "sim output differs");

    let _ = std::fs::remove_dir_all(&dir);
}
