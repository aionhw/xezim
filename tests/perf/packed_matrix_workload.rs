use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static RUN_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
enum Waveform {
    None,
    Vcd,
    Fst,
    Xtrace,
}

struct RunFiles {
    dir: PathBuf,
    waveform: Option<PathBuf>,
}

impl RunFiles {
    fn new(mode: Waveform) -> Self {
        let id = RUN_ID.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("xezim_packed_matrix_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).expect("create waveform regression directory");
        let waveform = match mode {
            Waveform::None => None,
            Waveform::Vcd => Some(dir.join("matrix.vcd")),
            Waveform::Fst => Some(dir.join("matrix.fst")),
            Waveform::Xtrace => Some(dir.join("matrix.xt")),
        };
        Self { dir, waveform }
    }
}

impl Drop for RunFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn counter(text: &str, prefix: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("missing counter `{prefix}`:\n{text}"))
}

fn combined_text(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn run(mode: Waveform) {
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/perf/packed_matrix_regression.sv");
    let files = RunFiles::new(mode);
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    command.args([
        "--simulate",
        "-s",
        "packed_matrix_regression_top",
        source.to_str().unwrap(),
        "--no-cache",
    ]);

    if let Some(path) = files.waveform.as_deref() {
        match mode {
            Waveform::Vcd => {
                // Source-driven `$dumpvars` needs `--wave`; `--fst`/`--xtrace`
                // below are explicit dump requests and imply it.
                command.arg("--wave");
                command.arg(format!("+PACKED_MATRIX_VCD={}", path.display()));
            }
            Waveform::Fst => {
                command.args(["--fst", path.to_str().unwrap()]);
            }
            Waveform::Xtrace => {
                command.args(["--xtrace", path.to_str().unwrap()]);
            }
            Waveform::None => unreachable!(),
        }
    }

    let output = command
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .unwrap_or_else(|error| panic!("run {mode:?} packed matrix workload: {error}"));
    let text = combined_text(&output);
    assert!(output.status.success(), "{mode:?} workload failed:\n{text}");
    assert!(
        text.contains("REGRESSION_OK cycles=256"),
        "{mode:?} workload did not reach checked completion:\n{text}"
    );
    assert!(
        text.lines().any(|line| line.contains("fallbacks=0")),
        "{mode:?} workload used an AST fallback:\n{text}"
    );
    assert!(
        counter(&text, "[FUSE] packed-loop NBA copies (static sites): ") >= 1,
        "{mode:?} packed NBA copies were not vectorized:\n{text}"
    );

    let fills = counter(&text, "[FUSE] packed blocking fills (dynamic executions): ");
    match mode {
        Waveform::None => assert!(
            fills >= 1,
            "packed blocking fills were not collapsed:\n{text}"
        ),
        _ => assert_eq!(
            fills, 0,
            "{mode:?} must retain element-level waveform changes:\n{text}"
        ),
    }

    if let Some(path) = files.waveform.as_deref() {
        validate_waveform(mode, path);
    }
}

fn validate_waveform(mode: Waveform, path: &Path) {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|error| panic!("read {mode:?} waveform {}: {error}", path.display()));
    assert!(bytes.len() > 128, "{mode:?} waveform is unexpectedly small");
    match mode {
        Waveform::Vcd => {
            let text = String::from_utf8(bytes).expect("VCD waveform must be text");
            assert!(text.contains("$timescale"), "VCD timescale is missing");
            assert!(
                text.contains("$enddefinitions $end"),
                "VCD definitions are incomplete"
            );
            assert!(
                text.lines().any(|line| line.starts_with('#')),
                "VCD has no time records"
            );
        }
        Waveform::Fst => {
            assert!(
                bytes.iter().any(|byte| *byte != 0),
                "FST waveform contains no encoded data"
            );
        }
        Waveform::Xtrace => {
            let text = String::from_utf8(bytes).expect("XTrace waveform must be text");
            assert!(text.starts_with("@xtrace "), "XTrace header is missing");
            for section in ["@section dict", "@section trace", "@section end"] {
                assert!(
                    text.contains(section),
                    "XTrace section is missing: {section}"
                );
            }
            assert!(
                text.contains("\nT,+0\n"),
                "XTrace has no initial checkpoint"
            );
        }
        Waveform::None => unreachable!(),
    }
}

#[test]
fn packed_matrix_workload_uses_fast_paths() {
    run(Waveform::None);
}

#[test]
fn packed_matrix_workload_writes_vcd() {
    run(Waveform::Vcd);
}

#[test]
fn packed_matrix_workload_writes_fst() {
    run(Waveform::Fst);
}

#[test]
fn packed_matrix_workload_writes_xtrace() {
    run(Waveform::Xtrace);
}
