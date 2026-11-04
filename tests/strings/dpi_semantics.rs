//! DPI semantics beyond type mapping (`dpi_integration_tests` covers the
//! argument and return types): imported TASKS that consume time through an
//! exported task, the scope a context import runs in, C -> SV -> C
//! recursion, `pure` imports in continuous logic, the `c_name =` link form,
//! a C-owned object carried as a chandle, one call writing outputs of
//! several types, and the Annex H utility API. Each bench prints `T|` lines;
//! the expected values were worked out by hand from IEEE 1800-2017 clause 35
//! and Annex H, and those of the imported-task and instance-export benches
//! (`dpi_task_*`, `dpi_instance_*`) checked on the reference simulator.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[test]
fn dpi_header_width_helpers_have_defined_c_shifts() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let binary =
        std::env::temp_dir().join(format!("dpi_width_check_{}_{}", std::process::id(), nanos));
    let built = Command::new("cc")
        .args([
            "-fsanitize=undefined",
            "-fno-sanitize-recover=undefined",
            "-I",
        ])
        .arg(manifest_path("include"))
        .arg(manifest_path("tests/dpi/header_width_checks.c"))
        .arg("-o")
        .arg(&binary)
        .status()
        .expect("launch C compiler");
    assert!(built.success(), "compile header checks");
    let out = Command::new(&binary).output().expect("run header checks");
    let _ = std::fs::remove_file(&binary);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// True on the nested-scheduler fallback (`XEZIM_DPI_FIBERS=0`, #290),
/// where imported tasks share one stack: concurrent calls return last-in,
/// first-out and the §35.9 disable protocol needs a stack per call, so the
/// benches that check those skip there (the rest run in both modes).
fn on_one_stack() -> bool {
    let one = std::env::var("XEZIM_DPI_FIBERS").as_deref() == Ok("0");
    if one {
        eprintln!("skipped on the nested-scheduler fallback: needs a stack per call");
    }
    one
}

/// Build `c_file` into a fresh shared library.
fn compile(c_file: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let stem = Path::new(c_file).file_stem().unwrap().to_string_lossy();
    let so = std::env::temp_dir().join(format!("{}_{}_{}.so", stem, std::process::id(), nanos));
    let mut cc = Command::new("cc");
    cc.args(["-shared", "-fPIC"]);
    if cfg!(target_os = "macos") {
        // The exported subroutines resolve when xezim loads the library.
        cc.arg("-Wl,-undefined,dynamic_lookup");
    }
    let status = cc
        .arg("-I")
        .arg(manifest_path("include"))
        .arg(manifest_path(c_file))
        .arg("-o")
        .arg(&so)
        .status()
        .expect("failed to launch cc");
    assert!(status.success(), "cc failed for {c_file}");
    so
}

