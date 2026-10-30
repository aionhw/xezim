//! IEEE 1800-2023 §4.6 / §10.4.2: event completion precedes ordered NBA publication.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str, source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "xezim_credits_{}_{}_{}",
            std::process::id(),
            label,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("top.sv");
        std::fs::write(&path, source).unwrap();
        Self(path)
    }

    fn run(&self, env: &[(&str, &str)], partition: Option<&Path>) -> String {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.args([
            "--simulate",
            "--no-cache",
            "--error-exit",
            "--profile",
            "-s",
            "top",
        ])
        .arg(&self.0);
        for name in [
            "XEZIM_NO_PARALLEL",
            "XEZIM_MULTICORE",
            "XEZIM_THREADS",
            "XEZIM_FORCE_PARALLEL",
            "XEZIM_PARALLEL_SERIALIZE",
            "XEZIM_TWO_STATE",
            "XEZIM_EDGE_MERGE",
            "XEZIM_JIT",
            "XEZIM_AOT",
        ] {
            cmd.env_remove(name);
        }
        cmd.env("XEZIM_EDGE_MERGE", "0").envs(env.iter().copied());
        if let Some(path) = partition {
            cmd.arg("--load-partition").arg(path);
        }
        let output = cmd.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{env:?}\n{text}");
        text
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn tags(text: &str) -> Vec<&str> {
    text.lines().filter(|line| line.starts_with("T|")).collect()
}

fn counter(text: &str, key: &str) -> u64 {
    text.split_whitespace()
        .find_map(|word| word.strip_prefix(key).and_then(|v| v.parse().ok()))
        .unwrap_or(0)
}

fn assert_conserved(text: &str) {
    let issued = counter(text, "credits_issued=");
    assert!(
        issued > 0,
        "event credit accounting was not exercised\n{text}"
    );
    assert_eq!(
        issued,
        counter(text, "credits_retired=") + counter(text, "credits_cancelled="),
        "{text}"
    );
    assert_eq!(counter(text, "credit_pending="), 0, "{text}");
}

fn expression(rounds: u32) -> String {
    let mut e = "q".to_string();
    for n in 0..rounds {
        e = format!(
            "(({e} ^ 32'h{:08x}) + SEED)",
            0x85ebca6bu32.wrapping_mul(n + 1)
        );
    }
    e
}

fn step(mut q: u32, id: u32, rounds: u32) -> u32 {
    let seed = 0x9e3779b9u32.wrapping_mul(id + 1);
    for n in 0..rounds {
        q = (q ^ 0x85ebca6bu32.wrapping_mul(n + 1)).wrapping_add(seed);
    }
    q
}

#[test]
fn changing_uneven_events_preserve_every_lane_with_idle_workers() {
    let rounds = [64, 128, 192, 320];
    let mut source = String::new();
    for (kind, &count) in rounds.iter().enumerate() {
        source.push_str(&format!(
            "module lane{kind} #(parameter int ID=0)(input logic clk, output logic[31:0] q=ID+1); localparam logic[31:0] SEED=32'h9e3779b9*(ID+1); always @(posedge clk) q <= {}; endmodule\n",
            expression(count)
        ));
    }
    source.push_str("module top; logic a=0,b=0,idle=0; wire[31:0] values[192];\n");
    for id in 0..192 {
        let clock = if id < 128 { "a" } else { "b" };
        source.push_str(&format!(
            "lane{} #(.ID({id})) u{id}({clock},values[{id}]);\n",
            id % 4
        ));
    }
    source.push_str(
        "initial begin #1; idle=1; #1; idle=0; #1; for(int epoch=0;epoch<12;epoch++) begin a=(epoch%3!=1); b=(epoch%3!=0); #1; a=0; b=0; #1; end for(int i=0;i<192;i++) $display(\"T|lane|%0d|%08h\",i,values[i]); $finish; end endmodule\n",
    );
    let fixture = Fixture::new("changing", &source);
    let expected: Vec<_> = (0..192u32)
        .map(|id| {
            let mut q = id + 1;
            for epoch in 0..12 {
                if (id < 128 && epoch % 3 != 1) || (id >= 128 && epoch % 3 != 0) {
                    q = step(q, id, rounds[id as usize % 4]);
                }
            }
            format!("T|lane|{id}|{q:08x}")
        })
        .collect();
    let serial = fixture.run(&[("XEZIM_NO_PARALLEL", "1")], None);
    assert_eq!(tags(&serial), expected);
    // Two occupied logical partitions leave workers idle and make one unit
    // much larger than the other, while both clock populations change size.
    let partition = fixture.0.parent().unwrap().join("skewed.part");
    std::fs::write(
        &partition,
        (0..192)
            .map(|id| if id == 191 { "7\n" } else { "0\n" })
            .collect::<String>(),
    )
    .unwrap();
    for threads in ["1", "2", "4"] {
        let parallel = fixture.run(
            &[
                ("XEZIM_MULTICORE", "1"),
                ("XEZIM_FORCE_PARALLEL", "1"),
                ("XEZIM_THREADS", threads),
            ],
            Some(&partition),
        );
        assert_eq!(tags(&parallel), expected, "{parallel}");
        assert!(counter(&parallel, "fast_blocks=") > 0, "{parallel}");
        assert_conserved(&parallel);
        if threads == "4" {
            let epochs = counter(&parallel, "credit_epochs=");
            let wakes = counter(&parallel, "worker_wakes=");
            assert!(
                epochs > 0 && wakes < 4 * epochs,
                "idle workers joined every event\n{parallel}"
            );
        }
    }
    let serialized = fixture.run(
        &[
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_FORCE_PARALLEL", "1"),
            ("XEZIM_THREADS", "4"),
            ("XEZIM_PARALLEL_SERIALIZE", "1"),
        ],
        Some(&partition),
    );
    assert_eq!(tags(&serialized), expected);
    assert_conserved(&serialized);
}

#[test]
fn late_unknowns_recover_without_duplicate_updates_or_early_nba_observation() {
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk, output logic[31:0] q=ID+1, output logic[31:0] stamp=0, mirror=0);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
always @(posedge clk) begin
  stamp<=SEED;
  q<={expression};
  mirror<=q;
  q<=({expression})^32'h12345678;
  stamp<=SEED+1;
end
endmodule
module top;
logic clk=0;
wire[31:0] values[192], stamps[192], mirrors[192];
logic[31:0] follow;
for(genvar i=0;i<192;i++) begin:g lane #(.ID(i)) u(clk,values[i],stamps[i],mirrors[i]); end
always @(values[7]) follow <= values[7]^32'hdeadbeef;
initial begin
  for(int epoch=0;epoch<16;epoch++) begin
    if(epoch==4) g[7].u.q=32'hx;
    if(epoch==9) g[7].u.q=32'h12345678;
    #1; clk=1; #1;
    $display("T|trace|%0d|%08h|%08h|%08h",epoch,values[7],mirrors[7],follow);
    clk=0;
  end
  for(int i=0;i<192;i++) $display("T|lane|%0d|%08h|%08h|%08h",i,values[i],stamps[i],mirrors[i]);
  $finish;
end
endmodule"#,
        expression = expression(96),
    );
    let fixture = Fixture::new("late_x", &source);
    let serial = fixture.run(&[("XEZIM_NO_PARALLEL", "1")], None);
    let mut expected = Vec::new();
    let mut lane7 = Some(8u32);
    for epoch in 0..16 {
        if epoch == 4 {
            lane7 = None;
        } else if epoch == 9 {
            lane7 = Some(0x12345678);
        }
        let mirror = lane7;
        lane7 = lane7.map(|q| step(q, 7, 96) ^ 0x12345678);
        let hex = |v: Option<u32>| v.map_or("xxxxxxxx".to_string(), |v| format!("{v:08x}"));
        expected.push(format!(
            "T|trace|{epoch}|{}|{}|{}",
            hex(lane7),
            hex(mirror),
            hex(lane7.map(|v| v ^ 0xdeadbeef))
        ));
    }
    for id in 0..192u32 {
        let mut q = id + 1;
        let mut mirror = 0;
        for epoch in 0..16 {
            if id == 7 && epoch == 9 {
                q = 0x12345678;
            }
            mirror = q;
            q = step(q, id, 96) ^ 0x12345678;
        }
        let stamp = 0x9e3779b9u32.wrapping_mul(id + 1).wrapping_add(1);
        expected.push(format!("T|lane|{id}|{q:08x}|{stamp:08x}|{mirror:08x}"));
    }
    assert_eq!(tags(&serial), expected);
    for threads in ["1", "2", "4"] {
        let parallel = fixture.run(
            &[
                ("XEZIM_MULTICORE", "1"),
                ("XEZIM_FORCE_PARALLEL", "1"),
                ("XEZIM_THREADS", threads),
            ],
            None,
        );
        assert_eq!(tags(&parallel), expected, "{parallel}");
        assert!(counter(&parallel, "fast_blocks=") > 0, "{parallel}");
        assert!(counter(&parallel, "fallback_blocks=") > 0, "{parallel}");
        assert_conserved(&parallel);
    }
}
