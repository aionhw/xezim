//! §6.21, §9.3.2 (#943): fork children whose block local has the name of
//! their parent's local, while two live frames of another process declare
//! that name too. Each process must see its own declaration's type metadata,
//! so the metadata under the name is swapped when the processes alternate.
//! Every swap used to rescan the whole module's struct-member and packed
//! layout tables (twice per fork-child scheduling), and the displaced entries
//! ping-ponged between the frames, so a design with many struct variables
//! ground to a halt. A swap now moves only the local's own entries: only the
//! first capture of a declaration scans the tables (`local_meta_scans`), so
//! that count does not grow with the number of clock edges. Expected values
//! are the reference simulator's.
use std::process::Command;

fn run(edges: u32) -> (Vec<String>, u64) {
    let dir = std::env::temp_dir().join(format!(
        "xezim_same_name_frames_{}_{edges}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join("design.sv");
    std::fs::write(
        &path,
        include_str!("../issue_cases/same_name_live_frames_943.sv"),
    )
    .expect("write design");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args([
            "--simulate",
            "-s",
            "top",
            path.to_str().unwrap(),
            "--no-cache",
        ])
        .arg(format!("+EDG={edges}"))
        .env("XEZIM_PROFILE_TIMING", "1")
        .output()
        .expect("run xezim");
    let _ = std::fs::remove_dir_all(&dir);
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "simulation failed:\n{text}");
    let lines = text
        .lines()
        .filter(|l| l.starts_with("T|"))
        .map(str::to_string)
        .collect();
    let scans = text
        .lines()
        .find_map(|l| l.split("local_meta_scans=").nth(1))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("missing local_meta_scans:\n{text}"));
    (lines, scans)
}

const WANT: [&str; 12] = [
    "T|poison inner a=7 b=0",
    "T|poison outer a=0 b=3",
    "T|kind 0 hits=40 sum=6",
    "T|kind 1 hits=40 sum=6",
    "T|kind 2 hits=40 sum=6",
    "T|kind 3 hits=40 sum=20",
    "T|kind 4 hits=40 sum=10",
    "T|kind 5 hits=40 sum=6",
    "T|kind 6 hits=40 sum=6",
    "T|kind 7 hits=40 sum=14",
    "T|kind 8 hits=40 sum=10",
    "T|kind 9 hits=40 sum=6",
];

#[test]
fn same_named_locals_in_live_frames_swap_without_rescanning_the_tables() {
    let (lines, scans) = run(10);
    assert_eq!(lines, WANT);
    // Four times the edges: the same per-edge sums, four times the hits, and
    // not one more scan of the tables.
    let (lines4, scans4) = run(40);
    let want4: Vec<String> = WANT
        .iter()
        .map(|l| l.replace("hits=40", "hits=160"))
        .collect();
    assert_eq!(lines4, want4);
    assert_eq!(scans4, scans, "local_meta_scans grew with the clock edges");
    // At most one per (name, declaration) pair: a handful of declarations.
    assert!(scans <= 32, "local_meta_scans={scans}");
}
