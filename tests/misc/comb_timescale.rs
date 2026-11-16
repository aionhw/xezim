//! IEEE 1800-2023 §20.3.1, §20.3.3: combinational time calls use the
//! invoking module, including top (whose name-resolution hint is absent).

fn output(src: &str) -> String {
    xezim::simulate(src, 4_000_000)
        .expect("simulate")
        .output
        .iter()
        .map(|line| line.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn top_comb_time_uses_its_module_unit() {
    let out = output(include_str!("comb_timescale.sv"));
    for line in [
        "SCOPE=top",
        "FORMAT time=0 real=1000000",
        "READ comb=1e-09 star=1e-09 edge=1e-09 initial=1.5e-09 delayed=2e-09 fn=1e-09 child=1e-09",
        "TIME comb=0 star=0",
        "PASS",
    ] {
        assert!(
            out.lines().any(|actual| actual == line),
            "missing {line}: {out}"
        );
    }
}

#[test]
fn top_comb_time_through_resolved_net_uses_its_module_unit() {
    let out = output(include_str!("comb_timescale_nettype.sv"));
    for line in ["t_net=1e-09 t_wire=1e-09", "PASS"] {
        assert!(
            out.lines().any(|actual| actual == line),
            "missing {line}: {out}"
        );
    }
}

#[test]
fn top_comb_explicit_unit_controls_time_calls_and_printtimescale() {
    let out = output(include_str!("comb_timescale_explicit.sv"));
    assert!(
        out.contains("DEFAULT scope=top marker=7 stamp=1 time=1"),
        "{out}"
    );
    // §20.4.2: check both unit lines independently of the printed scope label.
    let units: Vec<_> = out
        .lines()
        .filter_map(|line| line.strip_prefix("Time scale of (")?.split_once(") is "))
        .map(|(_, unit)| unit)
        .collect();
    assert_eq!(units, ["1ns / 1ns", "1ns / 1ns"], "{out}");
}

#[test]
fn top_comb_keeps_matching_top_and_source_units() {
    let src = include_str!("comb_timescale_explicit.sv").replace(
        "timeunit 1ps; timeprecision 1ps;",
        "timeunit 1ns; timeprecision 1ns;",
    );
    let out = output(&src);
    assert!(
        out.contains("DEFAULT scope=top marker=7 stamp=1 time=1"),
        "{out}"
    );
    // §20.4.2: check both unit lines independently of the printed scope label.
    let units: Vec<_> = out
        .lines()
        .filter_map(|line| line.strip_prefix("Time scale of (")?.split_once(") is "))
        .map(|(_, unit)| unit)
        .collect();
    assert_eq!(units, ["1ns / 1ns", "1ns / 1ns"], "{out}");
}

#[test]
fn top_comb_twostate_statement_fallback_uses_its_module_unit() {
    // §20.3.1, §20.3.2: 1ns rounds to zero in the top's 1us unit.
    let out = output(include_str!("comb_timescale_twostate.sv"));
    for line in ["TWOSTATE t=100 st=0 b=5 c=81", "PASS"] {
        assert!(
            out.lines().any(|actual| actual == line),
            "missing {line}: {out}"
        );
    }
}

#[test]
fn top_star_time_survives_a_write_later_in_the_waking_process() {
    let out = output(include_str!("comb_timescale_replay.sv"));
    for line in ["REPLAY stamp=1e-09", "PASS"] {
        assert!(
            out.lines().any(|actual| actual == line),
            "missing {line}: {out}"
        );
    }
}

#[cfg(feature = "jit")]
#[test]
fn native_top_comb_time_uses_its_module_unit() {
    // Isolate the environment from the library tests, and keep all temporary
    // output under the runner's TMPDIR. Exercise the native statement bridge.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    for (fixture, expected) in [
        ("comb_timescale.sv", "TIME comb=0 star=0"),
        ("comb_timescale_nettype.sv", "t_net=1e-09 t_wire=1e-09"),
        ("comb_timescale_replay.sv", "REPLAY stamp=1e-09"),
    ] {
        let dir = std::env::temp_dir().join(format!(
            "xezim_comb_timescale_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/misc")
            .join(fixture);
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .env("XEZIM_JIT", "1")
            .current_dir(&dir)
            .arg("--no-cache")
            .arg(source)
            .output()
            .expect("native simulator");
        std::fs::remove_dir_all(&dir).unwrap();
        let out = String::from_utf8_lossy(&run.stdout);
        assert!(run.status.success(), "{out}");
        assert!(out.lines().any(|line| line == expected), "{out}");
        assert!(out.lines().any(|line| line == "PASS"), "{out}");
    }
}
