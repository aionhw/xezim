//! FST (GTKWave binary) dump conformance, checked by DECODING the dump.
//!
//! Until this group existed, the entire suite's FST validation was one line in
//! `tests/perf/packed_matrix_workload.rs`:
//!
//! ```ignore
//! assert!(bytes.iter().any(|byte| *byte != 0), "FST waveform contains no encoded data");
//! ```
//!
//! — against 27 VCD compliance tests (`gates/vcd_lrm_compliance.rs`) and 16
//! XTrace conformance tests (`scheduling/xtrace_conformance.rs`). Two defects
//! that both of those formats had already fixed therefore lived in the FST
//! writer unnoticed:
//!
//!   * `fst_finish` closed the sink without re-running change detection, so a
//!     value written after the last `dump_write_changes` — anything assigned in
//!     a `final` block — was in the VCD and XTrace dumps and missing from the FST.
//!   * no closing time record was emitted, so the trailer's `end_time` was the
//!     last VALUE CHANGE. A run to t=200 whose last toggle was at t=20 produced
//!     a dump ending at 20, and a quiet tail was indistinguishable from a
//!     truncated file.
//!
//! Assertions here are on the DECODED dump (`fst-reader`, the reader half of
//! the `fst-writer` the simulator writes with), not on the bytes: an FST is
//! block-compressed, so byte-level checks cannot see a var's type, a signal's
//! width, an alias, or a value. Decoding is also what a viewer does, which is
//! the behaviour worth pinning.
//!
//! Declaration metadata, real values, and event pulses are decoded here as
//! active regression checks alongside the end-of-run cases.

use std::collections::HashMap;
use std::io::BufReader;
use std::path::PathBuf;

use fst_reader::{
    FstFilter, FstHierarchyEntry, FstReader, FstScopeType, FstSignalValue, FstVarType,
};

// ── harness ────────────────────────────────────────────────────────────

/// One decoded FST var.
#[derive(Debug, Clone)]
struct Var {
    /// Dotted path below (and including) the top scope: `top.u_sub.mid`.
    path: String,
    /// The declared name as written, which is where FST carries a bit range
    /// (`hi [15:8]`) — GTKWave's convention, and what Verilator emits.
    name: String,
    tpe: FstVarType,
    length: u32,
    handle: usize,
    is_alias: bool,
}

/// A decoded FST: hierarchy, per-signal value timeline, and trailer times.
struct Fst {
    vars: Vec<Var>,
    /// handle → [(time, value)], in file order, no-op repeats kept.
    changes: HashMap<usize, Vec<(u64, String)>>,
    start_time: u64,
    end_time: u64,
    timescale_exponent: i8,
}

impl Fst {
    fn var(&self, path: &str) -> &Var {
        self.vars
            .iter()
            .find(|v| v.path == path)
            .unwrap_or_else(|| panic!("no FST var `{}`; have {:?}", path, self.paths()))
    }

    fn paths(&self) -> Vec<&str> {
        self.vars.iter().map(|v| v.path.as_str()).collect()
    }

    fn has(&self, path: &str) -> bool {
        self.vars.iter().any(|v| v.path == path)
    }

    /// The value timeline of `path`, with consecutive duplicates collapsed —
    /// what a viewer draws.
    fn timeline(&self, path: &str) -> Vec<(u64, String)> {
        let h = self.var(path).handle;
        let raw = self.changes.get(&h).cloned().unwrap_or_default();
        let mut out: Vec<(u64, String)> = Vec::new();
        for (t, v) in raw {
            if out.last().map(|(_, p)| p.as_str()) != Some(v.as_str()) {
                out.push((t, v));
            }
        }
        out
    }
}

