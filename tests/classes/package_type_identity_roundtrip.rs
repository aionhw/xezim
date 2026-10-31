//! IEEE 1800-2017 §6.22.1 / §8.25: declaration identity survives artifact loading.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "xezim-type-identity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        for (name, source) in files {
            std::fs::write(path.join(name), source).unwrap();
        }
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Vec<String> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command.args(args).arg("--error-exit").current_dir(&self.0);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("XEZIM_") {
                command.env_remove(name);
            }
        }
        let out = command.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{args:?}\n{text}");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.starts_with("T|"))
            .map(str::to_owned)
            .collect()
    }
    fn check_all_paths(&self, files: &[&str], expected: &[&str]) {
        let mut source = vec!["--no-cache", "-s", "top"];
        source.extend_from_slice(files);
        assert_eq!(self.run(&source), expected);
        for _ in 0..2 {
            let mut cached = vec!["--cache-dir", "cache", "-s", "top"];
            cached.extend_from_slice(files);
            assert_eq!(self.run(&cached), expected);
        }
        let mut compile = vec!["--compile", "--no-cache", "-s", "top", "-o", "design.xez"];
        compile.extend_from_slice(files);
        assert!(self.run(&compile).is_empty());
        assert!(Path::new(&self.0.join("design.xez")).is_file());
        assert_eq!(self.run(&["--simulate", "design.xez"]), expected);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn imported_package_types_share_statics_on_cold_cache_and_artifact_paths() {
    Fixture::new(&[("input.sv", include_str!("package_scoped_type_args.sv"))]).check_all_paths(
        &["input.sv"],
        &[
            "T|enum|1",
            "T|enum_val|1",
            "T|struct|1",
            "T|vec|1",
            "T|class|1",
            "T|pclass|1",
            "T|pclass_other|0",
            "T|q_w|0",
        ],
    );
}

#[test]
fn identical_package_class_bodies_keep_distinct_declaration_identity() {
    let source = r#"
package p; class C; endclass class G#(int N=1); endclass endpackage
package q; class C; endclass class G#(int N=1); endclass endpackage
class db#(type T=int);
  static bit hit;
  static function void put();hit=1;endfunction
  static function bit has();return hit;endfunction
endclass
module top; initial begin
  db#(p::C)::put(); db#(p::G#(3))::put();
  $display("T|classes|%0d|%0d",db#(p::C)::has(),db#(q::C)::has());
  $display("T|generic|%0d|%0d|%0d",db#(p::G#(3))::has(),db#(q::G#(3))::has(),db#(p::G#(4))::has());
end endmodule
"#;
    Fixture::new(&[("input.sv", source)])
        .check_all_paths(&["input.sv"], &["T|classes|1|0", "T|generic|1|0|0"]);
}

#[test]
fn separate_enum_declarations_with_identical_file_offsets_do_not_share_statics() {
    // The enum AST, including byte spans, is identical in the two files.
    // They remain distinct nominal types; only real imported aliases match.
    let package = "package p; typedef enum int { A=0,B=1 } E; endpackage\n";
    let local = "           typedef enum int { A=0,B=1 } E;\n";
    assert_eq!(package.find("enum"), local.find("enum"));
    let top = format!(
        r#"{local}
class db#(type T=int);
 static bit hit;
 static function void put();hit=1;endfunction
 static function bit has();return hit;endfunction
endclass
module top; initial begin
 db#(p::E)::put();
 $display("T|nominal|%0d|%0d",db#(p::E)::has(),db#(E)::has());
end endmodule
"#
    );
    Fixture::new(&[("p.sv", package), ("local.sv", &top)])
        .check_all_paths(&["p.sv", "local.sv"], &["T|nominal|1|0"]);
}
