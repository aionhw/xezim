//! §7.4.5, §7.6, §11.5.1 (#869): an indexed part-select of an unpacked array,
//! `a[b +: w]` or `a[b -: w]`, has `w` elements whatever its base, which may
//! vary at run time. The compile-time array-size check counted it as the
//! range `[b:w]`, `|b - w| + 1` elements: `a[0 +: 6] = b` was rejected
//! ("the arrays have 7 and 6 elements"), some real mismatches were accepted,
//! and others were reported with sizes that are not the real ones. The
//! legal fixture holds 31 statements of the reporter's shapes (constant and
//! variable bases, both directions, rows of two-dimensional arrays, dynamic
//! arrays, arguments, continuous assignments, an initializer); the reference
//! simulator compiles it. Each mismatch below is an error there too.

use std::path::PathBuf;
use std::process::Command;

fn compile(name: &str, src: &str) -> (bool, String) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("unpacked_indexed_slice_size_869");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.sv"));
    std::fs::write(&path, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--compile", "--no-cache", "-s", "t"])
        .arg(&path)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let errors = text.lines().any(|l| l.starts_with("error:"));
    (!errors && out.status.success(), text)
}

const DECLS: &str = "  logic [7:0] a [7:0];
  logic [7:0] b [5:0];
  logic [7:0] b3 [2:0];
  logic [7:0] b5 [4:0];
  logic [7:0] big [255:0];
  logic [7:0] c [3:0][5:0];
  logic [7:0] c2 [1:0][5:0];
  logic [7:0] ca5 [4:0];
  int dyn [];
  int x5 [4:0];
  int i, j;
  task automatic take6(input logic [7:0] x [5:0]);
  endtask
";

#[test]
fn legal_indexed_slices_compile() {
    let (ok, text) = compile("legal", include_str!("unpacked_indexed_slice_size_869.sv"));
    assert!(ok, "rejected:\n{text}");
}

/// (statement, module-level item?, target elements, source elements)
const MISMATCHES: &[(&str, bool, u64, u64)] = &[
    ("b = a[0 +: 5];", false, 6, 5),
    ("a[i +: 6] = b5;", false, 6, 5),
    ("a[0 +: 2] = b3;", false, 2, 3),
    ("a[0 +: 6] = b5;", false, 6, 5),
    ("b3 = a[7 -: 2];", false, 3, 2),
    ("b = a[5:1];", false, 6, 5),
    ("a[j -: 3] = b5;", false, 3, 5),
    ("c[0 +: 3] = c2;", false, 3, 2),
    ("b = big[250 +: 5];", false, 6, 5),
    ("take6(a[0 +: 5]);", false, 6, 5),
    ("assign ca5 = big[58 +: 6];", true, 5, 6),
    ("logic [7:0] init5 [4:0] = big[58 +: 6];", true, 5, 6),
    ("x5 = dyn[0 +: 4];", false, 5, 4),
];

#[test]
fn mismatched_indexed_slices_report_their_real_sizes() {
    for (k, &(stmt, item, x, y)) in MISMATCHES.iter().enumerate() {
        let body = if item {
            format!("  {stmt}\n")
        } else {
            format!("  initial begin\n    {stmt}\n  end\n")
        };
        let src = format!("module t;\n{DECLS}{body}endmodule\n");
        let (ok, text) = compile(&format!("bad{k}"), &src);
        assert!(!ok, "accepted `{stmt}`:\n{text}");
        let want = format!("the arrays have {x} and {y} elements");
        assert!(text.contains(&want), "`{stmt}`: no '{want}' in:\n{text}");
    }
}