/// Run `sv_file` against the library: whether it exited 0, and its output.
fn run(so: &Path, sv_file: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--dpi-lib")
        .arg(so)
        .args(["--max-time", "1000"])
        .arg(manifest_path(sv_file))
        .output()
        .expect("failed to run xezim");
    let _ = std::fs::remove_file(so);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// Run `sv_file` against the library and return its `T|` lines.
fn tagged(so: &Path, sv_file: &str) -> Vec<String> {
    let (ok, text) = run(so, sv_file);
    assert!(ok, "xezim failed for {sv_file}:\n{text}");
    text.lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect()
}

/// §26.3 / §35.5.4: identical SV names in separate packages bind independently.
#[test]
fn package_dpi_import_names_are_scoped_676() {
    let so = compile("tests/dpi/scoped_imports_676.c");
    let mut lines = tagged(&so, "tests/dpi/scoped_imports_676.sv");
    lines.sort();
    assert_eq!(lines, [
        "T|design|306", "T|import|105", "T|scoped|101 202",
        "T|tasks|11 22 2", "T|wrapper|103 204",
    ]);
}

/// §23.9: a class property receiver shadows a package with the same name.
#[test]
fn package_dpi_name_does_not_capture_a_class_property_676() {
    let so = compile("tests/dpi/scoped_imports_676.c");
    let mut lines = tagged(&so, "tests/dpi/scoped_imports_shadow_676.sv");
    lines.sort();
    assert_eq!(lines, [
        "T|design|306", "T|import|105", "T|scoped|101 202", "T|shadow|99",
        "T|tasks|11 22 2", "T|wrapper|103 204",
    ]);
}

/// §23.9 / §26.3: a wildcard import does not displace module/class declarations.
#[test]
fn package_dpi_alias_does_not_capture_local_subroutines() {
    let so = compile("tests/dpi/scoped_imports_local_shadow.c");
    let mut lines = tagged(&so, "tests/dpi/scoped_imports_local_shadow.sv");
    lines.sort();
    assert_eq!(
        lines,
        [
            "T|import|12",
            "T|local_task|42 1",
            "T|local|42",
            "T|method|93 99",
            "T|package_task|11",
            "T|package|12 12",
            "T|receiver|99",
            "T|wrapper_task|11",
        ]
    );
}

/// The `T|` lines with the same-time steps of different processes sorted
/// (their order within a time slot is not defined).
fn tagged_sorted(so: &Path, sv_file: &str) -> Vec<String> {
    let mut out = tagged(so, sv_file);
    out.sort();
    out
}

fn sorted(lines: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

/// §35.5.2/§35.6.1: an imported task consumes time through an exported task
/// that delays; its outputs are written when it returns; a second call after
/// the first returned runs from the current time; a task that never waits
/// returns at once. `svGetTime` reads the simulation time from C.
#[test]
fn dpi_imported_task_consumes_time() {
    let so = compile("tests/dpi/dpi_tasks.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_tasks_test.sv"),
        [
            "T|instant d=42 t=0",
            "T|step who=0 step=0 t=5",
            "T|step who=0 step=1 t=10",
            "T|step who=0 step=2 t=15",
            "T|solo started=0 elapsed=15 t=15",
            "T|step who=1 step=0 t=22",
            "T|step who=1 step=1 t=29",
            "T|again started=15 elapsed=14 t=29",
            "T|done",
        ]
    );
}

/// §35.5.2: two processes inside the same imported task at once, each
/// waiting on its own period, interleave and each returns when its own
/// waits are done. Each call runs on its own stack, so the 7-cycle worker
/// does not wait for the 3-cycle one's C frame to return (its steps used to
/// land at 9 and 16).
#[test]
fn dpi_concurrent_imported_tasks_interleave() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_tasks.c");
    let mut out = tagged(&so, "tests/dpi/dpi_tasks_concurrent_test.sv");
    // Steps at the same time come from different processes; compare them
    // without their same-time order.
    out.sort();
    let mut want = vec![
        "T|step who=2 step=0 t=3",
        "T|step who=2 step=1 t=6",
        "T|step who=1 step=0 t=7",
        "T|step who=2 step=2 t=9",
        "T|step who=1 step=1 t=14",
        "T|pair w1 started=0 elapsed=14 w2 started=0 elapsed=9 t=14",
        "T|done",
    ];
    want.sort();
    assert_eq!(out, want);
}

/// §35.5.3/§36.6: a context import runs in the scope of the instance it is
/// called through (`ua.c_fn()`) or from (an `initial` inside the
/// instance) — named as `%m` names it — and a top-level one in `top`;
/// `svGetScopeFromName` and `svGetNameFromScope` round-trip; `svSetScope`
/// returns the scope it replaces and restores it; and a context import
/// recurses through an exported function (C -> SV -> C, 10 levels).
#[test]
fn dpi_context_scope_and_recursion() {
    let so = compile("tests/dpi/dpi_scope_exports.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_scope_exports_test.sv"),
        [
            "T|names a=top.ua b=top.ub",
            "T|top=top",
            "T|lookup top.ua | top.ub | top",
            "T|switch in=top.ub prev=top.ua back=top.ua",
            "T|fact 1=1 5=120 10=3628800",
            "T|self top.ua=top.ua",
            "T|self top.ub=top.ub",
            "T|done",
        ]
    );
}

/// §35.5.3: a function exported from an INSTANTIATED module is called, from
/// a context import, in the instance the import was called through.
#[test]
fn dpi_export_from_instantiated_module() {
    let so = compile("tests/dpi/dpi_scope_exports.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_instance_exports_test.sv"),
        ["T|ids a=11 b=22", "T|done"]
    );
}

