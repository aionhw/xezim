//! Active edge bodies sample before the NBA region, including pending overrides.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "xezim_active_nba_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("top.sv"), source).unwrap();
        Self(path)
    }
    fn run(&self, env: &[(&str, &str)]) -> String {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command
            .args([
                "--simulate",
                "--no-cache",
                "--error-exit",
                "--profile",
                "-s",
                "top",
            ])
            .arg(self.0.join("top.sv"));
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("XEZIM_") {
                command.env_remove(name);
            }
        }
        command
            .env("XEZIM_EDGE_MERGE", "0")
            .envs(env.iter().copied());
        let output = command.output().unwrap();
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
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tags(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.starts_with("T|"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn first_active_edge_samples_known_and_unknown_pre_nba_values_at_every_width() {
    let mut source = "module top;logic clk=0;\n".to_string();
    let mut assignments = String::new();
    let mut prints = String::new();
    let mut expected = Vec::new();
    for width in [1, 63, 64, 65, 127, 128, 129] {
        for (kind, init, bit) in [("known", "'0", '0'), ("x", "'x", 'x'), ("z", "'z", 'x')] {
            let name = format!("q_{width}_{kind}");
            source.push_str(&format!("logic[{width}-1:0] {name}={init},sample_{name},right_{name}=0;int fires_{name}=0;always@(posedge clk) begin sample_{name}={name} ^ right_{name};fires_{name}++;{name}<={name} ^ right_{name};end\n"));
            assignments.push_str(&format!("{name}<={width}'d1;"));
            prints.push_str(&format!(
                "$display(\"T|sample|{width}|{kind}|%0d|%b|%b\",fires_{name},sample_{name},{name});"
            ));
            let bits: String = std::iter::repeat_n(bit, width as usize).collect();
            expected.push(format!("T|sample|{width}|{kind}|1|{bits}|{bits}"));
        }
    }
    source.push_str(&format!(
        "initial begin #1;{assignments}clk=1;#1;{prints}$finish;end endmodule"
    ));
    let fixture = Fixture::new(&source);
    for env in [
        vec![("XEZIM_NO_PARALLEL", "1")],
        vec![("XEZIM_NO_PARALLEL", "1"), ("XEZIM_TWO_STATE", "0")],
        vec![("XEZIM_NO_PARALLEL", "1"), ("XEZIM_EVENT_EDGE", "0")],
    ] {
        assert_eq!(tags(&fixture.run(&env)), expected, "{env:?}");
    }
}

#[test]
fn edge_spawned_active_and_zero_delay_work_finishes_before_nba_publication() {
    let source = r#"
module top;
logic clk=0;logic[127:0] q=0,sample='x,right=0;int children=0;
always@(posedge clk) begin
 fork
  begin sample=q;#0;q<=q^right;children++;end
 join_none
end
initial begin
 #1;q<=128'd1;clk=1;
 #1;$display("T|child|%032h|%032h|%0d",sample,q,children);$finish;
end
endmodule
"#;
    let output = Fixture::new(source).run(&[("XEZIM_NO_PARALLEL", "1")]);
    assert_eq!(
        tags(&output),
        [format!("T|child|{}|{}|1", "0".repeat(32), "0".repeat(32))]
    );
}

#[test]
fn later_blocking_stores_and_intervening_clock_edges_preserve_sample_order() {
    let source = r#"
module top;
localparam logic[64:0] HIGH=65'h10000000000000000;
logic clk=0;logic[64:0] q=0,sample,right=0;int fires=0;
always@(posedge clk) begin sample=q;fires++;q<=q^right;end
initial begin
 #1;q<=65'd1;q=HIGH|65'd2;clk=1;
 #1;$display("T|stage|1|%h|%h|%0d",sample,q,fires);clk=0;q=HIGH|65'd3;
 #1;clk=1;
 #1;$display("T|stage|2|%h|%h|%0d",sample,q,fires);clk=0;
 #1;q<=65'd1;clk=1;
 #1;$display("T|stage|3|%h|%h|%0d",sample,q,fires);$finish;
end
endmodule
"#;
    let output = Fixture::new(source).run(&[("XEZIM_NO_PARALLEL", "1")]);
    assert_eq!(
        tags(&output),
        [
            "T|stage|1|10000000000000002|10000000000000002|1",
            "T|stage|2|10000000000000003|10000000000000003|2",
            "T|stage|3|10000000000000003|10000000000000003|3",
        ]
    );
}

fn count(text: &str, key: &str) -> u64 {
    text.lines()
        .filter(|line| line.starts_with("[PROF] multicore "))
        .flat_map(str::split_whitespace)
        .find_map(|part| part.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("missing {key}\n{text}"))
        .parse()
        .unwrap()
}

#[test]
fn pending_same_target_nba_stays_canonical_while_unrelated_lanes_use_credit_workers() {
    let mut expression = "acc".to_string();
    for n in 0..192u32 {
        expression = format!(
            "(({expression} ^ 32'h{:08x})+SEED)",
            0x85ebca6bu32.wrapping_mul(n + 1)
        );
    }
    let source = format!(
        r#"
module lane#(parameter int ID=0)(input logic clk,output logic[31:0] acc);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
initial acc=ID+1;always@(posedge clk) acc<={expression};
endmodule
module top;
logic clk=0;logic[127:0] q=0,right=0;wire[31:0] values[64];
for(genvar i=0;i<64;i++) begin:g lane#(.ID(i)) u(clk,values[i]);end
always@(posedge clk) q<=q^right;
initial begin
 #1;q<=128'd1;clk=1;
 #1;$display("T|pending|%032h",q);
 for(int i=0;i<64;i++) $display("T|lane|%0d|%08h",i,values[i]);$finish;
end
endmodule
"#
    );
    let mut expected = vec![format!("T|pending|{}", "0".repeat(32))];
    for i in 0..64u32 {
        let mut value = i + 1;
        let seed = 0x9e3779b9u32.wrapping_mul(i + 1);
        for n in 0..192u32 {
            value = (value ^ 0x85ebca6bu32.wrapping_mul(n + 1)).wrapping_add(seed);
        }
        expected.push(format!("T|lane|{i}|{value:08x}"));
    }
    let fixture = Fixture::new(&source);
    assert_eq!(tags(&fixture.run(&[("XEZIM_NO_PARALLEL", "1")])), expected);
    for workers in ["2", "4"] {
        let output = fixture.run(&[
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", workers),
            ("XEZIM_FORCE_PARALLEL", "1"),
            ("XEZIM_MULTICORE_WIDE_LOGIC", "0"),
        ]);
        assert_eq!(tags(&output), expected, "workers={workers}");
        assert!(count(&output, "credit_epochs") > 0);
        assert!(count(&output, "fast_blocks") > 0);
        assert_eq!(
            count(&output, "credits_issued"),
            count(&output, "credits_retired") + count(&output, "credits_cancelled")
        );
        assert_eq!(count(&output, "credit_pending"), 0);
    }
}
