//! #1055: the command line runs the simulation on the large-stack worker it
//! already has. It used to start a second worker of the same size for
//! `simulate_multi`, so the default pair of 1 GiB stacks took 2 GiB of
//! address space before the design allocated anything, and a run under an
//! address-space limit failed on an allocation it had room for.

/// A one-line design under a 2.5 GiB address-space limit: enough for one
/// default stack and the allocator's reservations, not for two stacks.
#[cfg(target_os = "linux")]
#[test]
fn simulation_fits_one_default_stack_under_an_address_space_limit() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("xezim_stack_1055_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    std::fs::write(
        &src,
        "module top; initial begin $display(\"STACK_PASS\"); $finish; end endmodule\n",
    )
    .unwrap();
    let out = Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 2500000 && exec \"$0\" --no-cache \"$1\"")
        .arg(env!("CARGO_BIN_EXE_xezim"))
        .arg(&src)
        .env_remove("XEZIM_STACK_MB")
        .output()
        .expect("run xezim");
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success() && text.contains("STACK_PASS"),
        "{:?}: {text}",
        out.status
    );
}
