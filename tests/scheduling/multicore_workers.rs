//! IEEE 1800-2023 §4.6 / §10.4.2: immutable evaluation, X fallback and ordered NBA updates.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn fixture(name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xezim_multicore_{}_{}_{}",
        std::process::id(),
        name,
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("top.sv");
    std::fs::write(&p, source).unwrap();
    p
}
fn design(unknown: bool) -> String {
    let mut e = "q".to_string();
    for n in 0..32u32 {
        e = format!(
            "(({e} ^ 32'h{:08x}) + (SEED << {}))",
            0x85ebca6bu32.wrapping_mul(n + 1),
            n % 7 + 1
        );
    }
    let inject = if unknown {
        "initial begin #2; if (ID==7) q=32'hx; end"
    } else {
        ""
    };
    format!(
        r#"module lane #(parameter int ID=0)(input logic clk,output logic[31:0] q=ID+1);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
{inject}
always @(posedge clk) q<={e};
endmodule
module top; logic clk=0; wire[31:0] values[192];
always #1 clk=~clk;
for(genvar i=0;i<192;i++) begin:g lane #(.ID(i)) u(clk,values[i]); end
initial begin repeat(160) @(negedge clk); for(int i=0;i<192;i++) $display("T|%0d|%08h",i,values[i]); $finish; end
endmodule"#
    )
}
fn run(path: &std::path::Path, extra: &[(&str, &str)]) -> String {
    run_with_args(path, extra, &[])
}
fn run_with_args(path: &std::path::Path, extra: &[(&str, &str)], args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.args([
        "--simulate",
        "--no-cache",
        "--profile",
        "--error-exit",
        "-s",
        "top",
    ])
    .arg(path)
    .args(args);
    for name in [
        "XEZIM_NO_PARALLEL",
        "XEZIM_MULTICORE",
        "XEZIM_THREADS",
        "XEZIM_FORCE_PARALLEL",
        "XEZIM_PARALLEL_SERIALIZE",
        "XEZIM_TWO_STATE",
    ] {
        cmd.env_remove(name);
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    text
}
fn tags(s: &str) -> Vec<&str> {
    s.lines().filter(|s| s.starts_with("T|")).collect()
}
fn counter(s: &str, key: &str) -> u64 {
    s.split_whitespace()
        .find_map(|p| p.strip_prefix(key).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
}
#[test]
fn persistent_word_workers_match_an_independent_oracle() {
    let path = fixture("words", &design(false));
    let serial = run(&path, &[("XEZIM_NO_PARALLEL", "1")]);
    let expected: Vec<String> = (0..192u32)
        .map(|i| {
            let mut q = i + 1;
            let seed = 0x9e3779b9u32.wrapping_mul(i + 1);
            for _ in 0..160 {
                for n in 0..32u32 {
                    q = (q ^ 0x85ebca6bu32.wrapping_mul(n + 1))
                        .wrapping_add(seed.wrapping_shl(n % 7 + 1));
                }
            }
            format!("T|{i}|{q:08x}")
        })
        .collect();
    assert_eq!(tags(&serial), expected);
    for threads in ["1", "2", "4"] {
        let parallel = run(
            &path,
            &[
                ("XEZIM_MULTICORE", "1"),
                ("XEZIM_THREADS", threads),
                ("XEZIM_FORCE_PARALLEL", "1"),
            ],
        );
        assert_eq!(tags(&parallel), expected);
        assert!(counter(&parallel, "fast_blocks=") > 0, "{parallel}");
    }
    let isolated = run(
        &path,
        &[
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", "4"),
            ("XEZIM_FORCE_PARALLEL", "1"),
            ("XEZIM_PARALLEL_SERIALIZE", "1"),
        ],
    );
    assert_eq!(tags(&isolated), expected);
    // More logical partitions than CPUs must still preserve canonical order
    // and stay within the configured physical worker limit.
    let partition = path.parent().unwrap().join("many.part");
    std::fs::write(
        &partition,
        (0..192)
            .map(|i| format!("{}\n", i % 64))
            .collect::<String>(),
    )
    .unwrap();
    let partitioned = run_with_args(
        &path,
        &[
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", "2"),
            ("XEZIM_FORCE_PARALLEL", "1"),
        ],
        &["--load-partition", partition.to_str().unwrap()],
    );
    assert!(
        partitioned.contains("[PART] loaded partition"),
        "{partitioned}"
    );
    assert_eq!(tags(&partitioned), expected);
    assert!(
        (1..=2).contains(&counter(&partitioned, "workers=")),
        "{partitioned}"
    );
}
#[test]
fn word_bail_discards_private_updates_before_four_state_fallback() {
    let path = fixture("unknown", &design(true));
    let serial = run(&path, &[("XEZIM_NO_PARALLEL", "1")]);
    let parallel = run(
        &path,
        &[
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", "4"),
            ("XEZIM_FORCE_PARALLEL", "1"),
        ],
    );
    assert_eq!(tags(&serial), tags(&parallel));
    assert!(tags(&parallel).contains(&"T|7|xxxxxxxx"));
    assert!(
        counter(&parallel, "fast_blocks=") > 0 && counter(&parallel, "fallback_blocks=") > 0,
        "{parallel}"
    );
}
#[test]
fn compiled_artifacts_apply_partition_and_profile_controls() {
    let path = fixture(
        "artifact",
        r#"module top; logic clk=0; int a,b; always #1 clk=~clk;
always @(posedge clk) a<=a+1; always @(negedge clk) b<=b+1;
initial begin #5 $display("T|done"); $finish; end endmodule"#,
    );
    let dir = path.parent().unwrap();
    let artifact = dir.join("design.xez");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--compile", "--no-cache", "-s", "top"])
        .arg(&path)
        .arg("-o")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(output.status.success());
    let partition = dir.join("plan.part");
    std::fs::write(&partition, "0\n1\n").unwrap();
    let graph = dir.join("graph.hgr");
    let profile = dir.join("profile.txt");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("--no-cache")
        .arg("--load-partition")
        .arg(partition)
        .arg("--emit-hypergraph")
        .arg(&graph)
        .arg("--write-profile")
        .arg(&profile)
        .arg(&artifact)
        .env("XEZIM_EDGE_BLOCK_STATS", "1")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && text.contains("[PART] loaded partition") && text.contains("k=2"),
        "{text}"
    );
    assert!(graph.exists() && profile.exists(), "{text}");
}