/// §35.5.3: three instances (one nested) export the same subroutines under
/// one C name each. A context import reaches its instance's copy, called
/// through the instance or from inside it, svSetScope selects one by path,
/// an imported task in each instance waits through its own exported task,
/// `%m` in the export names the instance, and an export of top is found
/// from an instance's scope (the lookup goes upward).
#[test]
fn dpi_instance_exports_follow_the_scope() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_instance_scopes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_instance_scopes_test.sv"),
        [
            "T|ids a=3 b=5 w=7",
            "T|set a=3 b=5 w=7",
            "T|where top.ua.sv_where t=4",
            "T|ticked top.ua id=3 t=4",
            "T|where top.ub.sv_where t=6",
            "T|ticked top.ub id=5 t=6",
            "T|where top.w.u.sv_where t=8",
            "T|ticked top.w.u id=7 t=8",
            "T|up top.ua=42",
            "T|up top.ub=42",
            "T|up top.w.u=42",
            "T|done",
        ]
    );
}

/// §35.5.2/§35.5.3: imported tasks called through two instance paths at
/// once each wait through their own instance's exported task.
#[test]
fn dpi_imported_tasks_called_through_instances() {
    let so = compile("tests/dpi/dpi_instance_scopes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_instance_task_via_test.sv"),
        [
            "T|where top.ub.sv_where t=2",
            "T|where top.ua.sv_where t=4",
            "T|done t=4",
        ]
    );
}

/// §35.5.3: an export declared only in an instance is not visible from
/// top's scope; the reference simulator stops with a fatal error too.
#[test]
fn dpi_instance_export_outside_its_scope_is_fatal() {
    let so = compile("tests/dpi/dpi_instance_scopes.c");
    let (ok, text) = run(&so, "tests/dpi/dpi_instance_scope_missing_test.sv");
    assert!(!ok, "expected a failing run:\n{text}");
    assert!(
        text.contains("DPI export 'sv_get_id' is not declared in the calling scope 'top'"),
        "{text}"
    );
    assert!(!text.contains("T|done"), "{text}");
}

/// §35.5.2: the C code of an imported task waits through exported tasks
/// that make each kind of wait — an edge, a level `wait`, `fork ... join`,
/// `wait fork`, an output written after a delay — with two callers at once;
/// a call inside a loop and inside a user task; and C -> SV -> C recursion
/// that waits at every level, 41 and 26 levels deep at once.
#[test]
fn dpi_imported_task_wait_kinds_and_recursion() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged_sorted(&so, "tests/dpi/dpi_task_waits_test.sv"),
        sorted(&[
            "T|log who=2 v=105 t=5",
            "T|log who=1 v=105 t=5",
            "T|log who=1 v=225 t=25",
            "T|log who=2 v=225 t=25",
            "T|log who=1 v=330 t=30",
            "T|log who=2 v=330 t=30",
            "T|log who=1 v=434 t=34",
            "T|log who=2 v=434 t=34",
            "T|log who=1 v=577 t=35",
            "T|log who=2 v=577 t=35",
            "T|kinds f1=35 f2=35 t=35",
            "T|log who=20 v=0 t=38",
            "T|log who=10 v=0 t=39",
            "T|log who=20 v=1 t=41",
            "T|log who=10 v=1 t=43",
            "T|log who=20 v=2 t=44",
            "T|wrap who=20 done f=44 t=44",
            "T|log who=11 v=0 t=47",
            "T|log who=11 v=1 t=51",
            "T|loop f3=51 t=51",
            "T|nest lv1=41 lv2=26 t=91",
            "T|done",
        ])
    );
}

/// §35.5.2: two hundred processes inside one imported task at once, each on
/// its own period.
#[test]
fn dpi_many_concurrent_imported_tasks() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_task_many_test.sv"),
        ["T|many sum=2382 max=21 t=21", "T|done"]
    );
}

/// §35.9: a process killed while it waits inside an imported task — by
/// `disable` of its fork block, process::kill() and `disable fork` — unwinds
/// the call by the disable protocol: the exported task returns 1,
/// svIsDisabledState() is 1 (seen = 11), the C code returns 1, and the
/// killing process carries on.
#[test]
fn dpi_imported_task_disable_protocol() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_task_disable_test.sv"),
        [
            "T|log who=1 step=0 t=10",
            "T|log who=1 step=1 t=20",
            "T|fork-block seen=11 t=26",
            "T|log who=2 step=0 t=36",
            "T|log who=2 step=1 t=46",
            "T|kill seen=11 t=52",
            "T|log who=3 step=0 t=62",
            "T|log who=3 step=1 t=72",
            "T|disable-fork seen=11 t=78",
            "T|log who=4 step=0 t=81",
            "T|log who=4 step=1 t=84",
            "T|normal seen=1 t=84",
            "T|done",
        ]
    );
}

