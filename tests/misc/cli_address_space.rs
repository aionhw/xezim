//! CLI resource handling; IEEE 1800-2023 §13.4 (automatic function recursion),
//! §20.17.1 ($system), §21.6 ($value$plusargs), §21.2.1 ($display) and §20.2 ($finish).
#![cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn limited(args: &[&str], stack: Option<&str>, limit_kib: u64) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    command.args(args).env_remove("XEZIM_STACK_MB");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("MIMALLOC_") {
            command.env_remove(key);
        }
    }
    command
        .env_remove("MALLOC_ARENA_MAX")
        .env_remove("XEZIM_NO_MEM_WATCHDOG");
    if let Some(stack) = stack {
        command.env("XEZIM_STACK_MB", stack);
    }
    // Only the child receives the limit; pre_exec calls only async-signal-safe
    // resource-limit operations, leaving the test runner unrestricted.
    unsafe {
        command.pre_exec(move || {
            let mut limit = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_AS, &mut limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            limit.rlim_cur = (limit_kib * 1024) as libc::rlim_t;
            if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.output().expect("run limited xezim")
}

#[test]
fn informational_commands_do_not_spawn_a_worker() {
    for stack in [None, Some("1536")] {
        for (flag, expected) in [
            ("-V", "xezim version"),
            ("-h", "Usage: xezim"),
            ("--help", "Usage: xezim"),
            ("--show-env-avail", "XEZIM_STACK_MB"),
        ] {
            let out = limited(&[flag], stack, 2097152);
            let text = String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr);
            assert!(
                out.status.success() && text.contains(expected),
                "{flag}: {text}"
            );
            assert!(!text.contains("cannot spawn"), "{flag}: {text}");
        }
    }
    let out = limited(&[], Some("1536"), 2097152);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("Usage: xezim") && !stderr.contains("cannot spawn"));
}

#[test]
fn simulation_retries_smaller_workers_and_preserves_child_environment() {
    let dir = std::env::temp_dir().join(format!("xezim_address_space_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("top.sv");
    // §20.17.1: the shell inherits the environment, including whether the
    // variable is set; the simulator must not replace the user's stack choice.
    std::fs::write(
        &src,
        r#"module top;
  int rc;
  initial begin
    rc = $system("echo CHILD_STACK=${XEZIM_STACK_MB-unset}");
    if (rc != 0) $fatal(1, "child failed");
    $display("STACK_PASS");
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    // The allocator reserves a 1 GiB arena before the worker starts, so at 2 GiB
    // the default 1024 MiB stack (and 1536 MiB) cannot fit and a retry runs.
    for stack in [None, Some("256"), Some("1536"), Some("0")] {
        let out = limited(&["--no-cache", src.to_str().unwrap()], stack, 2097152);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.contains("STACK_PASS"),
            "stack={stack:?}, {:?}: {stdout} {stderr}",
            out.status
        );
        assert!(
            stdout
                .lines()
                .any(|line| line == format!("CHILD_STACK={}", stack.unwrap_or("unset"))),
            "{stdout}"
        );
        assert!(!stderr.contains("panicked"), "{stderr}");
        if stack.is_none() || stack == Some("1536") {
            assert_eq!(
                stderr
                    .matches("Warning: cannot spawn the simulation thread")
                    .count(),
                1,
                "{stderr}"
            );
            assert!(
                stderr.contains("running with a 512 MiB stack")
                    && stderr.contains("set XEZIM_STACK_MB to a size that fits"),
                "{stderr}"
            );
        } else {
            assert!(!stderr.contains("cannot spawn"), "{stderr}");
        }
    }
    // §13.4: an automatic function has a distinct activation at every call.
    // Depth 800 overflows an 8 MiB main stack in the measured debug build;
    // it must run on the smaller worker after the requested stack fails.
    std::fs::write(
        &src,
        r#"module top;
  function automatic int recurse(int n);
    if (n == 0) return 0;
    return 1 + recurse(n - 1);
  endfunction
  int depth;
  initial begin
    if (!$value$plusargs("D=%d", depth)) depth = 800;
    $display("DEPTH=%0d", recurse(depth));
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    for stack in [None, Some("1536")] {
        let out = limited(
            &["--no-cache", src.to_str().unwrap(), "+D=800"],
            stack,
            2097152,
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success()
                && stdout.contains("DEPTH=800")
                && stderr.contains("running with a 512 MiB stack"),
            "stack={stack:?}, {:?}: {stdout} {stderr}",
            out.status
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
