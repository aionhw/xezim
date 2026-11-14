//! §5.3, §20.10: preprocessing warnings preserve exit status with closed stderr.

use std::process::{Command, Stdio};

#[test]
fn preprocessor_warnings_preserve_success_and_failure_status() {
    let dir = std::env::temp_dir().join(format!("preprocessor_pipe_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, body, code) in [
        ("success", "$display(\"PASS\"); $finish;", 0),
        ("failure", "$fatal(1,\"expected\");", 1),
    ] {
        let source = dir.join(format!("{name}.sv"));
        std::fs::write(
            &source,
            format!("module top;\0 initial begin {body} end endmodule"),
        )
        .unwrap();
        let run = || {
            let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
            cmd.args(["--no-cache", "--relax-nul-bytes"]).arg(&source);
            cmd
        };
        let normal = run().output().unwrap();
        assert_eq!(normal.status.code(), Some(code));
        assert!(String::from_utf8_lossy(&normal.stderr).contains("NUL byte"));
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let closed = run().stderr(Stdio::from(writer)).output().unwrap();
        assert_eq!(
            closed.status.code(),
            Some(code),
            "{name}: {:?}",
            closed.status
        );
        assert_eq!(closed.stdout, normal.stdout);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
