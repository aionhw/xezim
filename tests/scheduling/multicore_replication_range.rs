//! §4.6 / §10.4.2: guarded workers preserve replication and selected range bits.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str, source: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "xezim_replication_{}_{}_{}",
            std::process::id(),
            label,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("top.sv");
        std::fs::write(&path, source).unwrap();
        Self(path)
    }

    fn run(&self, workers: Option<&str>, replication: &str, wide_range: &str) -> String {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command.args([
            "--simulate",
            "--no-cache",
            "--error-exit",
            "--profile",
            "-s",
            "top",
        ]);
        command.arg(&self.0);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("XEZIM_") {
                command.env_remove(key);
            }
        }
        command
            .env("XEZIM_EDGE_MERGE", "0")
            .env("XEZIM_MULTICORE_REPLICATION", replication)
            .env("XEZIM_MULTICORE_WIDE_RANGE", wide_range);
        if let Some(workers) = workers {
            command
                .env("XEZIM_MULTICORE", "1")
                .env("XEZIM_FORCE_PARALLEL", "1")
                .env("XEZIM_THREADS", workers);
        } else {
            command.env("XEZIM_NO_PARALLEL", "1");
        }
        let output = command.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{text}");
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
    text.lines()
        .filter(|line| line.starts_with("[PROF] multicore "))
        .flat_map(str::split_whitespace)
        .find_map(|word| word.strip_prefix(key)?.parse().ok())
        .unwrap_or(0)
}
fn conserved(text: &str) {
    let issued = counter(text, "credits_issued=");
    assert!(issued > 0, "{text}");
    assert_eq!(
        issued,
        counter(text, "credits_retired=") + counter(text, "credits_cancelled="),
        "{text}"
    );
    assert_eq!(counter(text, "credit_pending="), 0, "{text}");
}
fn expression() -> String {
    let mut expression = "q".to_owned();
    for n in 0..96u32 {
        expression = format!(
            "(({expression} ^ 32'h{:08x}) + SEED)",
            0x85ebca6bu32.wrapping_mul(n + 1)
        );
    }
    expression
}
fn step(mut q: u32, id: u32) -> u32 {
    let seed = 0x9e3779b9u32.wrapping_mul(id + 1);
    for n in 0..96u32 {
        q = (q ^ 0x85ebca6bu32.wrapping_mul(n + 1)).wrapping_add(seed);
    }
    q
}
fn repeated(part: u8, width: usize, count: usize) -> String {
    (0..width * count)
        .rev()
        .map(|bit| {
            if (part >> (bit % width)) & 1 == 0 {
                '0'
            } else {
                '1'
            }
        })
        .collect()
}