/// Run `src` through the simulator with an FST dump, then decode it.
///
/// Drives `simulate_multi`'s `fst_file` — the same field `--fst` sets — so the
/// dump opens in `run()` before time 0, which is the path users take. (The
/// `$fsdbDumpvars` entry point reaches the same `fst_start_dump`, just later.)
fn dump(tag: &str, src: &str) -> Fst {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("xezim_fst_rt_{}_{}.fst", tag, std::process::id()));
    let _ = std::fs::remove_file(&path);

    let _sim = xezim::simulate_multi(
        &[src.to_string()],
        1_000_000,
        None,
        &[],
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
        Some(path.to_str().unwrap()),
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");

    let decoded = decode(&path);
    let _ = std::fs::remove_file(&path);
    decoded
}

fn decode(path: &PathBuf) -> Fst {
    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("no FST written to {}: {}", path.display(), e));
    let mut reader = FstReader::open_and_read_time_table(BufReader::new(file))
        .unwrap_or_else(|e| panic!("FST at {} does not decode: {:?}", path.display(), e));

    let header = reader.get_header();

    let mut vars: Vec<Var> = Vec::new();
    let mut scopes: Vec<String> = Vec::new();
    reader
        .read_hierarchy(|entry| match entry {
            FstHierarchyEntry::Scope { name, tpe, .. } => {
                assert_eq!(
                    tpe,
                    FstScopeType::Module,
                    "scope `{}` is not a module",
                    name
                );
                scopes.push(name);
            }
            FstHierarchyEntry::UpScope => {
                scopes.pop();
            }
            FstHierarchyEntry::Var {
                tpe,
                name,
                length,
                handle,
                is_alias,
                ..
            } => {
                // FST carries a bit range inside the var name (`hi [15:8]`);
                // the hierarchical path is keyed on the bare reference.
                let leaf = name.split_whitespace().next().unwrap_or(&name).to_string();
                let mut path = scopes.join(".");
                path.push('.');
                path.push_str(&leaf);
                vars.push(Var {
                    path,
                    name,
                    tpe,
                    length,
                    handle: handle.get_index(),
                    is_alias,
                });
            }
            _ => {}
        })
        .expect("FST hierarchy does not decode");

    let mut changes: HashMap<usize, Vec<(u64, String)>> = HashMap::new();
    reader
        .read_signals(&FstFilter::all(), |time, handle, value| {
            let rendered = match value {
                FstSignalValue::String(bytes) => String::from_utf8_lossy(bytes).to_string(),
                FstSignalValue::Real(f) => format!("r{}", f),
            };
            changes
                .entry(handle.get_index())
                .or_default()
                .push((time, rendered));
            Ok::<(), ()>(())
        })
        .expect("FST value changes do not decode");

    Fst {
        vars,
        changes,
        start_time: header.start_time,
        end_time: header.end_time,
        timescale_exponent: header.timescale_exponent,
    }
}

// ── designs ────────────────────────────────────────────────────────────

/// A quiet tail: the last value change is at t=20, the run ends at t=200.
const QUIET_TAIL: &str = r#"
`timescale 1ns/1ns
module top;
  logic [7:0] q = 0;
  initial begin
    #10 q = 8'h11;
    #10 q = 8'h22;
    #180 $finish;
  end
endmodule
"#;

/// A `final` block writes after the event loop has drained.
const FINAL_WRITE: &str = r#"
`timescale 1ns/1ns
module top;
  logic [7:0] q = 0;
  initial begin
    #10 q = 8'h11;
    #10 $finish;
  end
  final begin
    q = 8'hEE;
  end
endmodule
"#;