/// §35.9, §9.6.2: another process disables a named block around the call:
/// the imported task unwinds by the protocol, the rest of the block is
/// skipped and the process carries on after it.
#[test]
fn dpi_imported_task_disabled_block_unwinds() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_task_disable_block_test.sv"),
        [
            "T|log who=5 step=0 t=10",
            "T|log who=5 step=1 t=20",
            "T|log who=5 step=2 t=30",
            "T|after inner seen=11 t=35",
            "T|done",
        ]
    );
}

/// §35.9: the waiting exported task's own code kills the calling process
/// (process::self().kill()): the export returns 1, the C code acknowledges
/// and returns 1, and the process does not carry on.
#[test]
fn dpi_imported_task_process_killed_from_its_export() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_task_self_kill_test.sv"),
        [
            "T|log who=6 step=0 t=10",
            "T|self-kill t=20",
            "T|seen=11 t=30",
            "T|done",
        ]
    );
}

/// §35.9: `disable` of the exported task the C code waits in ends that
/// export with 0; the imported task is not disabled and carries on.
#[test]
fn dpi_disabled_export_returns_zero() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_task_disable_export_test.sv"),
        [
            "T|log who=7 step=0 t=10",
            "T|log who=7 step=1 t=15",
            "T|log who=7 step=2 t=25",
            "T|returned seen=1 t=25",
            "T|done",
        ]
    );
}

/// §35.9 b) and d): a disabled imported task that returns 0, or that calls
/// another export, is a fatal error, as in the reference simulator.
#[test]
fn dpi_disable_protocol_violations_are_fatal() {
    if on_one_stack() {
        return;
    }
    for (bench, what) in [
        (
            "tests/dpi/dpi_task_bad_return_test.sv",
            "imported task 'c_bad_return' was disabled but did not return 1",
        ),
        (
            "tests/dpi/dpi_task_bad_export_test.sv",
            "an exported subroutine was called after its imported caller was disabled",
        ),
    ] {
        let so = compile("tests/dpi/dpi_task_shapes.c");
        let (ok, text) = run(&so, bench);
        assert!(!ok, "{bench}: expected a failing run:\n{text}");
        assert!(text.contains(what), "{bench}:\n{text}");
        assert!(!text.contains("T|log who=99"), "{bench}:\n{text}");
        assert!(!text.contains("T|done"), "{bench}:\n{text}");
    }
}

/// §20.2: `$finish` while two processes wait inside imported tasks ends the
/// run at once and cleanly; the suspended calls are left.
#[test]
fn dpi_finish_while_imported_tasks_wait() {
    if on_one_stack() {
        return;
    }
    let so = compile("tests/dpi/dpi_task_shapes.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_task_finish_test.sv"),
        [
            "T|log who=2 step=0 t=7",
            "T|log who=1 step=0 t=10",
            "T|log who=2 step=1 t=14",
            "T|log who=1 step=1 t=20",
            "T|log who=2 step=2 t=21",
            "T|finish t=25",
        ]
    );
}

/// §35.5.1-35.5.4: a `pure` import in a continuous assignment and an
/// always_comb block follows its input; two SV names linked to one C symbol
/// with `c_name = function ...`; a C-owned counter held as a chandle by a
/// class; and one call writing real, string, bit-vector, logic-vector (with
/// an X bit) and bit outputs.
#[test]
fn dpi_pure_alias_chandle_and_outputs() {
    let so = compile("tests/dpi/dpi_pure_alias.c");
    assert_eq!(
        tagged(&so, "tests/dpi/dpi_pure_alias_test.sv"),
        [
            "T|pure v=0 wire=0 comb=1",
            "T|pure v=7 wire=1 comb=1",
            "T|pure v=6 wire=0 comb=1",
            "T|pure called=1",
            "T|alias 5 42",
            "T|chandle a=2 b=20",
            "T|released 1",
            "T|outputs half=18.5 label=x=37 byte=37 nib=0101 odd=1",
            "T|outputs half=10.5 label=x=21 byte=21 nib=010x odd=1",
            "T|done",
        ]
    );
}

