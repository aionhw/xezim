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
//! Declaration-level gaps that are known and NOT yet fixed have `#[ignore]`d
//! tests with the reason on each, so they gate the fix rather than silently
//! blessing the current output. Run them with
//! `cargo test --test gates -- --ignored`. F3 (real dumped as its raw
//! IEEE-754 bit pattern) is FIXED and its test now runs; F4 (event traced as
//! a level) and F5 (var type + bit range) remain.

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
        1,
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
        None,
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
                assert_eq!(tpe, FstScopeType::Module, "scope `{}` is not a module", name);
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
        assert_eq!(val.len(), 96, "a 96-bit net must emit 96 characters: {}", val);
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
        1,
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
        None,
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

// ── known gaps: these gate the fix, they do not bless the current output ──

/// FST hardcodes `FstVarType::Wire` for every var and emits no bit range,
/// discarding the classification `dump_var_kind()` already computes for VCD.
/// `logic [15:8] hi` renumbers to `[7:0]` and an ascending `logic [0:7]` loses
/// its bit order. FST carries the range inside the var name (`hi [15:8]`) —
/// the form Verilator writes.
#[test]
#[ignore = "F5: all FST vars are typed Wire with no bit range (fix pending)"]
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

/// F3 (FIXED). Every var used to be declared `FstSignalType::bit_vec`, so a
/// `real` became a 64-bit vector carrying the float's raw bit image: `real r =
/// 1.25` decoded as 0x3FF4000000000000. Reals now declare FST's native real
/// slot (`FstSignalType::real()` / `FstVarType::Real`) and their changes are
/// written as the f64's 8 raw bytes, which is what a viewer decodes back to a
/// double — parity with the VCD (`r<decimal>`) and XTrace paths.
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
#[ignore = "F4: event is traced as a level, so triggers cancel (fix pending)"]
fn an_event_emits_a_pulse_at_every_trigger() {
    let fst = dump("event", KITCHEN);
    assert_eq!(fst.var("top.ev").tpe, FstVarType::Event);
    // Posedges at 5, 15, 25 within the 32 ns run: three pulses, all value 1.
    let pulses: Vec<u64> = fst
        .changes
        .get(&fst.var("top.ev").handle)
        .map(|v| v.iter().filter(|(_, x)| x == "1").map(|(t, _)| *t).collect())
        .unwrap_or_default();
    assert_eq!(pulses, vec![5, 15, 25], "one pulse per trigger");
}
