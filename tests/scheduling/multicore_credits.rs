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
    // A heavily skewed logical partition must split across execution lanes
    // while both clock populations change size.
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
            let lanes = std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(4);
            assert_eq!(counter(&parallel, "workers="), lanes as u64, "{parallel}");
            assert!(
                counter(&parallel, "credits_issued=") > 2 * epochs,
                "oversized partition stayed indivisible\n{parallel}"
            );
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

fn guarded_lanes(unknown: bool) -> (Fixture, Vec<String>) {
    let injection = if unknown {
        "if(epoch==4) noise=32'hx; if(epoch==9) noise=0;"
    } else {
        ""
    };
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk, reset, enable, input logic[31:0] noise,
output logic[31:0] q=ID+1, output logic[31:0] stamp=0, mirror=0);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
always @(posedge clk) begin
  stamp<=SEED;
  if(reset) begin q<=ID+1; mirror<=0; end
  else begin
    if(enable) begin
      q<={expression};
      q<=({expression})^noise;
      mirror<=q;
    end
  end
  stamp<=SEED+1;
end
endmodule
module top;
logic clk=0, reset=0, enable=0;
logic[31:0] noise=0;
wire[31:0] values[192], stamps[192], mirrors[192];
for(genvar i=0;i<192;i++) begin:g
  lane #(.ID(i)) u(clk,reset,enable,(i==7 ? noise : 32'h0),values[i],stamps[i],mirrors[i]);
end
initial begin
  for(int epoch=0;epoch<16;epoch++) begin
    reset=(epoch==0 || epoch==9);
    enable=(epoch%3!=1);
    {injection}
    #1; clk=1; #1;
    $display("T|guard|%0d|%08h|%08h|%08h",epoch,values[7],stamps[7],mirrors[7]);
    clk=0;
  end
  for(int i=0;i<192;i++) $display("T|lane|%0d|%08h|%08h|%08h",i,values[i],stamps[i],mirrors[i]);
  $finish;
end
endmodule"#,
        expression = expression(96),
    );
    let fixture = Fixture::new(if unknown { "guarded_x" } else { "guarded" }, &source);
    let mut expected = Vec::new();
    let mut values: Vec<_> = (1..=192u32).map(Some).collect();
    let mut mirrors = vec![Some(0); 192];
    let hex = |v: Option<u32>| v.map_or("xxxxxxxx".to_string(), |v| format!("{v:08x}"));
    for epoch in 0..16 {
        for id in 0..192u32 {
            let i = id as usize;
            if epoch == 0 || epoch == 9 {
                values[i] = Some(id + 1);
                mirrors[i] = Some(0);
            } else if epoch % 3 != 1 {
                mirrors[i] = values[i];
                values[i] = if unknown && id == 7 && (4..9).contains(&epoch) {
                    None
                } else {
                    values[i].map(|q| step(q, id, 96))
                };
            }
        }
        let stamp = 0x9e3779b9u32.wrapping_mul(8).wrapping_add(1);
        expected.push(format!(
            "T|guard|{epoch}|{}|{stamp:08x}|{}",
            hex(values[7]),
            hex(mirrors[7])
        ));
    }
    for id in 0..192u32 {
        let stamp = 0x9e3779b9u32.wrapping_mul(id + 1).wrapping_add(1);
        expected.push(format!(
            "T|lane|{id}|{}|{stamp:08x}|{}",
            hex(values[id as usize]),
            hex(mirrors[id as usize])
        ));
    }
    (fixture, expected)
}

fn check_guarded_workers(unknown: bool) {
    let (fixture, expected) = guarded_lanes(unknown);
    let serial = fixture.run(&[("XEZIM_NO_PARALLEL", "1")], None);
    assert_eq!(tags(&serial), expected, "{serial}");
    for threads in ["2", "4"] {
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
        if unknown {
            assert!(counter(&parallel, "fallback_blocks=") > 0, "{parallel}");
        }
        assert_conserved(&parallel);
    }
}

#[test]
fn nested_reset_enable_workers_match_every_lane_and_previous_value() {
    check_guarded_workers(false);
}

#[test]
fn guarded_workers_skip_unknowns_then_roll_back_late_unknown_nbas() {
    check_guarded_workers(true);
}

