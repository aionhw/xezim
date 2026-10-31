//! §4.9.4 / §10.4.2: an NBA captures intent before later Active writes.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str, source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "xezim_nba_intent_{}_{}_{}",
            std::process::id(),
            label,
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

#[test]
fn equal_nba_intents_survive_later_active_writes_across_widths_and_source_order() {
    let mut source = "module top; logic clk=0;\n".to_owned();
    let mut prints = String::new();
    let mut expected = Vec::new();
    for width in [1usize, 63, 64, 65, 128, 129] {
        for kind in [
            "same", "separate", "inactive", "initial", "value", "last", "partial", "x", "z",
        ] {
            let name = format!("q{width}_{kind}");
            let initial = if kind == "x" {
                "'x"
            } else if kind == "z" {
                "'z"
            } else {
                "'0"
            };
            source.push_str(&format!("logic[{}:0] {name}={initial};\n", width - 1));
            let body = match kind {
                "same" => format!("always @(posedge clk) begin {name}<='0; {name}='1; end\n"),
                "separate" => {
                    format!("always @(posedge clk) {name}<='0; always @(posedge clk) {name}='1;\n")
                }
                "inactive" => {
                    format!("always @(posedge clk) begin {name}<='0; #0; {name}='1; end\n")
                }
                "initial" => format!("initial begin #1; {name}<='0; {name}='1; end\n"),
                "value" => format!(
                    "logic[{}:0] right{width}=0; always @(posedge clk) begin {name}<={name} ^ right{width}; {name}='1; end\n",
                    width - 1
                ),
                "last" => {
                    format!("always @(posedge clk) begin {name}<='1; {name}<='0; {name}='1; end\n")
                }
                "partial" => format!(
                    "always @(posedge clk) begin {name}[0]<=1'b1; {name}<='0; {name}='1; end\n"
                ),
                _ => format!("always @(posedge clk) begin {name}<={name}; {name}='0; end\n"),
            };
            source.push_str(&body);
            prints.push_str(&format!("$display(\"T|{width}|{kind}|%b\",{name});\n"));
            let digit = if kind == "x" {
                'x'
            } else if kind == "z" {
                'z'
            } else {
                '0'
            };
            expected.push(format!(
                "T|{width}|{kind}|{}",
                digit.to_string().repeat(width)
            ));
        }
    }
    source.push_str(&format!(
        "initial begin #1; clk=1; #1; {prints} $finish; end endmodule\n"
    ));
    let fixture = Fixture::new("regions", &source);
    for two_state in ["0", "1"] {
        let text = fixture.run(&[("XEZIM_NO_PARALLEL", "1"), ("XEZIM_TWO_STATE", two_state)]);
        assert_eq!(tags(&text), expected, "{text}");
    }
    #[cfg(feature = "jit")]
    {
        let text = fixture.run(&[
            ("XEZIM_NO_PARALLEL", "1"),
            ("XEZIM_TWO_STATE", "0"),
            ("XEZIM_JIT", "1"),
        ]);
        assert_eq!(tags(&text), expected, "{text}");
    }
}

#[test]
fn worker_fallback_retains_equal_nba_before_a_sequential_blocking_writer() {
    let mut expression = "q".to_owned();
    for n in 0..96u32 {
        expression = format!(
            "(({expression} ^ 32'h{:08x}) + SEED)",
            0x85ebca6bu32.wrapping_mul(n + 1)
        );
    }
    let source = format!(
        r#"module lane #(parameter int ID=0)(input logic clk,output logic[31:0] q=ID+1);
localparam logic[31:0] SEED=32'h9e3779b9*(ID+1);
always @(posedge clk) q<={expression};
endmodule
module top;
logic clk=0;logic[127:0] captured=0;wire[31:0] values[64];
for(genvar i=0;i<64;i++) begin:g lane #(.ID(i)) u(clk,values[i]);end
always @(posedge clk) captured<=128'b0;
always @(posedge clk) captured=128'b1;
initial begin #1;clk=1;#1;
$display("T|captured|%b",captured);
for(int i=0;i<64;i++) $display("T|lane|%0d|%08h",i,values[i]);
$finish;end endmodule"#
    );
    let fixture = Fixture::new("workers", &source);
    let mut expected = vec![format!("T|captured|{}", "0".repeat(128))];
    for id in 0..64u32 {
        let mut q = id + 1;
        let seed = 0x9e3779b9u32.wrapping_mul(id + 1);
        for n in 0..96u32 {
            q = (q ^ 0x85ebca6bu32.wrapping_mul(n + 1)).wrapping_add(seed);
        }
        expected.push(format!("T|lane|{id}|{q:08x}"));
    }
    let serial = fixture.run(&[("XEZIM_NO_PARALLEL", "1")]);
    assert_eq!(tags(&serial), expected, "{serial}");
    for workers in ["2", "4"] {
        let text = fixture.run(&[
            ("XEZIM_MULTICORE", "1"),
            ("XEZIM_THREADS", workers),
            ("XEZIM_FORCE_PARALLEL", "1"),
        ]);
        assert_eq!(tags(&text), expected, "{text}");
        assert!(counter(&text, "fallback_blocks=") > 0, "{text}");
        let issued = counter(&text, "credits_issued=");
        assert!(issued > 0, "{text}");
        assert_eq!(
            issued,
            counter(&text, "credits_retired=") + counter(&text, "credits_cancelled="),
            "{text}"
        );
        assert_eq!(counter(&text, "credit_pending="), 0, "{text}");
    }
}

#[test]
fn equal_array_element_nbas_survive_blocking_updates_with_and_without_packed_storage() {
    let source = r#"module top;
logic clk=0;integer index=1;logic[7:0] narrow[0:3];logic[128:0] wide[0:3];
initial begin for(int i=0;i<4;i++)begin narrow[i]=0;wide[i]=0;end end
always @(posedge clk) begin
 narrow[index]<=narrow[index];wide[index]<=wide[index];
 narrow[index]='1;wide[index]='1;
end
initial begin #1;clk=1;#1;
$display("T|narrow|%b",narrow[1]);$display("T|wide|%b",wide[1]);$finish;end endmodule"#;
    let fixture = Fixture::new("arrays", source);
    let expected = vec![
        format!("T|narrow|{}", "0".repeat(8)),
        format!("T|wide|{}", "0".repeat(129)),
    ];
    for packed in ["0", "1"] {
        for two_state in ["0", "1"] {
            let text = fixture.run(&[
                ("XEZIM_NO_PARALLEL", "1"),
                ("XEZIM_PACKED_MEM", packed),
                ("XEZIM_TWO_STATE", two_state),
            ]);
            assert_eq!(tags(&text), expected, "{text}");
        }
    }
}
