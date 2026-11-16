//! #721, IEEE 1800-2023 §3.12.1 / §6.7 / §23.10 / §27.5: compilation-unit
//! variables and nets stay visible where a module, a unit subroutine or a
//! unit class names them, `$unit::` reaches past a local declaration, and
//! instances whose parameter values prepare equal bodies still drive their
//! own nets with their own values. Expected lines checked against the
//! reference simulator.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// §6.20 / §23.10: sharing symbolic preparation must preserve each
/// instance's widths, signed initializer, net drivers and parameter values.
#[test]
fn symbolic_preparation_preserves_instance_values() {
    let fixture = Fixture::new(&[("design.sv", r#"
module leaf #(parameter W=4, V=3)(output wire [W-1:0] out);
  localparam K=V+1;
  logic signed [W-1:0] data = -K;
  wire [W-1:0] n = data;
  assign out = n;
endmodule
module top;
  wire [3:0] a, c;
  wire [8:0] b;
  leaf #(.W(4), .V(3)) u0(a);
  leaf #(.W(9), .V(7)) u1(b);
  leaf #(.W(4), .V(3)) u2(c);
  initial begin
    #1;
    $display("T|%h %h %h", a, b, c);
    $display("T|%0d %0d %0d", $bits(u0.data), $bits(u1.data), $bits(u2.data));
    $display("T|%0d %0d %0d", u0.data, u1.data, u2.data);
    $finish;
  end
endmodule
"#)]);
    fixture.check_all_paths("design.sv", &["T|c 1f8 c", "T|4 9 4", "T|-4 -8 -4"]);
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "xezim-unit-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        for (name, source) in files {
            std::fs::write(path.join(name), source).unwrap();
        }
        Self(path)
    }
    fn output(&self, args: &[&str], env: &[(&str, &str)]) -> (bool, String, Vec<String>) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command.args(args).arg("--error-exit").current_dir(&self.0);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("XEZIM_") {
                command.env_remove(name);
            }
        }
        command.envs(env.iter().copied());
        let out = command.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let lines = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.starts_with("T|"))
            .map(str::to_owned)
            .collect();
        (out.status.success(), text, lines)
    }
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Vec<String> {
        let (ok, text, lines) = self.output(args, env);
        assert!(ok, "{args:?} {env:?}\n{text}");
        lines
    }
    fn check_all_paths(&self, file: &str, expected: &[&str]) {
        assert_eq!(self.run(&["--no-cache", "-s", "top", file], &[]), expected);
        assert_eq!(
            self.run(&["--no-cache", "-s", "top", file], &[("XEZIM_COMPILE_METHODS", "0")]),
            expected
        );
        for _ in 0..2 {
            assert_eq!(
                self.run(&["--cache-dir", "cache", "-s", "top", file], &[]),
                expected
            );
        }
        let compile = ["--compile", "--no-cache", "-s", "top", "-o", "design.xez", file];
        assert!(self.run(&compile, &[]).is_empty());
        assert!(Path::new(&self.0.join("design.xez")).is_file());
        assert_eq!(self.run(&["--simulate", "design.xez"], &[]), expected);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn unit_storage_and_parameter_shared_bodies_keep_per_instance_values() {
    Fixture::new(&[("lanes.sv", include_str!("unit_storage_shared_bodies_721.sv"))])
        .check_all_paths(
            "lanes.sv",
            &[
                "T|42|41|140|5|40|16",
                "T|14 13 12 11|10 101 12 103",
                "T|27 26 25 24|23",
            ],
        );
}

/// §3.12.1, §23.6: a compilation-unit variable is not declared in an
/// instance's scope, so `inst.v` names nothing there (the reference
/// simulator rejects it too).
#[test]
fn unit_variable_is_not_a_member_of_an_instance() {
    let fixture = Fixture::new(&[(
        "hier.sv",
        "int g = 1;\nmodule c; endmodule\nmodule top; c uc(); initial $display(\"T|%0d\", uc.g); endmodule\n",
    )]);
    let (ok, text, _) = fixture.output(&["--no-cache", "-s", "top", "hier.sv"], &[]);
    assert!(!ok, "{text}");
    assert!(text.contains("'g' is not declared in 'top.uc'"), "{text}");
}