#[test]
fn wide_slices_and_masked_worker_nbas_preserve_every_lane_and_recover_from_late_x() {
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk, reset, enable, input logic[128:0] source,
output logic[31:0] q=ID+1, stamp=0, mirror=0,
output logic[128:0] bus);
initial bus=129'h1_12345678_zzzzxxxx_89abcdef_01234567;
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
always @(posedge clk) begin
  stamp<=SEED;
  if(reset) begin
    q<=ID+1; mirror<=0;
    bus[127:96]<=SEED;
    bus[64:63]<=2'b10;
  end else if(enable) begin
    q<={expression};
    bus[127:96]<={expression};
    bus[67:60]<=source[71:64];
    bus[128:128]<=source[127];
    bus[63:63]<=source[64];
    mirror<=q;
  end
  stamp<=SEED+1;
end
endmodule
module top;
logic clk=0, reset=0, enable=0;
logic[128:0] source=129'h1_80000000_zz0000a5_00000000_0000000x;
wire[31:0] values[192], stamps[192], mirrors[192];
wire[128:0] buses[192];
for(genvar i=0;i<192;i++) begin:g
  lane #(.ID(i)) u(clk,reset,enable,(i==7 ? source : 129'h1_80000000_zz0000a5_00000000_0000000x),values[i],stamps[i],mirrors[i],buses[i]);
end
initial begin
  for(int epoch=0;epoch<16;epoch++) begin
    reset=(epoch==0 || epoch==9);
    enable=(epoch%3!=1);
    if(epoch==4) source[71:64]=8'hx;
    if(epoch==9) source[71:64]=8'ha5;
    #1; clk=1; #1;
    $display("T|wide|%0d|%08h|%08h|%08h|%b",epoch,values[7],stamps[7],mirrors[7],buses[7]);
    clk=0;
  end
  for(int i=0;i<192;i++) $display("T|lane|%0d|%08h|%08h|%08h|%b",i,values[i],stamps[i],mirrors[i],buses[i]);
  $finish;
