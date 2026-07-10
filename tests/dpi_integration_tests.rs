use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn unique_so_path(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{}_{}_{}.so", stem, std::process::id(), nanos))
}

fn compile_dpi_lib(c_file: &str, stem: &str) -> PathBuf {
    let c_path = manifest_path(c_file);
    let so_path = unique_so_path(stem);
    // Include the xezim/include dir for vpi_user.h, svdpi.h,
    // sv_vpi_user.h, veriuser.h. The headers live in their own
    // subdirectory so users don't accidentally pick up unrelated
    // xezim source files via the include search path.
    let include_dir = manifest_path("include");
    let status = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-I")
        .arg(&include_dir)
        .arg(&c_path)
        .arg("-o")
        .arg(&so_path)
        .status()
        .expect("failed to launch cc");
    assert!(status.success(), "cc failed for {}", c_path.display());
    so_path
}

fn run_xezim_with_dpi(so_path: &Path, sv_file: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_xezim");
    let sv_path = manifest_path(sv_file);
    let out = Command::new(bin)
        .arg("--dpi-lib")
        .arg(so_path)
        .arg("--max-time")
        .arg("1000")
        .arg(&sv_path)
        .output()
        .expect("failed to run xezim");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "xezim failed for {}:\n{}",
        sv_path.display(),
        text
    );
    text
}

/// Like `run_xezim_with_dpi`, but kills the run after `secs` and fails.
///
/// `vpi_handle_by_name` used to spin forever on any dotted name that did
/// not resolve on the first try. A regression of that bug HANGS rather
/// than failing an assertion, which would wedge the test run instead of
/// reporting. Anything exercising VPI name resolution goes through here.
fn run_xezim_with_dpi_timeout(so_path: &Path, sv_file: &str, secs: u64) -> String {
    use std::io::Read;
    let bin = env!("CARGO_BIN_EXE_xezim");
    let sv_path = manifest_path(sv_file);
    let mut child = Command::new(bin)
        .arg("--dpi-lib")
        .arg(so_path)
        .arg("--max-time")
        .arg("1000")
        .arg(&sv_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn xezim");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait().expect("try_wait failed") {
            Some(status) => {
                let mut text = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut text);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut text);
                }
                assert!(status.success(), "xezim failed for {}:\n{}", sv_path.display(), text);
                return text;
            }
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "xezim did not finish within {}s for {} — a VPI hang regression?",
                    secs,
                    sv_path.display()
                );
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
}

/// Run a VPI module (`--vpi-lib`) rather than a DPI one, under a timeout.
fn run_xezim_with_vpi_timeout(so_path: &Path, sv_file: &str, secs: u64) -> String {
    use std::io::Read;
    let bin = env!("CARGO_BIN_EXE_xezim");
    let sv_path = manifest_path(sv_file);
    let mut child = Command::new(bin)
        .arg("--vpi-lib")
        .arg(so_path)
        .arg("--max-time")
        .arg("1000")
        .arg(&sv_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn xezim");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait().expect("try_wait failed") {
            Some(status) => {
                let mut text = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut text);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut text);
                }
                assert!(status.success(), "xezim failed for {}:\n{}", sv_path.display(), text);
                return text;
            }
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("xezim did not finish within {}s for {}", secs, sv_path.display());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
}

fn assert_dpi_pass(c_file: &str, stem: &str, sv_file: &str) {
    let so = compile_dpi_lib(c_file, stem);
    let log = run_xezim_with_dpi(&so, sv_file);
    assert!(
        log.contains("TEST_PASS"),
        "missing TEST_PASS for {}:\n{}",
        sv_file,
        log
    );
}

#[test]
fn dpi_simple_test() {
    assert_dpi_pass(
        "tests/dpi/simple_dpi.c",
        "simple_dpi",
        "tests/dpi/simple_dpi_test.sv",
    );
}

#[test]
fn dpi_extended_test() {
    assert_dpi_pass(
        "tests/dpi/extended_dpi.c",
        "extended_dpi",
        "tests/dpi/extended_dpi_test.sv",
    );
}

#[test]
fn dpi_logic_vec_test() {
    assert_dpi_pass(
        "tests/dpi/logic_vec_dpi.c",
        "logic_vec_dpi",
        "tests/dpi/logic_vec_dpi_test.sv",
    );
}

#[test]
fn dpi_shortreal_string_test() {
    assert_dpi_pass(
        "tests/dpi/shortreal_string_dpi.c",
        "shortreal_string_dpi",
        "tests/dpi/shortreal_string_dpi_test.sv",
    );
}

#[test]
fn dpi_open_array_test() {
    assert_dpi_pass(
        "tests/dpi/open_array_dpi.c",
        "open_array_dpi",
        "tests/dpi/open_array_dpi_test.sv",
    );
}

#[test]
fn dpi_vpi_backdoor_compliance_test() {
    let so = compile_dpi_lib("tests/dpi/vpi_backdoor_compliance.c", "vpi_backdoor_compliance");
    let log = run_xezim_with_dpi(&so, "tests/dpi/vpi_backdoor_compliance.sv");
    // The vpi_backdoor_compliance test outputs "RESULT: PASSED" not "TEST_PASS"
    assert!(
        log.contains("RESULT: PASSED"),
        "missing RESULT: PASSED for vpi_backdoor_compliance:\n{}",
        log
    );
}

