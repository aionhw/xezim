//! IEEE 1800-2017 §22.5.1 / §5.3: the actual-argument list of a macro
//! defined with formals may start on a later line than the macro name, with
//! white space, blank lines and comments between them (#578, reported by
//! git-hevangel). Each such call was rejected as "requires parentheses".
//! A use with no `(` at all is still an error.

use std::path::Path;
use std::process::Command;

fn run(file: &str) -> (bool, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("t")
        .arg("--no-cache")
        .arg(path.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn macro_argument_list_on_a_later_line() {
    let (ok, text) = run("tests/misc/macro_args_next_line_578.sv");
    assert!(ok, "{text}");
    assert!(!text.contains("FAIL"), "{text}");
    assert!(text.contains("ALL PASS"), "{text}");
}

#[test]
fn macro_with_formals_used_without_a_list_is_rejected() {
    let (ok, text) = run("tests/misc/macro_args_next_line_578_guard.sv");
    assert!(!ok, "must not compile:\n{text}");
    assert!(text.contains("requires parentheses"), "{text}");
    assert!(!text.contains("FAIL:"), "{text}");
}