end
endmodule"#,
        expression = expression(96),
    );
    let fixture = Fixture::new("wide_partial", &source);
    let mut values: Vec<_> = (1..=192u32).collect();
    let mut mirrors = vec![0; 192];
    let mut buses = vec![vec!['0'; 129]; 192];
    let put_word = |bus: &mut Vec<char>, lo: usize, width: usize, value: u32| {
        for bit in 0..width {
            bus[lo + bit] = if value & (1 << bit) != 0 { '1' } else { '0' };
        }
    };
    for bus in &mut buses {
        put_word(bus, 0, 32, 0x01234567);
        put_word(bus, 32, 32, 0x89abcdef);
        bus[64..80].fill('x');
        bus[80..96].fill('z');
        put_word(bus, 96, 32, 0x12345678);
        bus[128] = '1';
    }
    let mut expected = Vec::new();
    for epoch in 0..16 {
        for id in 0..192u32 {
            let i = id as usize;
            let seed = 0x9e3779b9u32.wrapping_mul(id + 1);
            if epoch == 0 || epoch == 9 {
                values[i] = id + 1;
                mirrors[i] = 0;
                put_word(&mut buses[i], 96, 32, seed);
                put_word(&mut buses[i], 63, 2, 2);
            } else if epoch % 3 != 1 {
                mirrors[i] = values[i];
                values[i] = step(values[i], id, 96);
                put_word(&mut buses[i], 96, 32, values[i]);
                if id == 7 && (4..9).contains(&epoch) {
                    buses[i][60..68].fill('x');
                } else {
                    put_word(&mut buses[i], 60, 8, 0xa5);
                }
                buses[i][128] = '1';
                buses[i][63] = if id == 7 && (4..9).contains(&epoch) {
                    'x'
                } else {
                    '1'
                };
            }
        }
        let stamp = 0x9e3779b9u32.wrapping_mul(8).wrapping_add(1);
        let bus: String = buses[7].iter().rev().collect();
        expected.push(format!(
            "T|wide|{epoch}|{:08x}|{stamp:08x}|{:08x}|{bus}",
            values[7], mirrors[7]
        ));
    }
    for id in 0..192u32 {
        let i = id as usize;
        let stamp = 0x9e3779b9u32.wrapping_mul(id + 1).wrapping_add(1);
        let bus: String = buses[i].iter().rev().collect();
        expected.push(format!(
            "T|lane|{id}|{:08x}|{stamp:08x}|{:08x}|{bus}",
            values[i], mirrors[i]
        ));
    }
    let serial = fixture.run(&[("XEZIM_NO_PARALLEL", "1")], None);
    assert_eq!(tags(&serial), expected, "{serial}");
    for threads in ["2", "4"] {
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

#[test]
fn guarded_wide_copy_workers_preserve_full_and_overlapping_partial_states() {
    let payload: Vec<char> = (0..271)
        .map(|bit| {
            if bit % 7 == 1 || bit % 7 == 4 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    let reset: Vec<char> = (0..271)
        .map(|bit| {
            if bit % 5 == 0 || bit % 5 == 2 {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    let payload_literal: String = payload.iter().rev().collect();
    let reset_literal: String = reset.iter().rev().collect();
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk, reset, enable, input logic[270:0] payload, reset_payload,
output logic[31:0] q=ID+1, stamp=0, mirror=0,
output logic[270:0] copy, output logic[399:0] bus);
initial begin copy='z; bus='z; end
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
always @(posedge clk) begin
  stamp<=SEED;
  if(reset) begin
    q<=ID+1; mirror<=0;
    copy<=reset_payload;
    bus[399:129]<=reset_payload;
  end else if(enable) begin
    q<={expression};
    bus[399:129]<=reset_payload;
    copy<=payload;
    bus[270:0]<=payload;
    mirror<=q;
  end
  stamp<=SEED+1;
end
endmodule
module top;
logic clk=0, reset=0, enable=0;
logic[270:0] payload, reset_payload;
initial begin payload=271'b{payload_literal}; reset_payload=271'b{reset_literal}; end
wire[31:0] values[192], stamps[192], mirrors[192];
wire[270:0] copies[192];
wire[399:0] buses[192];
for(genvar i=0;i<192;i++) begin:g
  lane #(.ID(i)) u(clk,reset,enable,(i==7 ? payload : 271'b{payload_literal}),reset_payload,values[i],stamps[i],mirrors[i],copies[i],buses[i]);
end
initial begin
  for(int epoch=0;epoch<16;epoch++) begin
    reset=(epoch==0 || epoch==9);
    enable=(epoch%3!=1);
    if(epoch==4) payload[128]=1'bx;
    if(epoch==10) reset_payload='0;
    if(epoch==11) begin payload=271'b{payload_literal}; reset_payload=271'b{reset_literal}; end
    #1; clk=1; #1;
    $display("T|copy|%0d|%08h|%08h|%08h|%b|%b",epoch,values[7],stamps[7],mirrors[7],copies[7],buses[7]);
    clk=0;
  end
  for(int i=0;i<192;i++) $display("T|lane|%0d|%08h|%08h|%08h|%b|%b",i,values[i],stamps[i],mirrors[i],copies[i],buses[i]);
  $finish;
end
endmodule"#,
        expression = expression(96),
    );
    let fixture = Fixture::new("wide_copy", &source);
    let mut values: Vec<_> = (1..=192u32).collect();
    let mut mirrors = vec![0; 192];
    let mut copies = vec![vec!['z'; 271]; 192];
    let mut buses = vec![vec!['z'; 400]; 192];
    let mut expected = Vec::new();
    for epoch in 0..16 {
        for id in 0..192u32 {
            let i = id as usize;
            if epoch == 0 || epoch == 9 {
                values[i] = id + 1;
                mirrors[i] = 0;
                copies[i] = reset.clone();
                buses[i][129..400].copy_from_slice(&reset);
            } else if epoch % 3 != 1 {
                mirrors[i] = values[i];
                values[i] = step(values[i], id, 96);
                let mut sampled = payload.clone();
                if id == 7 && (4..11).contains(&epoch) {
                    sampled[128] = 'x';
                }
                copies[i] = sampled.clone();
                buses[i][129..400].copy_from_slice(&reset);
                buses[i][0..271].copy_from_slice(&sampled);
            }
        }
        let stamp = 0x9e3779b9u32.wrapping_mul(8).wrapping_add(1);
        let copy: String = copies[7].iter().rev().collect();
        let bus: String = buses[7].iter().rev().collect();
        expected.push(format!(
            "T|copy|{epoch}|{:08x}|{stamp:08x}|{:08x}|{copy}|{bus}",
            values[7], mirrors[7]
        ));
    }
    for id in 0..192u32 {
        let i = id as usize;
        let stamp = 0x9e3779b9u32.wrapping_mul(id + 1).wrapping_add(1);
        let copy: String = copies[i].iter().rev().collect();
        let bus: String = buses[i].iter().rev().collect();
        expected.push(format!(
            "T|lane|{id}|{:08x}|{stamp:08x}|{:08x}|{copy}|{bus}",
            values[i], mirrors[i]
        ));
    }
    let serial = fixture.run(&[("XEZIM_NO_PARALLEL", "1")], None);
    assert_eq!(tags(&serial), expected, "{serial}");
    for threads in ["2", "4"] {
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
