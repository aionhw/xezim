//! IEEE 1800-2023 §5.4: the text of a comment is not interpreted. A `-v`/`-y`
//! library file is decoded as source files are: a byte that is not UTF-8 (a
//! Latin-1 `®`, 0xAE, in a comment, as in some vendor simulation libraries)
//! becomes U+FFFD. The read used to fail with "stream did not contain valid
//! UTF-8", the whole file was dropped, and its modules were "not found",
//! while the same byte in a source file was accepted.

use std::process::Command;

#[test]
fn latin1_byte_in_a_library_file_keeps_the_file() {
    let dir = std::env::temp_dir().join(format!("xezim_lib_latin1_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("y")).unwrap();
    let mut head = b"// Copyright ".to_vec();
    head.push(0xAE);
    head.extend_from_slice(b" vendor\n");
    let module = |name: &str, n: u32| {
        let mut text = head.clone();
        text.extend_from_slice(
            format!("module {name}; initial $display(\"{name}={n}\"); endmodule\n").as_bytes(),
        );
        text
    };
    std::fs::write(dir.join("lib.v"), module("sub", 7)).unwrap();
    std::fs::write(dir.join("y").join("ysub.v"), module("ysub", 9)).unwrap();
    std::fs::write(
        dir.join("top.sv"),
        "module top; sub u(); ysub w(); initial #1 $finish; endmodule\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", "top.sv", "-v", "lib.v", "-y", "y", "+libext+.v", "-s", "top"])
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("sub=7") && text.contains("ysub=9"), "{text}");
    assert!(!text.contains("UTF-8"), "{text}");
}