/// Annex H from C: the open-array queries (H.12.2) on ascending and
/// descending ranges, a dynamic array, a queue and an empty dynamic array
/// (the range [0:-1], no element pointer); out-of-range element pointers
/// (H.12.3); each canonical element type (char, short, long long, double,
/// float, svBitVecVal and svLogicVecVal words, scalar bit and logic) read
/// in place and written back through inout and output formals
/// (H.12.4-H.12.6); bit and part selects of plain 2- and 4-state vectors
/// (H.10.1); per-scope user data kept apart for two instances (H.9.3); and
/// svGetCallerInfo / the disable protocol (H.9.4, H.9.5).
#[test]
fn dpi_svdpi_annex_h_api() {
    let so = compile("tests/dpi/svdpi_api.c");
    assert_eq!(
        tagged(&so, "tests/dpi/svdpi_api_test.sv"),
        [
            "T|up l=2 r=5 lo=2 hi=5 inc=-1 size=4 dims=1 bytes=16 | 2:20 3:30 4:40 5:50",
            "T|down l=7 r=4 lo=4 hi=7 inc=1 size=4 dims=1 bytes=16 | 7:700 6:600 5:500 4:400",
            "T|dyn l=0 r=2 lo=0 hi=2 inc=-1 size=3 dims=1 bytes=12 | 0:7 1:8 2:9",
            "T|queue l=0 r=1 lo=0 hi=1 inc=-1 size=2 dims=1 bytes=8 | 0:5 1:6",
            "T|none l=0 r=-1 lo=0 hi=-1 inc=-1 size=0 dims=1 bytes=0 | empty ptr=null",
            "T|edges below=null above=null variadic=same raw=low dim2=0",
            "T|sums byte=-4 short=31000 long=1099511627771 real=98.750 shortreal=2.750",
            "T|inout b=2,-4,6,120 r=1.875,-3.750,150.000 dyn=21,24,27",
            "T|bitvec dims=2 p.l=11 p.r=0 p.size=12 | 3:abc 2:123 1:fff",
            "T|logicvec 0:10xz 1:0101 2:zzzz",
            "T|fill 000 111 222",
            "T|mark 1x0z 0101 zzz1",
            "T|flip ones=3 now=01001",
            "T|resolve unknown=2 now=1001",
            "T|part low=0000cdef cross=00000078 top=00000001 bit4=0 bit5=1",
            "T|put 01234567c9abcdef",
            "T|lpart 000c000b",
            "T|lput 111101zx",
            "T|ud put 0 0",
            "T|ud get 11 22",
            "T|ud again 33 22",
            "T|ud null_scope=-1 null_key=-1 unused=null replaced=2",
            "T|misc caller=0 disabled=0",
            "T|done",
        ]
    );
}

/// IEEE 1800-2023 §35.5.6 / Annex H (LRM-audit finding): a packed struct or
/// union formal travels as an svBitVecVal / svLogicVecVal vector in every
/// direction — it mapped to no argument kind, so the import was marked
/// unsupported and each call returned 0 without reaching C. Also an enum
/// formal (its base type), 2-D and 3-D open arrays in and out (§35.5.6.1),
/// and the output / inout formals of exported functions and tasks, which C
/// passes by pointer and which were never written back (§35.5.6, H.8.2).
/// Expected values come from the reference simulator.
#[test]
fn dpi_packed_aggregates_multidim_arrays_and_export_outputs() {
    let so = compile("tests/dpi/packed_aggregate_dpi.c");
    assert_eq!(
        tagged(&so, "tests/dpi/packed_aggregate_dpi_test.sv"),
        [
            "T|ps out=02030405 in=01020304",
            "T|ps inout=11121314",
            "T|ps output=a1b2c3d4",
            "T|pl in=182032",
            "T|pl output=0101xzxz",
            "T|pw output=deadbfef123457",
            "T|pu in=77",
            "T|pu output=0000cafe",
            "T|enum in=9",
            "T|2d 272 152 57",
            "T|3d 16362",
            "T|2d output='{'{1, 2}, '{11, 12}}",
            "T|export output=17",
            "T|export outputs=105291",
            "T|export task output=33",
        ]
    );
}
