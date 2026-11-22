//! §7.2, §7.2.1: a read of a packed-struct member below the first level
//! (`hwif_out.FUSE_WR_DONE.done.value`, `decoded_reg_strb.intr_block_rf.err_trig_r`)
//! is one static bit range of the root variable, which elaboration lists in
//! its flattened layout under one dotted path. The compiler split the name
//! only at its last dot, found no container, and sent every such read to the
//! interpreter (`ident_lookup`); a register block reads its field storage and
//! its decoded strobes this way in every flop and read-back mux. The design
//! also reads a signed nested member into a wider target (sign-extended, and
//! zero-extended when added to an unsigned one) and a function-local struct
//! that shadows a module variable of the same name. Expected values are
//! Verilator's (5.046).

use std::path::PathBuf;
use std::process::Command;

#[test]
fn nested_member_reads_compile_and_match() {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/perf/packed_member_path_read.sv");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "pmr_check",
            source.to_str().unwrap(),
            "--no-cache",
        ])
        .env("XEZIM_FALLBACK_SITES", "1")
        .output()
        .expect("run packed member path read workload");

    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "packed member path read workload failed:\n{text}"
    );
    assert!(
        text.contains("a: done=1 cnt=5b ext=00f ext2=3fa lr=a strb=010")
            && text.contains("b: done=1 cnt=37 ext=015 ext2=3f9 lr=9 sg=3ff")
            && text.contains("PACKED_MEMBER_PATH_READ_OK"),
        "packed member path read values did not match:\n{text}"
    );
    assert!(
        !text.contains("reason=ident_lookup"),
        "a nested packed-struct member read fell back to the interpreter:\n{text}"
    );
}