#[test]
fn dpi_uvm_test() {
    let so = compile_dpi_lib("tests/dpi/uvm_dpi_test.c", "uvm_dpi_test");
    let log = run_xezim_with_dpi(&so, "tests/dpi/uvm_dpi_test.sv");
    assert!(
        log.contains("RESULT: PASSED"),
        "uvm_dpi_test missing RESULT: PASSED:\n{}",
        log
    );
}

/// IEEE 1800-2017 clause 38 (VPI) conformance. Guards the audit's findings:
/// the `vpi_handle_by_name` infinite loop, `vpi_get_value` silently ignoring
/// every format but two (including the `vpiVectorVal` UVM's HDL backdoor
/// reads with) and never reporting failure, `vpi_put_value` dropping the
/// upper word and the X/Z bits of a vector deposit, and `vpi_get(vpiType)`
/// answering from the current value rather than the declared type.
#[test]
fn vpi_conformance_test() {
    let so = compile_dpi_lib("tests/dpi/vpi_conformance.c", "vpi_conformance");
    // Timeout-guarded: a regression of the name-resolution hang would
    // otherwise wedge the run instead of failing it.
    let log = run_xezim_with_dpi_timeout(&so, "tests/dpi/vpi_conformance.sv", 60);
    assert!(
        log.contains("RESULT: PASSED"),
        "vpi_conformance missing RESULT: PASSED:\n{}",
        log
    );
    // vpi_get_vlog_info used to hand back a hardcoded "0.9.0-uvm".
    let want = format!("VLOG_VERSION: {}", env!("CARGO_PKG_VERSION"));
    assert!(
        log.contains(&want),
        "vpi_get_vlog_info must report the crate version ({}):\n{}",
        env!("CARGO_PKG_VERSION"),
        log
    );
}

/// The classic VPI surface (IEEE 1800-2017 clause 38): a module loaded with
/// `--vpi-lib`, registering a $systf via `vlog_startup_routines`, then
/// walking the flattened design with `vpi_iterate`/`vpi_scan`/`vpi_get_str`.
///
/// Also pins three things the object model got wrong on first contact: an
/// instance name must be a `vpiModule` rather than the 1-bit placeholder
/// signal elaboration invents for it, a parameter must report
/// `vpiParameter` rather than `vpiReg`, and two live `vpi_get_str` results
/// must not alias the same buffer (the idiomatic
/// `vpi_printf("%s %s", get_str(vpiName), get_str(vpiDefName))` needs both).
#[test]
fn vpi_object_model_test() {
    let so = compile_dpi_lib("tests/dpi/vpi_object_model.c", "vpi_object_model");
    let log = run_xezim_with_vpi_timeout(&so, "tests/dpi/vpi_object_model.sv", 60);
    assert!(
        log.contains("OM_ERRORS: 0"),
        "vpi_object_model reported failures:\n{}",
        log
    );
    assert!(
        log.contains("RESULT: PASSED"),
        "vpi_object_model missing RESULT: PASSED:\n{}",
        log
    );
}

/// The rest of IEEE 1800-2017 clause 38: a registered `$systf` reading its own
/// arguments (`vpiSysTfCall` / `vpiArgument`), writing an output argument,
/// system FUNCTIONS returning a value (including `vpiSizedFunc` sizing itself
/// through `sizetf`), `vpi_chk_error`, and `vpi_control`.
///
/// Also pins that a system function inside an expression is invoked exactly
/// once: `infer_width` learned a width by EVALUATING, so every `$systf` in a
/// binary operator ran its calltf twice.
#[test]
fn vpi_systf_test() {
    let so = compile_dpi_lib("tests/dpi/vpi_systf.c", "vpi_systf");
    let log = run_xezim_with_vpi_timeout(&so, "tests/dpi/vpi_systf.sv", 60);
    assert!(
        log.contains("SYSTF_ERRORS: 0"),
        "vpi_systf reported C-side failures:\n{}",
        log
    );
    assert!(
        log.contains("RESULT: PASSED"),
        "vpi_systf missing RESULT: PASSED:\n{}",
        log
    );
    // vpi_control(vpiFinish) must actually end the run.
    assert!(log.contains("BEFORE_FINISH"), "$st_finish never ran:\n{}", log);
    assert!(
        !log.contains("vpi_control(vpiFinish) did not end the run"),
        "vpi_control(vpiFinish) did not end the run:\n{}",
        log
// Regression: a DPI-C import using a typedef name for a packed logic
// vector (the UVM `uvm_hdl_data_t` pattern) must resolve to a
// svLogicVecVal* argument. Before the fix this emitted
// `[DPI] unsupported prototype for '...'` because dpi_atom_kind()
// did not resolve DataType::TypeReference to the underlying
// IntegerVector, so the import was never bound.
#[test]
fn dpi_typedef_vec_test() {
    assert_dpi_pass(
        "tests/dpi/typedef_dpi.c",
        "typedef_dpi",
        "tests/dpi/typedef_dpi_test.sv",
    );
}
