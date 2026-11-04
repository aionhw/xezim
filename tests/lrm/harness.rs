//! Shared runner for the IEEE 1800-2023 conformance group.
//!
//! Every fixture prints its observations as `T|<clause>|<text>` lines. A test
//! runs one fixture and compares those lines with the reference simulator's
//! lines for the same file, which are embedded in the test as the expected
//! value. Anything before `T|` on a line (a severity prefix from
//! `$error`/`$info`, for example) is the simulator's own message format and
//! is stripped. Most fixtures run in-process through `xezim::simulate_multi`;
//! fixtures that touch files or load a DPI/VPI library run the command-line
//! binary in a temporary directory.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// How the `T|` lines are compared.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Same lines in the same order.
    Exact,
    /// Same lines in any order: the fixture prints from processes that run in
    /// the same time step, whose relative order IEEE 1800 §4.7 leaves
    /// nondeterministic.
    Multiset,
}

const MAX_TIME: u64 = 100_000_000;

/// The fixture directory, used as the include path.
fn fixture_dir() -> String {
    format!("{}/tests/lrm/sv", env!("CARGO_MANIFEST_DIR"))
}

fn simulate(src: &str, top: &str) -> Result<xezim::compiler::Simulator, String> {
    xezim::simulate_multi(
        &[src.to_string()],
        MAX_TIME,
        (!top.is_empty()).then_some(top),
        &[fixture_dir()],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &[],
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
}

/// The `T|` lines `src` prints when elaborated with `top` as the top module
/// (an empty `top` elaborates every uninstantiated module as a top).
pub fn t_lines(src: &str, top: &str) -> Result<Vec<String>, String> {
    let sim = simulate(src, top)?;
    Ok(extract(sim.output.iter().map(|o| o.message.as_str())))
}

/// Run fixture `name` and require a run-time error whose message contains
/// `needle`; a run that reports no error panics with all of its output.
pub fn check_error(name: &str, src: &str, top: &str, needle: &str) {
    let sim = simulate(src, top).unwrap_or_else(|e| panic!("{name}: simulate failed: {e}"));
    let text = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        sim.error_count > 0,
        "{name}: expected a run-time error, none was reported\n{text}"
    );
    assert!(
        text.contains(needle),
        "{name}: expected an error containing {needle:?}\n{text}"
    );
}

/// Keep the `T|...` tail of every output line that has one.
pub fn extract<'a>(messages: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out = Vec::new();
    for m in messages {
        for line in m.lines() {
            if let Some(i) = line.find("T|") {
                out.push(line[i..].trim_end().to_string());
            }
        }
    }
    out
}

/// Compare `actual` with the reference lines `expected`. Lines that start
/// with one of `skip` are known gaps (each documented at the test) and are
/// dropped from `actual` before the comparison; the expectation already
/// leaves them out.
pub fn compare(name: &str, actual: Vec<String>, order: Order, expected: &[&str], skip: &[&str]) {
    let actual: Vec<String> = actual
        .into_iter()
        .filter(|l| !skip.iter().any(|s| l.starts_with(s)))
        .collect();
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    match order {
        Order::Exact => {
            if actual != expected {
                panic!(
                    "{name}: T| lines differ from the reference\n{}",
                    render_diff(&expected, &actual)
                );
            }
        }
        Order::Multiset => {
            let count = |v: &[String]| {
                let mut m: BTreeMap<String, i64> = BTreeMap::new();
                for l in v {
                    *m.entry(l.clone()).or_default() += 1;
                }
                m
            };
            let (e, a) = (count(&expected), count(&actual));
            if e != a {
                let mut missing = Vec::new();
                let mut extra = Vec::new();
                for (l, n) in &e {
                    let k = a.get(l).copied().unwrap_or(0);
                    if k < *n {
                        missing.push(format!("{l}  (x{})", n - k));
                    }
                }
                for (l, n) in &a {
                    let k = e.get(l).copied().unwrap_or(0);
                    if k < *n {
                        extra.push(format!("{l}  (x{})", n - k));
                    }
                }
                panic!(
                    "{name}: T| line multiset differs from the reference\nmissing:\n  {}\nunexpected:\n  {}",
                    missing.join("\n  "),
                    extra.join("\n  ")
                );
            }
        }
    }
}