/// Declaration shapes: types, bit ranges, reals, events, arrays, hierarchy.
const KITCHEN: &str = r#"
`timescale 1ns/1ns
module sub(input logic [7:0] din, output logic [7:0] dout);
  logic [7:0] mid;
  always_comb mid = din + 8'd1;
  assign dout = mid;
endmodule
module top;
  logic        clk = 0;
  logic [15:8] hi;
  logic [0:7]  asc;
  real         r;
  integer      iv;
  event        ev;
  logic [7:0]  bus;
  logic [7:0]  obus;
  logic [3:0]  mem [0:3];
  logic [95:0] vwide;
  logic [7:0]  xz;

  sub u_sub(.din(bus), .dout(obus));

  always #5 clk = ~clk;

  initial begin
    hi = 8'hA5; asc = 8'h3C; r = 0.0; iv = 0; bus = 8'h10;
    vwide = 96'h0; xz = 8'b1010_xz01;
    mem[0] = 0; mem[1] = 1; mem[2] = 2; mem[3] = 3;
  end

  always @(posedge clk) begin
    bus   <= bus + 8'h11;
    r     <= r + 1.25;
    iv    <= iv + 7;
    hi    <= hi ^ 8'h0F;
    vwide <= vwide + 96'h1_0000_0000_0000_0000;
    mem[1] <= mem[1] + 1;
    ->ev;
  end

  initial #32 $finish;
endmodule
"#;

// ── the two defects this group was written for ─────────────────────────

/// The trailer's `end_time` is the run's end, not the last transition. Without
/// a closing time record a viewer stops drawing at the last toggle, so a quiet
/// tail reads exactly like a truncated file.
#[test]
fn dump_closes_at_the_final_simulation_time() {
    let fst = dump("quiet_tail", QUIET_TAIL);
    assert_eq!(
        fst.end_time, 200,
        "FST end_time is the last VALUE CHANGE, not the end of the run; \
         the last 180 ns of the simulation are missing from the dump"
    );
    assert_eq!(fst.start_time, 0);
    // The tail is quiet, so the value itself must still be the t=20 one.
    assert_eq!(
        fst.timeline("top.q").last().map(|(_, v)| v.as_str()),
        Some("00100010")
    );
}

/// A `final` block runs after the event loop drains and before the dump is
/// closed. `vcd_finish` and `xtrace_finish` both re-run change detection there;
/// `fst_finish` did not, so the write was in two dumps and absent from the third.
#[test]
fn final_region_writes_reach_the_dump() {
    let fst = dump("final_write", FINAL_WRITE);
    let tl = fst.timeline("top.q");
    assert_eq!(
        tl.last().map(|(t, v)| (*t, v.as_str())),
        Some((20, "11101110")),
        "the `final` block's write is missing from the FST; timeline was {:?}",
        tl
    );
}

// ── what the writer already gets right ─────────────────────────────────

#[test]
fn hierarchy_nests_below_the_top_module() {
    let fst = dump("hier", KITCHEN);
    for p in ["top.clk", "top.bus", "top.u_sub.mid", "top.u_sub.din"] {
        assert!(fst.has(p), "missing `{}`; have {:?}", p, fst.paths());
    }
}

/// A port bound to a whole net is ONE net: the formal reuses the actual's
/// signal id and is declared an FST alias, so the net emits one change record
/// rather than one per hierarchical name.
#[test]
fn port_connected_nets_are_declared_as_aliases() {
    let fst = dump("alias", KITCHEN);
    let bus = fst.var("top.bus");
    let din = fst.var("top.u_sub.din");
    assert_eq!(
        bus.handle, din.handle,
        "`bus` and `u_sub.din` are one net and must share a signal handle"
    );
    assert!(!bus.is_alias, "the actual is the canonical signal");
    assert!(din.is_alias, "the formal must be declared an alias");
}

#[test]
fn unpacked_array_elements_are_dumped_individually() {
    let fst = dump("mem", KITCHEN);
    for i in 0..4 {
        let p = format!("top.mem[{}]", i);
        assert!(fst.has(&p), "missing `{}`; have {:?}", p, fst.paths());
        assert_eq!(fst.var(&p).length, 4);
    }
    // Only mem[1] is written, and it counts up once per posedge.
    let tl = fst.timeline("top.mem[1]");
    assert!(
        tl.len() >= 3,
        "mem[1] should change every posedge, got {:?}",
        tl
    );
    assert_eq!(fst.timeline("top.mem[2]").len(), 1, "mem[2] never changes");
}

#[test]
fn x_and_z_values_survive_the_round_trip() {
    let fst = dump("xz", KITCHEN);
    assert_eq!(
        fst.timeline("top.xz").first().map(|(_, v)| v.as_str()),
        Some("1010xz01"),
        "x/z bits must round-trip exactly, not collapse to 0/1"
    );
}

#[test]
fn wide_vectors_round_trip_at_full_width() {
    let fst = dump("wide", KITCHEN);
    let v = fst.var("top.vwide");
    assert_eq!(v.length, 96);
    let tl = fst.timeline("top.vwide");
    for (_, val) in &tl {
        assert_eq!(
            val.len(),
            96,
            "a 96-bit net must emit 96 characters: {}",
            val
        );
    }
    assert_eq!(tl[0].1, "0".repeat(96));
    // +1<<64 per posedge: bit 64 set, counting up in the high 32 bits.
    assert_eq!(&tl[1].1[..32], &format!("{:032b}", 1u32));
}

#[test]
fn the_header_carries_the_designs_timescale() {
    let fst = dump("ts", KITCHEN);
    assert_eq!(
        fst.timescale_exponent, -9,
        "`timescale 1ns/1ns` must land as exponent -9"
    );
}

/// A purely behavioral design marks nothing in the lazily-synced `self.signals`
/// mirror, which is what made an earlier version of the enumeration dump
/// nothing at all.
#[test]
fn a_purely_behavioral_design_is_not_dumped_empty() {
    let fst = dump("behavioral", QUIET_TAIL);
    assert!(!fst.vars.is_empty(), "no vars were declared");
    assert!(
        fst.timeline("top.q").len() >= 3,
        "q's transitions are missing: {:?}",
        fst.timeline("top.q")
    );
}

#[test]
fn scope_filter_restricts_the_dump_to_the_named_subtree() {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("xezim_fst_rt_scope_{}.fst", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let _sim = xezim::simulate_multi(
        &[KITCHEN.to_string()],
        1_000_000,
        None,
        &[],
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
        Some(path.to_str().unwrap()),
        &["top.u_sub".to_string()],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let fst = decode(&path);
    let _ = std::fs::remove_file(&path);

    assert!(fst.has("top.u_sub.mid"), "have {:?}", fst.paths());
    assert!(
        !fst.has("top.clk"),
        "`--fst-scope top.u_sub` must exclude top-level signals; have {:?}",
        fst.paths()
    );
}

/// FST hardcodes `FstVarType::Wire` for every var and emits no bit range,
/// discarding the classification `dump_var_kind()` already computes for VCD.
/// `logic [15:8] hi` renumbers to `[7:0]` and an ascending `logic [0:7]` loses
/// its bit order. FST carries the range inside the var name (`hi [15:8]`) —
/// the form Verilator writes.
#[test]
fn var_declarations_carry_the_right_type_and_bit_range() {
    let fst = dump("vartype", KITCHEN);
    assert_eq!(fst.var("top.hi").tpe, FstVarType::Reg);
    assert_eq!(fst.var("top.u_sub.dout").tpe, FstVarType::Wire);
    assert_eq!(fst.var("top.iv").tpe, FstVarType::Integer);
    assert_eq!(fst.var("top.r").tpe, FstVarType::Real);
    assert_eq!(fst.var("top.ev").tpe, FstVarType::Event);
    assert_eq!(fst.var("top.hi").name, "hi [15:8]");
    assert_eq!(fst.var("top.asc").name, "asc [0:7]");
}

/// Every var is declared `FstSignalType::bit_vec`, so a `real` becomes a
/// 64-bit vector carrying the float's raw bit image: `real r = 1.25` decodes
/// as 0x3FF4000000000000. VCD and XTrace both carry explicit fixes for this;
/// `fst-writer` exposes `FstSignalType::real()`.
#[test]
fn real_variables_decode_as_reals() {
    let fst = dump("real", KITCHEN);
    assert_eq!(fst.var("top.r").tpe, FstVarType::Real);
    let tl = fst.timeline("top.r");
    assert_eq!(tl[0].1, "r0");
    assert_eq!(tl[1].1, "r1.25");
}

/// An SV `event` is a pulse, not a level. Tracing its 1-bit storage makes a
/// design firing `->ev` every posedge show a signal toggling at HALF the
/// trigger rate, and a trigger whose 0→1→0 cancels inside one time slot
/// vanishes. VCD emits a `1<id>` pulse per trigger, XTrace an `X,event` record.
#[test]
fn an_event_emits_a_pulse_at_every_trigger() {
    let fst = dump("event", KITCHEN);
    assert_eq!(fst.var("top.ev").tpe, FstVarType::Event);
    // Posedges at 5, 15, 25 within the 32 ns run: three pulses, all value 1.
    let pulses: Vec<u64> = fst
        .changes
        .get(&fst.var("top.ev").handle)
        .map(|v| {
            v.iter()
                .filter(|(_, x)| x == "1")
                .map(|(t, _)| *t)
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(pulses, vec![5, 15, 25], "one pulse per trigger");
}

/// A `--fst-scope` that selects nothing must not leave an FST behind. An FST
/// with no variables is not a readable file — GTKWave and `fst2vcd` both
/// refuse to open it — so writing one hands the user a broken artifact where
/// the count line (`[FST] dumping 0 signals`) is the only clue. The common
/// cause is a generate block: the scopes are `gen[0]`/`gen[1]`, so a bare
/// `--fst-scope top.gen` matches nothing.
#[test]
fn a_scope_that_selects_nothing_warns_and_writes_no_file() {
    let bin = {
        let mut p = std::env::current_exe().expect("current_exe");
        p.pop();
        if p.ends_with("deps") {
            p.pop();
        }
        p.join("xezim")
    };
    let dir = std::env::temp_dir().join(format!("xezim_fst_scope_none_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sv = dir.join("h.sv");
    std::fs::write(
        &sv,
        r#"
module top;
  logic clk = 0;
  logic [15:0] top_sig;
  always #5 clk = ~clk;
  always @(posedge clk) top_sig <= top_sig + 1;
  genvar g;
  generate for (g = 0; g < 2; g++) begin : gen
    logic [1:0] gsig;
    always @(posedge clk) gsig <= gsig + 1;
  end endgenerate
  initial begin repeat (5) @(posedge clk); $finish; end
endmodule
"#,
    )
    .expect("write");

    let run = |scope: &str, out: &std::path::Path| -> String {
        let _ = std::fs::remove_file(out);
        let o = std::process::Command::new(&bin)
            .current_dir(&dir)
            .args([
                "--simulate",
                "--max-time",
                "200",
                "-s",
                "top",
                "--fst-scope",
                scope,
                "--fst",
            ])
            .arg(out)
            .arg(&sv)
            .output()
            .expect("run xezim");
        String::from_utf8_lossy(&o.stderr).into_owned()
    };

    // A generate block named without its index selects nothing.
    let gen_fst = dir.join("gen.fst");
    let err = run("top.gen", &gen_fst);
    assert!(
        !gen_fst.exists(),
        "an FST with no variables must not be written:\n{err}"
    );
    assert!(
        err.contains("matched no signals") && err.contains("top.gen[0]"),
        "expected a warning pointing at the indexed form:\n{err}"
    );

    // A plain typo must NOT be told to add an index.
    let typo_fst = dir.join("typo.fst");
    let err = run("top.nope", &typo_fst);
    assert!(!typo_fst.exists(), "no file for an unmatched scope:\n{err}");
    assert!(
        err.contains("matched no signals") && !err.contains("generate block"),
        "a typo must not get generate-block advice:\n{err}"
    );

    // The indexed form still works, and a good scope alongside a bad one
    // still produces a readable dump.
    let ok_fst = dir.join("ok.fst");
    let err = run("top.gen[0]", &ok_fst);
    assert!(ok_fst.exists(), "indexed generate scope must dump:\n{err}");
    let fst = decode(&ok_fst);
    assert!(fst.has("top.gen[0].gsig"), "have {:?}", fst.paths());
}

/// IEEE 1800 §10.8: waveform encoding follows the declaration even when a
/// temporary carries more bits. These shapes cover the reported 1/32 mismatch
/// and truncation, signed extension, four-state extension, fill and real data.
#[test]
fn declared_width_controls_fst_encoding() {
    use xezim::compiler::fst_sink::fst_format_declared_value as format;
    use xezim_core::value::{LogicBit, Value};
    let word = Value::from_u64(439, 32);
    assert_eq!(format(&word, 1), b"1");
    assert_eq!(format(&word, 8), b"10110111");
    let mut signed = Value::from_u64(0x80, 8);
    signed.is_signed = true;
    assert_eq!(format(&signed, 16), b"1111111110000000");
    assert_eq!(format(&Value::from_u64(0x80, 8), 16), b"0000000010000000");
    signed.set_bit(7, LogicBit::X);
    assert_eq!(format(&signed, 12), b"xxxxx0000000");
    signed.set_bit(7, LogicBit::Z);
    assert_eq!(format(&signed, 12), b"zzzzz0000000");
    let mut fill = Value::from_u64(1, 1);
    fill.is_fill = true;
    assert_eq!(format(&fill, 9), b"111111111");
    let mut wide = Value::zero(131);
    wide.set_bit(130, LogicBit::One);
    wide.set_bit(0, LogicBit::One);
    assert_eq!(format(&wide, 1), b"1");
    assert_eq!(format(&wide, 132).len(), 132);
    let mut real = Value::from_f64(3.25);
    real.is_real = false; // declaration metadata restores the real encoding
    assert_eq!(format(&real, 0), 3.25f64.to_le_bytes());
    assert_eq!(format(&Value::zero(0), 1), b"0");
}

/// Exercise both actual sink modes with mismatched runtime widths, then
/// decode the output instead of merely checking that the writer survived.
#[test]
fn sink_preserves_declared_width_inline_and_threaded() {
    use fst_writer::{
        FstFileType, FstInfo, FstScopeType, FstSignalType, FstVarDirection, FstVarType,
    };
    use xezim::compiler::fst_sink::{FstSink, FstTimestep, open_fst};
    use xezim_core::value::Value;
    for threaded in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "xezim_fst_width_{}_{}.fst",
            std::process::id(),
            threaded
        ));
        let info = FstInfo {
            start_time: 0,
            timescale_exponent: -9,
            version: "regression".into(),
            date: String::new(),
            file_type: FstFileType::Verilog,
        };
        let (mut header, target) = open_fst(path.to_str().unwrap(), &info).unwrap();
        header.scope("top", "", FstScopeType::Module).unwrap();
        let flag = header
            .var(
                "flag",
                FstSignalType::bit_vec(1),
                FstVarType::Reg,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        let byte = header
            .var(
                "byte",
                FstSignalType::bit_vec(8),
                FstVarType::Reg,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        let real = header
            .var(
                "real",
                FstSignalType::real(),
                FstVarType::Real,
                FstVarDirection::Implicit,
                None,
            )
            .unwrap();
        header.up_scope().unwrap();
        let mut body = header.finish().unwrap();
        body.signal_change(flag, b"0").unwrap();
        body.signal_change(byte, b"00000000").unwrap();
        body.signal_change(real, &0f64.to_le_bytes()).unwrap();
        let mut sink = if threaded {
            FstSink::threaded(body, target, 0)
        } else {
            FstSink::inline(body, target, 0)
        };
        sink.post(FstTimestep {
            time: 1,
            changes: vec![
                (flag, Value::from_u64(439, 32), 1),
                (byte, Value::from_u64(439, 32), 8),
                (real, Value::from_f64(3.25), 0),
            ],
        });
        sink.post(FstTimestep {
            time: 4,
            changes: vec![],
        });
        sink.finish();
        let fst = decode(&path);
        assert_eq!(fst.var("top.flag").length, 1);
        assert_eq!(fst.var("top.byte").length, 8);
        assert_eq!(
            fst.timeline("top.flag"),
            vec![(0, "0".into()), (1, "1".into())]
        );
        assert_eq!(
            fst.timeline("top.byte"),
            vec![(0, "00000000".into()), (1, "10110111".into())]
        );
        assert_eq!(
            fst.timeline("top.real"),
            vec![(0, "r0".into()), (1, "r3.25".into())]
        );
        assert_eq!(fst.end_time, 4);
        std::fs::remove_file(path).unwrap();
    }
}

/// Strings have no fixed maximum length (§6.16). They must not be declared as
/// 1024-bit vectors and later overflow the writer when a string grows.
#[test]
fn variable_length_strings_leave_other_fst_signals_complete() {
    for length in [0, 1, 127, 128, 129, 200] {
        let src = format!(
            r#"
module top;
  string text = "";
  bit flag = 0;
  logic [130:0] wide = 0;
  initial begin
    #1; text = {{{length}{{"A"}}}}; flag = 1; wide = 131'h400000000000000000000000000000001;
    #1; text = ""; flag = 0;
    #2; $finish;
  end
endmodule
"#
        );
        let fst = dump(&format!("string_{length}"), &src);
        assert!(
            !fst.has("top.text"),
            "string was declared as a fixed vector"
        );
        assert_eq!(
            fst.timeline("top.flag"),
            vec![(0, "0".into()), (1, "1".into()), (2, "0".into())]
        );
        assert_eq!(fst.var("top.wide").length, 131);
        let wide = &fst.timeline("top.wide")[1].1;
        assert_eq!(wide.len(), 131);
        assert!(wide.starts_with('1') && wide.ends_with('1'));
        assert_eq!(fst.end_time, 4);
    }
}

#[test]
fn string_typedef_and_array_do_not_become_fixed_vectors() {
    let fst = dump(
        "string_types",
        r#"
module top;
  typedef string text_t;
  text_t alias_text = "";
  string words[2];
  bit flag = 0;
  initial begin
    #1; alias_text = {129{"A"}}; words[0] = {200{"B"}}; words[1] = "short"; flag = 1;
    #3; $finish;
  end
endmodule
"#,
    );
    assert!(!fst.has("top.alias_text"));
    assert!(!fst.paths().iter().any(|p| p.starts_with("top.words")));
    assert_eq!(
        fst.timeline("top.flag"),
        vec![(0, "0".into()), (1, "1".into())]
    );
    assert_eq!(fst.end_time, 4);
}

#[test]
fn generated_and_aggregate_string_leaves_are_omitted() {
    let fst = dump(
        "string_paths",
        r#"
package types_pkg;
  typedef string text_t;
endpackage
module top;
  typedef int text_t;
  typedef struct { string text; bit flag; } entry_t;
  typedef struct { entry_t entry; string words[2]; bit flag; } outer_t;
  entry_t holder;
  entry_t entries[2];
  outer_t nested;
  types_pkg::text_t qualified_text = "";
  for (genvar g = 0; g < 2; g++) begin : gen
    typedef string generated_t;
    generated_t text = "";
    bit flag = 0;
    initial begin #1; text = {200{"G"}}; flag = 1; end
  end
  bit flag = 0;
  initial begin
    #1;
    holder.text = {200{"H"}}; holder.flag = 1;
    entries[0].text = {129{"E"}}; entries[1].text = "entry";
    nested.entry.text = {200{"N"}}; nested.words[0] = {129{"W"}};
    qualified_text = {200{"P"}}; flag = 1;
    #3; $finish;
  end
endmodule
"#,
    );
    for path in fst.paths() {
        assert!(
            !path.ends_with(".text")
                && !path.ends_with("qualified_text")
                && !path.contains(".words"),
            "unsupported string declared: {path}"
        );
    }
    for path in [
        "top.flag",
        "top.gen[0].flag",
        "top.gen[1].flag",
        "top.holder.flag",
    ] {
        assert_eq!(
            fst.timeline(path),
            vec![(0, "0".into()), (1, "1".into())],
            "{path}"
        );
    }
    assert_eq!(fst.end_time, 4);
}

/// `--fst-repack`: the finished dump is gzip-wrapped in an FST `ZWRAPPER`
/// block (GTKWave's repack-on-close). The file starts with the wrapper's
/// type byte, and a reader inflates it on open to the same hierarchy and
/// value changes as the plain dump.
#[test]
fn fst_repack_wraps_the_dump_and_reads_back() {
    let src = "module top;\n\
               \x20 reg clk = 0; reg [7:0] cnt = 0;\n\
               \x20 always #5 clk = ~clk;\n\
               \x20 always @(posedge clk) cnt <= cnt + 3;\n\
               \x20 initial #100 $finish;\n\
               endmodule\n";
    let dir = std::env::temp_dir().join(format!("xezim_fst_repack_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let design = dir.join("top.sv");
    std::fs::write(&design, src).unwrap();
    let run = |args: &[&str], out: &PathBuf| {
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
            .arg(&design)
            .args(["--simulate", "-s", "top", "--no-cache", "--fst"])
            .arg(out)
            .args(args)
            .output()
            .expect("run xezim");
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );
    };
    let plain = dir.join("plain.fst");
    let packed = dir.join("packed.fst");
    run(&[], &plain);
    run(&["--fst-repack=4"], &packed);
    let head = std::fs::read(&packed).unwrap();
    assert_eq!(head[0], 254, "packed dump does not start with a ZWRAPPER block");
    assert_ne!(std::fs::read(&plain).unwrap()[0], 254);
    assert!(
        head.len() < std::fs::metadata(&plain).unwrap().len() as usize + 64,
        "the wrapper should not grow a tiny dump by more than its headers"
    );
    let a = decode(&plain);
    let b = decode(&packed);
    assert_eq!(a.paths(), b.paths());
    assert_eq!(a.timeline("top.cnt"), b.timeline("top.cnt"));
    // Ten rising edges before `$finish` at 100: 10 * 3 = 30.
    assert_eq!(b.timeline("top.cnt").last().map(|(_, v)| v.as_str()), Some("00011110"));
    assert_eq!((a.start_time, a.end_time), (b.start_time, b.end_time));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--fst-repack` on a dump path that is a symbolic link repacks the link's
/// target and keeps the link: renaming over the path made the link a regular
/// file and left the target uncompressed (#598, reported by akashlevy). No
/// temporary file is left beside the target.
#[cfg(unix)]
#[test]
fn fst_repack_through_a_symbolic_link() {
    let dir = std::env::temp_dir().join(format!("xezim_fst_repack_link_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let design = dir.join("top.sv");
    std::fs::write(
        &design,
        "module top; reg [7:0] c = 0; always #5 c = c + 1; initial #50 $finish; endmodule\n",
    )
    .unwrap();
    let target = dir.join("real.fst");
    let link = dir.join("link.fst");
    std::fs::write(&target, b"").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg(&design)
        .args(["--simulate", "-s", "top", "--no-cache", "--fst"])
        .arg(&link)
        .arg("--fst-repack=4")
        .output()
        .expect("run xezim");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the dump path is no longer a link"
    );
    assert_eq!(
        std::fs::read(&target).unwrap()[0],
        254,
        "the target was not repacked"
    );
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".repack."))
        .collect();
    assert!(leftovers.is_empty(), "temporary files left: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