#[test]
fn replication_extensions_execute_real_workers_and_match_every_lane() {
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk, one, input logic[2:0] part,
output logic[31:0] q=ID+1, output logic[62:0] narrow, mixed,
output logic[128:0] wide_value, output logic[63:0] filled);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
initial begin narrow=0; mixed=0; wide_value=0; filled=0; end
always @(posedge clk) begin
  q<={expression};
  narrow<={{21{{part}}}};
  mixed<={{21{{part ^ 3'b101}}}};
  wide_value<={{43{{part}}}};
  filled<={{64{{one}}}};
end
endmodule
module top;
logic clk=0, one=0; logic[2:0] part=0;
wire[31:0] values[64]; wire[62:0] narrows[64], mixeds[64];
wire[128:0] wides[64]; wire[63:0] fills[64];
for(genvar i=0;i<64;i++) begin:g
  lane #(.ID(i)) u(clk,one,part,values[i],narrows[i],mixeds[i],wides[i],fills[i]);
end
initial begin
  for(int epoch=0;epoch<6;epoch++) begin
    part=(epoch*3+1)&7; one=epoch&1;
    #1; clk=1; #1; clk=0;
  end
  for(int i=0;i<64;i++) $display("T|repl|%0d|%08h|%b|%b|%b|%b",i,values[i],narrows[i],mixeds[i],wides[i],fills[i]);
  $finish;
end
endmodule"#,
        expression = expression()
    );
    let fixture = Fixture::new("known", &source);
    let expected: Vec<_> = (0..64u32)
        .map(|id| {
            let mut q = id + 1;
            for _ in 0..6 {
                q = step(q, id);
            }
            format!(
                "T|repl|{id}|{q:08x}|{}|{}|{}|{}",
                repeated(0, 3, 21),
                repeated(5, 3, 21),
                repeated(0, 3, 43),
                repeated(1, 1, 64)
            )
        })
        .collect();
    assert_eq!(tags(&fixture.run(None, "0", "0")), expected);
    for workers in ["2", "4"] {
        for enabled in ["0", "1"] {
            let text = fixture.run(Some(workers), enabled, "0");
            assert_eq!(tags(&text), expected, "{text}");
            conserved(&text);
            assert_eq!(
                counter(&text, "workers="),
                workers.parse::<u64>().unwrap(),
                "run with at least four available CPUs\n{text}"
            );
            if enabled == "1" {
                assert!(counter(&text, "fast_blocks=") > 0, "{text}");
            } else {
                assert_eq!(counter(&text, "fast_blocks="), 0, "{text}");
            }
        }
    }
}

#[test]
fn selected_wide_ranges_ignore_outer_unknowns_and_recover_after_late_partial_rollback() {
    let source_bits: Vec<char> = (0..320)
        .map(|bit| {
            if bit % 5 == 1 || bit % 5 == 3 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    let initial: Vec<char> = (0..257)
        .map(|bit| {
            if bit % 7 == 2 || bit % 7 == 5 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    let source_literal: String = source_bits.iter().rev().collect();
    let initial_literal: String = initial.iter().rev().collect();
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk, input logic[2:0] part,
input logic[319:0] source, output logic[31:0] q=ID+1, stamp=0, output logic[256:0] bus);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
initial bus=257'b{initial_literal};
always @(posedge clk) begin
  q<={expression};
  stamp<=SEED;
  bus[4:2]<=part;
  bus[192:64]<=source[191:63];
  bus[189:61]<=source[255:127];
  stamp<=SEED+1;
end
endmodule
module top;
logic clk=0; logic[2:0] part=0;
logic[319:0] source=320'b{source_literal};
wire[31:0] values[64], stamps[64]; wire[256:0] buses[64];
for(genvar i=0;i<64;i++) begin:g
  lane #(.ID(i)) u(clk,part,source,values[i],stamps[i],buses[i]);
end
initial begin
  source[0]=1'bx; source[319]=1'bz;
  for(int epoch=0;epoch<6;epoch++) begin
    part=(epoch*3+1)&7;
    if(epoch==2) source[240]=1'bx;
    if(epoch==3) source[240]=1'bz;
    if(epoch==4) source[240]=1'b{restored};
    #1; clk=1; #1;
    $display("T|range|%0d|%08h|%08h|%b",epoch,values[7],stamps[7],buses[7]);
    clk=0;
  end
  for(int i=0;i<64;i++) $display("T|lane|%0d|%08h|%08h|%b",i,values[i],stamps[i],buses[i]);
  $finish;
end
endmodule"#,
        expression = expression(),
        restored = source_bits[240]
    );
    let fixture = Fixture::new("selected", &source);
    let mut q: Vec<_> = (1..=64u32).collect();
    let mut bus = initial;
    let mut expected = Vec::new();
    for epoch in 0..6 {
        let part = ((epoch * 3 + 1) & 7) as u8;
        for bit in 0..3 {
            bus[2 + bit] = if (part >> bit) & 1 == 0 { '0' } else { '1' };
        }
        bus[64..193].copy_from_slice(&source_bits[63..192]);
        bus[61..190].copy_from_slice(&source_bits[127..256]);
        if epoch == 2 {
            bus[174] = 'x';
        }
        if epoch == 3 {
            bus[174] = 'z';
        }
        for id in 0..64 {
            q[id] = step(q[id], id as u32);
        }
        let stamp = 0x9e3779b9u32.wrapping_mul(8).wrapping_add(1);
        let bits: String = bus.iter().rev().collect();
        expected.push(format!("T|range|{epoch}|{:08x}|{stamp:08x}|{bits}", q[7]));
    }
    let bits: String = bus.iter().rev().collect();
    for id in 0..64 {
        let stamp = 0x9e3779b9u32.wrapping_mul(id + 1).wrapping_add(1);
        expected.push(format!(
            "T|lane|{id}|{:08x}|{stamp:08x}|{bits}",
            q[id as usize]
        ));
    }
    assert_eq!(tags(&fixture.run(None, "0", "0")), expected);
    for workers in ["2", "4"] {
        for enabled in ["0", "1"] {
            let text = fixture.run(Some(workers), "0", enabled);
            assert_eq!(tags(&text), expected, "{text}");
            conserved(&text);
            assert_eq!(
                counter(&text, "workers="),
                workers.parse::<u64>().unwrap(),
                "run with at least four available CPUs\n{text}"
            );
            assert!(counter(&text, "fallback_blocks=") > 0, "{text}");
            if enabled == "1" {
                assert!(counter(&text, "fast_blocks=") > 0, "{text}");
            } else {
                assert_eq!(counter(&text, "fast_blocks="), 0, "{text}");
            }
        }
    }
}