fn render_diff(expected: &[String], actual: &[String]) -> String {
    let mut s = String::new();
    let n = expected.len().max(actual.len());
    for i in 0..n {
        let e = expected.get(i).map(String::as_str).unwrap_or("<none>");
        let a = actual.get(i).map(String::as_str).unwrap_or("<none>");
        let mark = if e == a { "  " } else { "!!" };
        s.push_str(&format!("{mark} {i:3} ref: {e}\n{mark}     got: {a}\n"));
    }
    s
}

/// Run fixture `name` (`src`, top module `top`) and compare its `T|` lines
/// with the reference's.
pub fn check(name: &str, src: &str, top: &str, order: Order, expected: &[&str], skip: &[&str]) {
    let actual = t_lines(src, top);
    if let Ok(dir) = std::env::var("XEZIM_LRM_DUMP") {
        let text = match &actual {
            Ok(lines) => lines.join("\n") + "\n",
            Err(e) => format!("SIMERR {e}\n"),
        };
        let _ = std::fs::write(format!("{dir}/{name}.txt"), text);
    }
    let actual = actual.unwrap_or_else(|e| panic!("{name}: simulate failed: {e}"));
    compare(name, actual, order, expected, skip);
}

/// Run `src` through the command-line binary in a fresh temporary
/// directory, as the DPI/VPI tests in `tests/strings/` do. `setup` prepares
/// the directory and returns extra command-line arguments.
fn run_cli(
    name: &str,
    src: &str,
    top: &str,
    setup: impl FnOnce(&Path) -> Vec<String>,
) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("xezim_lrm_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let sv = format!("{name}.sv");
    std::fs::write(dir.join(&sv), src).unwrap();
    let extra = setup(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(&extra)
        .args(["--no-cache", "-s", top])
        .arg(&sv)
        .current_dir(&dir)
        .output()
        .expect("failed to run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    let actual = extract(std::iter::once(text.as_str()));
    if let Ok(d) = std::env::var("XEZIM_LRM_DUMP") {
        let _ = std::fs::write(format!("{d}/{name}.txt"), actual.join("\n") + "\n");
    }
    actual
}

/// As `check`, for a fixture that reads or writes files: it runs through
/// the command-line binary in a fresh temporary directory, so parallel tests
/// never share a file and nothing is written into the source tree, and
/// `$write` pieces join into lines as they do on stdout. `args` are extra
/// command-line arguments (plusargs, `--wave`); each name in `support` is
/// copied into the directory from the fixture directory first.
#[allow(clippy::too_many_arguments)]
pub fn check_files(
    name: &str,
    src: &str,
    top: &str,
    order: Order,
    expected: &[&str],
    skip: &[&str],
    args: &[&str],
    support: &[&str],
) {
    let actual = run_cli(name, src, top, |dir| {
        for f in support {
            std::fs::copy(Path::new(&fixture_dir()).join(f), dir.join(f))
                .expect("copy support file");
        }
        args.iter().map(|a| a.to_string()).collect()
    });
    compare(name, actual, order, expected, skip);
}

/// As `check`, for a DPI or VPI fixture: builds `c_src` into a shared
/// library and runs the command-line binary with it (`lib_flag` is
/// `--dpi-lib` or `--vpi-lib`).
#[allow(clippy::too_many_arguments)]
pub fn check_lib(
    name: &str,
    src: &str,
    c_src: &str,
    lib_flag: &str,
    top: &str,
    order: Order,
    expected: &[&str],
    skip: &[&str],
) {
    let actual = run_cli(name, src, top, |dir| {
        let (c, so) = (dir.join("lib.c"), dir.join("lib.so"));
        std::fs::write(&c, c_src).unwrap();
        let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("include");
        let ok = Command::new("cc")
            .args(["-shared", "-fPIC", "-I"])
            .arg(&include)
            .arg(&c)
            .arg("-o")
            .arg(&so)
            .status()
            .expect("failed to launch cc")
            .success();
        assert!(ok, "{name}: cc failed");
        vec![lib_flag.to_string(), so.display().to_string()]
    });
    compare(name, actual, order, expected, skip);
}
