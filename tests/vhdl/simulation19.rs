//! VHDL-2019 designs simulated end to end: each `designs19/<name>.vhd` runs
//! through `xezim --std=19`, and its TEXTIO output and exit status must equal
//! `designs19/<name>.golden` and `designs19/<name>.exit`, produced by nvc
//! (`designs19/regen.sh`; GHDL 4.1 as packaged has no VHDL-2019 libraries).
//! nvc prints report and assertion messages on stderr in its own format, so
//! xezim's (GHDL-format) report lines are left out of the comparison; the
//! designs write their results with std.textio.

use std::path::PathBuf;
use std::process::Command;

fn designs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl/designs19")
}

/// A line xezim prints that nvc does not print on stdout: a report or
/// assertion message (`file:line:col:@time:(report note): ...`) or a GHDL
/// error line.
fn is_message(l: &str) -> bool {
    l.starts_with("ghdl:error:") || (l.contains(":@") && l.contains(":(") && l.contains("): "))
}

fn check(name: &str) {
    let dir = designs();
    // Run in a directory of its own: a design may create files.
    let run_dir =
        std::env::temp_dir().join(format!("xezim-vhdl19-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&run_dir);
    std::fs::create_dir_all(&run_dir).expect("run directory");
    let src = format!("{}.vhd", name);
    std::fs::copy(dir.join(&src), run_dir.join(&src)).expect("copy design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&run_dir)
        .args(["--no-cache", "--std=19"])
        .arg(&src)
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&run_dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]")
                || is_message(l))
        })
        .collect();
    let golden = std::fs::read_to_string(dir.join(format!("{}.golden", name))).expect("golden");
    let exit: i32 = std::fs::read_to_string(dir.join(format!("{}.exit", name)))
        .expect("exit status")
        .trim()
        .parse()
        .expect("exit status number");
    assert_eq!(
        actual.join("\n").trim_end(),
        golden.trim_end(),
        "{}.vhd differs from nvc\nstdout:\n{}\nstderr:\n{}",
        name,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(exit), "{}.vhd exit status", name);
}

/// A design analysed as VHDL-2019 (IEEE 1076-2019): the 2008 base —
/// processes, signals, waits, records, TEXTIO — under `--std=19`.
#[test]
fn basics19() {
    check("basics19");
}

/// VHDL-2019 §24.2 conditional analysis: `` `if ``/`` `elsif ``/`` `else ``
/// over VHDL_VERSION, TOOL_TYPE and TOOL_NAME; §10.13 `return ... when` with
/// an `unaffected` arm and `return when c`; §6.5.3 generic type classes
/// (`(<>)`, `range <>`) on generic subprograms.
#[test]
fn cond19() {
    check("cond19");
}

/// VHDL-2019 §24.2: `` `warning `` and `` `error `` directives of the
/// analysed branches are reported at their location; an error fails the
/// analysis. Directives of excluded branches are not.
#[test]
fn tool_directives_are_reported() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl"))
        .args(["--no-cache", "--std=19", "errors19/directives.vhd"])
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{}", text);
    assert!(
        text.contains("errors19/directives.vhd:4:1: warning: VHDL: check the configuration"),
        "{}",
        text
    );
    assert!(
        text.contains("errors19/directives.vhd:8:3: error: VHDL: this design needs a synthesis tool"),
        "{}",
        text
    );
    assert!(!text.contains("not taken"), "{}", text);
}

/// VHDL-2019 §6.5.2 mode views: record ports whose elements have their own
/// modes (`view master`, `view master'converse`), a view of a record element,
/// an array of views (`view (master) of bus_array`), a port's elements
/// associated one by one with their own modes (in-port and out-port links of
/// one port, in a loop through two instances), and a signal parameter with
/// a view.
#[test]
fn views19() {
    check("views19");
}

/// A VHDL port with a mode view cannot be connected to a SystemVerilog
/// instance: a located error, not a silently wrong connection.
#[test]
fn view_port_at_the_sv_boundary_is_an_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl"))
        .args([
            "--no-cache",
            "--std=19",
            "errors19/view_boundary.vhd",
            "errors19/view_boundary.sv",
        ])
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{}", text);
    assert!(
        text.contains("view_boundary.vhd:12:9: error: VHDL: port 'm' has a mode view"),
        "{}",
        text
    );
}

/// VHDL-2019 §9.1 conditional expressions (scalars, vectors, records; only
/// the selected value evaluated), sequential block statements (their
/// objects initialised each time the block runs, nested blocks), empty
/// record types, and variable and signal subtypes inferred from their
/// initial values.
#[test]
fn lang19() {
    check("lang19");
}

/// VHDL-2019 §9.1 conditional expressions whose values have different
/// lengths (LRM golden: nvc 1.23.1 fails on them).
#[test]
fn condlen19() {
    check("condlen19");
}

/// VHDL-2019 §5.6.2 private variables of protected types, of a scalar type
/// and of another protected type, and an alias publishing a method of one.
#[test]
fn prot19() {
    check("prot19");
}

/// VHDL-2019 §4.2.1 function return identifiers (`return r of bit_vector`).
#[test]
fn retid19() {
    check("retid19");
}

/// VHDL-2019 §16.2.2 'IMAGE and §5.7 TO_STRING of records and arrays
/// (nested), the subtype attributes 'SUBTYPE, 'ELEMENT, 'BASE,
/// 'DESIGNATED_SUBTYPE and 'INDEX as type marks, and scalar 'VALUE.
#[test]
fn attr19() {
    check("attr19");
}

/// VHDL-2019 §16.2.2 'VALUE of records and arrays (nested, a string literal
/// for an array of character literals), and a failure for text with the
/// wrong number of elements (LRM golden: nvc 1.23.1 fails on these).
#[test]
fn value19() {
    check("value19");
}

/// VHDL-2019 §16.5 STD.ENV: VHDL_VERSION, TOOL_TYPE, FILE_NAME, FILE_LINE,
/// DIR_SEPARATOR, GETENV, the calendar and time conversions of fixed
/// times, the assertion counts and enables, and the directory functions.
#[test]
fn env19() {
    check("env19");
}

/// VHDL-2019 §5.5.2 file state, mode, size, position and seeking (LRM
/// golden for xezim's file format).
#[test]
fn fileapi19() {
    check("fileapi19");
}

/// VHDL-2019 §16.5: TOOL_* name xezim (the values of the conditional
/// analysis identifiers, §24.2) and VHDL_VERSION the revision.
#[test]
fn tool_identity() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl"))
        .args(["--no-cache", "--std=19", "errors19/tool19.vhd"])
        .output()
        .expect("run xezim");
    let text = String::from_utf8_lossy(&out.stdout);
    let expect = format!("[SIMULATION|xezim|xezim||{}|2019]", env!("CARGO_PKG_VERSION"));
    assert!(text.contains(&expect), "{}", text);
}

/// VHDL-2019 STD.REFLECTION: subtype and value mirrors of enumeration,
/// integer, floating, physical, record, array and access types.
#[test]
fn refl19() {
    check("refl19");
}

/// VHDL-2019 STD.REFLECTION: the designated object of an access value
/// mirror (LRM golden: nvc 1.23.1 gives a wrong value).
#[test]
fn reflacc19() {
    check("reflacc19");
}

/// VHDL-2019 §5.4.1: access types of protected types, a protected type
/// completing an incomplete type, method calls through access values (LRM
/// golden: nvc 1.23.1 rejects the incomplete type through a use clause).
#[test]
fn protacc19() {
    check("protacc19");
}
