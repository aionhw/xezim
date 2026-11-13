//! #1131 / §21.6: native DPI and SV see the same ordered command-file plusargs.

use std::process::{Command, Output};

fn run(command: &mut Command) -> Output {
    let output = command.output().expect("run argv fixture");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn file_plusargs_reach_dpi_in_source_and_artifact_runs() {
    let dir = std::env::temp_dir().join(format!("xezim_runtime_argv_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    let source = dir.join("top.sv");
    let artifact = dir.join("top.xez");
    std::fs::write(
        &source,
        r#"
module top;
  import "DPI-C" function string uvm_dpi_get_next_arg_c(input int init);
  string arg, mode;
  initial begin
    arg = uvm_dpi_get_next_arg_c(1);
    while (arg != "") begin
      $display("DPI|%s", arg);
      arg = uvm_dpi_get_next_arg_c(0);
    end
    if (!$value$plusargs("MODE=%s", mode)) $fatal(1,"missing plusarg");
    $display("VALUE|%s", mode);
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    std::fs::write(dir.join("run.args"), "+MODE=file\n+MARKER\n").unwrap();
    std::fs::write(dir.join("nested/inner.args"), "+MODE=nested\n+MARKER\n").unwrap();
    std::fs::write(dir.join("outer.args"), "-F nested/inner.args\n+TAIL\n").unwrap();
    let bin = env!("CARGO_BIN_EXE_xezim");
    run(Command::new(bin)
        .args(["--compile", "--no-cache", "--error-exit", "-s", "top"])
        .arg(&source)
        .arg("-o")
        .arg(&artifact));
    for (args, expected) in [
        (vec!["+MODE=direct"], vec!["+MODE=direct"]),
        (vec!["-f", "run.args"], vec!["+MODE=file", "+MARKER"]),
        (vec!["-F", "run.args"], vec!["+MODE=file", "+MARKER"]),
        (
            vec!["-f", "run.args", "+MODE=direct"],
            vec!["+MODE=file", "+MARKER", "+MODE=direct"],
        ),
        (
            vec!["+MODE=direct", "-f", "run.args"],
            vec!["+MODE=direct", "+MODE=file", "+MARKER"],
        ),
        (
            vec!["-f", "run.args", "+MODE=file"],
            vec!["+MODE=file", "+MARKER", "+MODE=file"],
        ),
        (
            vec!["-F", "outer.args"],
            vec!["+MODE=nested", "+MARKER", "+TAIL"],
        ),
    ] {
        for input in [&source, &artifact] {
            let output = run(Command::new(bin)
                .current_dir(&dir)
                .args(["--simulate", "--no-cache", "--error-exit"])
                .arg(input)
                .args(&args));
            let stdout = String::from_utf8_lossy(&output.stdout);
            let actual: Vec<_> = stdout
                .lines()
                .filter_map(|line| line.strip_prefix("DPI|"))
                .filter(|arg| arg.starts_with('+'))
                .collect();
            assert_eq!(actual, expected, "{args:?}: {stdout}");
            assert!(
                stdout.contains(&format!(
                    "VALUE|{}",
                    expected[0].strip_prefix("+MODE=").unwrap()
                )),
                "{stdout}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
