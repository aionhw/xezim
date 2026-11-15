//! Guards against the explicit `$root` hierarchical reference being hijacked by
//! a same-named net inside the assigning instance (§23.6 absolute hierarchy).
//!
//! Upstream cousin #4 of #1123: an explicit `$root.top.n` on the RHS of a
//! cross-scope continuous drive was absolutely rooted, but a leaf-declared
//! `wire n;` under the active resolve hint rebound the read to `l.n` (x/z)
//! instead of the top module's `n`. Each driver's RHS must evaluate under its
//! own scope; an explicit `$root` reference is that reference, so the top
//! signal's value must win over any same-named net in the hinted scope.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct RunDir(std::path::PathBuf);
impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check(source: &str, expected: &[&str], no_warnings: bool) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = RunDir(std::env::temp_dir().join(format!(
        "xezim-root1124-{}-{unique}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    std::fs::write(dir.0.join("test.sv"), source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("XEZIM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .current_dir(&dir.0)
        .args(["--no-cache", "test.sv"])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    if no_warnings {
        assert!(!stderr.to_ascii_lowercase().contains("warning"), "{stderr}");
    }
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("T|") || line.starts_with("PASS") || line.starts_with("FAIL")
        })
        .collect();
    let mut actual = lines;
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{stdout}\n{stderr}");
}

/// An explicit `$root` read into a leaf that declares the same-named nets must
/// carry the top module's values, not the leaf's unreferenced wires.
#[test]
fn explicit_dollar_root_read_beats_same_named_leaf_nets() {
    check(
        r#"module leaf;
    wire n;
    wire [1:0] v2;
    logic n_root;
    logic [1:0] v2_root;
    assign n_root = $root.top.n;
    assign v2_root = ~$root.top.v2;
endmodule

module top;
    logic n = 1'b0;
    logic [1:0] v2 = 2'b10;
    leaf l ();
    initial begin
        #1;
        if (l.n_root !== 1'b0) begin
            $display("FAIL n_root got %b, expected 0", l.n_root);
        end else begin
            $display("PASS");
        end
        if (l.v2_root !== 2'b01) begin
            $display("FAIL v2_root got %b, expected 01", l.v2_root);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS", "PASS"],
        false,
    );
}

/// Without the leaf's same-named nets, explicit `$root` reads already worked;
/// guard that they still do after the hinted re-scoping fix.
#[test]
fn keeps_plain_dollar_root_read_into_leaf() {
    check(
        r#"module leaf;
    logic n_root;
    assign n_root = $root.top.n;
endmodule

module top;
    logic n = 1'b1;
    leaf l ();
    initial begin
        #1;
        if (l.n_root !== 1'b1) begin
            $display("FAIL n_root got %b, expected 1", l.n_root);
        end else begin
            $display("PASS");
        end
        $finish;
    end
endmodule
"#,
        &["PASS"],
        false,
    );
}