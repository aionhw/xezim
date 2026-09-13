//! Activity-trace census recorder for xezim.
//!
//! Counts per-signal value changes during a simulation run and writes a
//! deterministic JSON sidecar at finalize. The census is a *superset* —
//! every signal in the design gets a row, with `change_count == 0` for
//! signals that never changed. The sidecar is keyed by RTL hierarchical
//! path (top-relative, same form the signal table uses) so it lines up
//! with an FST/VCD/XTrace dump.
//!
//! ## Design
//!
//! The census uses three dense `Vec`s indexed by signal id — the same
//! indexing the signal table and every other per-signal array in the
//! simulator uses. Incrementing happens inside `dump_write_changes`
//! over the shared `dump_dirty` set, so the per-write hot path pays
//! only a branch (the `act_trace_active` flag), not a counter increment.
//!
//! First and last change times use `u64::MAX` as the "not yet changed"
//! sentinel for first-change (sim time starts at 0, so `MAX` is safe)
//! and 0 for last-change (a signal that only changes at t=0 still
//! records that fact).
//!
//! The sidecar is written by the simulation thread at finalize, after
//! the FST writer has finished — the census data is small (one JSON
//! object per signal) and does not need a background thread.

use std::io::{self, Write};

/// Per-signal activity census. Dense, cache-friendly, zero-allocation
/// once initialized.
pub struct ActTraceCensus {
    /// Change count per signal id.
    pub counters: Vec<u64>,
    /// First change time per signal id (`u64::MAX` = not yet changed).
    pub first_change: Vec<u64>,
    /// Last change time per signal id.
    pub last_change: Vec<u64>,
    /// Number of signals that have changed at least once.
    pub active_count: u64,
}

impl ActTraceCensus {
    /// Allocate counters for `nsig` signals. All start at zero / MAX.
    pub fn new(nsig: usize) -> Self {
        ActTraceCensus {
            counters: vec![0; nsig],
            first_change: vec![u64::MAX; nsig],
            last_change: vec![0; nsig],
            active_count: 0,
        }
    }

    /// Record one change to signal `sid` at time `t`.
    #[inline]
    pub fn record(&mut self, sid: usize, t: u64) {
        if sid < self.counters.len() {
            if self.counters[sid] == 0 {
                self.first_change[sid] = t;
                self.active_count += 1;
            }
            self.counters[sid] += 1;
            self.last_change[sid] = t;
        }
    }

    /// Write the census sidecar as a sorted JSON file. Each entry is the
    /// signal's original id (which indexes `counters`/`first_change`/
    /// `last_change`), its RTL path, and its declared width in bits. The
    /// caller may have filtered enum-literal constants and empty-name slots
    /// out of the signal table, so list position in `signals` is not the
    /// signal id — counters must be resolved through the id, not re-indexed.
    /// Output is deterministic: signals are sorted by path, values formatted
    /// consistently.
    pub fn write_sidecar(
        &self,
        w: &mut dyn Write,
        signals: &[(usize, &str, u32)],
        tick_s: f64,
    ) -> io::Result<()> {
        // Build (path, width, count, first, last) tuples, sorted by path.
        let mut entries: Vec<(&str, u32, u64, u64, u64)> = signals
            .iter()
            .map(|&(id, name, width)| {
                let cnt = self.counters.get(id).copied().unwrap_or(0);
                let first = self.first_change.get(id).copied().unwrap_or(u64::MAX);
                let last = self.last_change.get(id).copied().unwrap_or(0);
                (name, width, cnt, first, last)
            })
            .collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));

        writeln!(w, "{{")?;
        writeln!(w, "  \"version\": 1,")?;
        writeln!(w, "  \"tick_s\": {},", tick_s)?;
        writeln!(w, "  \"total_signals\": {},", entries.len())?;
        writeln!(w, "  \"active_signals\": {},", self.active_count)?;
        writeln!(w, "  \"signals\": [")?;
        let total = entries.len();
        for (idx, (path, width, count, first, last)) in entries.iter().enumerate() {
            let comma = if idx + 1 < total { "," } else { "" };
            let first_ns = if *first == u64::MAX {
                -1.0
            } else {
                *first as f64 * tick_s * 1e9
            };
            let last_ns = *last as f64 * tick_s * 1e9;
            writeln!(
                w,
                "    {{\"path\": \"{}\", \"width\": {}, \"change_count\": {}, \"first_change_ns\": {}, \"last_change_ns\": {}}}{comma}",
                path, width, count, first_ns, last_ns,
            )?;
        }
        writeln!(w, "  ]")?;
        writeln!(w, "}}")?;
        Ok(())
    }
}

/// Validated `--debug` flag family configuration.
///
/// Produced by [`resolve_debug_flags`]; `main` turns it into the
/// process-global dump/trace setters.
#[derive(Clone, Debug)]
pub struct DebugConfig {
    /// Record the per-signal activity census sidecar.
    pub census: bool,
    /// Expand the driver/load graph to every signal (`--debug+all`).
    pub trace_all_signals: bool,
    /// Named query signals for the driver/load graph.
    pub query_signals: Vec<String>,
    /// Graph sidecar path: user-named, or the auto default under `--debug`.
    pub graph_file: Option<String>,
    /// FST dump path: user-named, or the auto default under `--debug`.
    pub fst_file: Option<String>,
    /// Per-scope dump depths from `--debug-scope-file`.
    pub scope_depths: Vec<(String, u32)>,
    /// Validated `--debug-access` tokens: `r`, `w`, `mem`, `pp`, `all`.
    pub access: Vec<String>,
}

/// Parsed `--debug*` flags handed to [`resolve_debug_flags`]. The booleans and
/// optional paths are exactly the parsed CLI values, so the resolver is a
/// pure function over the flag set.
#[derive(Clone, Debug)]
pub struct DebugFlagsInput {
    pub all: bool,
    pub access_raw: Option<String>,
    pub query_signals: Vec<String>,
    pub graph_file: Option<String>,
    pub scope_depths: Vec<(String, u32)>,
    pub auto_output: bool,
    pub census_requested: bool,
    pub fst_requested: Option<String>,
    pub default_top: Option<String>,
    pub default_graph: String,
    pub default_fst: String,
}

/// Resolve the `--debug*` flag family into a concrete configuration.
///
/// Every member stands alone: `--debug+all` is the whole-debug switch,
/// `--debug-access=+r` and `--debug-scope-file <f>` enable the census + FST
/// pipeline themselves, and `--debug-signals` produces the graph only. No
/// separate `--debug` prefix flag exists. Rejects contradictory or
/// meaningless combinations; the error message names the conflict and shows a
/// valid command built from the same flags. This function is the single
/// source of truth: the binary calls it after parsing, and the
/// flag-combination tests call it directly over an exhaustive truth table.
pub fn resolve_debug_flags(input: DebugFlagsInput) -> Result<DebugConfig, String> {
    let DebugFlagsInput {
        all: debug_all,
        access_raw: debug_access,
        query_signals,
        graph_file,
        scope_depths,
        auto_output,
        census_requested,
        fst_requested,
        default_top,
        default_graph,
        default_fst,
    } = input;
    let mut access: Vec<String> = Vec::new();
    if let Some(raw) = debug_access {
        let toks: Vec<&str> = raw
            .trim_start_matches('+')
            .split('+')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        if toks.is_empty() {
            return Err(format!(
                "--debug-access value '{}' is empty; access tokens are joined with '+'\n\
                 Use --debug-access=+rw or --debug-access=+mem+pp.",
                raw
            ));
        }
        for tok in toks {
            match tok {
                "r" | "w" | "mem" | "pp" | "all" => access.push(tok.to_string()),
                // `+rw` is shorthand for `+r+w`; the single letters r/w may be
                // combined in one token, everything else must be spelled out.
                _ if !tok.is_empty() && tok.chars().all(|c| c == 'r' || c == 'w') => {
                    for ch in tok.chars() {
                        access.push(ch.to_string());
                    }
                }
                _ => {
                    return Err(format!(
                        "--debug-access token '{}' is not valid; allowed: r, w, mem, pp, all\n\
                         (r and w may be combined, e.g. +rw)\n\
                         Use --debug-access=+rw or --debug-access=+mem+pp.",
                        tok
                    ));
                }
            }
        }
    }
    if debug_all && !scope_depths.is_empty() {
        return Err(
            "--debug+all cannot be combined with a --debug-scope-file selection: \
             +all dumps the whole hierarchy, scope-file limits the dump depth\n\
             Use one dump scope: --debug-scope-file <file>, or --debug+all alone."
                .to_string(),
        );
    }
    if debug_all && !query_signals.is_empty() {
        return Err(
            "--debug+all cannot be combined with --debug-signals: +all traces every \
             signal, --debug-signals names a subset\n\
             Use --debug-signals a,b --debug-graph-file g.json, or drop the \
             signal list for --debug+all."
                .to_string(),
        );
    }
    let tracing = debug_all || !access.is_empty() || !scope_depths.is_empty();
    if !tracing && !auto_output {
        return Err(
            "--debug-no-auto-output disables automatic output names, so it needs a \
             --debug* flag that implies debug output (--debug+all, --debug-access=+r, \
             or --debug-scope-file <file>) alongside\n\
             Use --debug+all --debug-no-auto-output --fst out.fst --debug-graph-file g.json."
                .to_string(),
        );
    }
    // The FST dump defaults to the whole hierarchy (the top module at depth 0)
    // so enabling tracing is useful immediately. A scope-file selection or
    // `--debug-no-auto-output` keeps whatever the user chose.
    let scope_depths = if scope_depths.is_empty() && tracing && auto_output {
        default_top
            .map(|top| vec![(top.to_string(), 0u32)])
            .unwrap_or_default()
    } else {
        scope_depths
    };
    Ok(DebugConfig {
        census: census_requested || tracing,
        trace_all_signals: debug_all,
        query_signals: if debug_all { Vec::new() } else { query_signals },
        graph_file: graph_file.or_else(|| {
            if tracing && auto_output {
                Some(default_graph.to_string())
            } else {
                None
            }
        }),
        fst_file: fst_requested.or_else(|| {
            if tracing && auto_output {
                Some(default_fst.to_string())
            } else {
                None
            }
        }),
        scope_depths,
        access,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn census_basic() {
        let mut c = ActTraceCensus::new(4);
        c.record(1, 10);
        c.record(1, 20);
        c.record(3, 5);
        assert_eq!(c.counters[1], 2);
        assert_eq!(c.first_change[1], 10);
        assert_eq!(c.last_change[1], 20);
        assert_eq!(c.counters[3], 1);
        assert_eq!(c.first_change[3], 5);
        assert_eq!(c.counters[0], 0);
        assert_eq!(c.first_change[0], u64::MAX);
        assert_eq!(c.active_count, 2);
    }

    #[test]
    fn census_sidecar_deterministic() {
        let mut c = ActTraceCensus::new(3);
        c.record(2, 100);
        c.record(0, 50);
        let signals = vec![(2usize, "sig_c", 8u32), (0, "sig_a", 1), (1, "sig_b", 16)];
        let mut out1 = Vec::new();
        let mut out2 = Vec::new();
        c.write_sidecar(&mut out1, &signals, 1e-9).unwrap();
        c.write_sidecar(&mut out2, &signals, 1e-9).unwrap();
        assert_eq!(out1, out2, "sidecar output must be byte-identical");
    }

    #[test]
    fn census_rows_resolve_counters_by_signal_id() {
        // A caller can drop signal 0 (an enum-literal constant) from the row
        // list. The surviving rows must resolve their change counts through
        // their ORIGINAL ids: id 1 keeps 2 changes and id 2 keeps 1, instead
        // of shifting onto each other.
        let mut c = ActTraceCensus::new(3);
        c.record(1, 10);
        c.record(1, 20);
        c.record(2, 300);
        let signals = [(1usize, "a", 1u32), (2, "b", 2)]; // id 0 filtered out
        let mut out = Vec::new();
        c.write_sidecar(&mut out, &signals, 1e-9).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("\"path\": \"a\", \"width\": 1, \"change_count\": 2"),
            "row a must show id-1 counters: {s}"
        );
        assert!(
            s.contains("\"path\": \"b\", \"width\": 2, \"change_count\": 1"),
            "row b must show id-2 counters: {s}"
        );
    }
}

// ---------------------------------------------------------------------------
// Sidecar readers. The census and graph writers above emit deterministic
// JSON; these loaders read it back into typed structs so Rust, C, and
// Python consumers can query the results of a run without re-parsing
// strings. Parsing goes through `mini_json`, a dependency-free reader for
// exactly the subset the sidecars emit.
// ---------------------------------------------------------------------------

use super::mini_json::{JsonValue, parse as parse_json};

/// One row of the driver/load graph sidecar.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceGraphRow {
    /// RTL hierarchical path, top-relative (same form the signal table uses).
    pub path: String,
    /// Transitive write-set (fan-in) cone: everything that can drive this
    /// signal, including itself.
    pub drivers: Vec<String>,
    /// Transitive read-set (fan-out) cone: everything this signal feeds.
    pub loads: Vec<String>,
}

/// A parsed `--debug-graph-file` sidecar.
#[derive(Clone, Debug, Default)]
pub struct TraceGraph {
    pub signals: Vec<TraceGraphRow>,
}

impl TraceGraph {
    /// Parse a graph sidecar from its text.
    pub fn parse(src: &str) -> Result<TraceGraph, String> {
        let v =
            parse_json(src).map_err(|e| format!("bad graph JSON at {}: {}", e.offset, e.msg))?;
        let obj = JsonObj(v.as_obj().ok_or("graph sidecar must be a JSON object")?);
        let signals = obj
            .get_arr("signals")
            .ok_or("graph sidecar missing \"signals\" array")?;
        let mut out = Vec::with_capacity(signals.len());
        for s in signals {
            let row = JsonObj(s.as_obj().ok_or("graph row must be an object")?);
            let path = row.get_str("path").ok_or("graph row missing \"path\"")?;
            let drivers = row.get_strs("drivers");
            let loads = row.get_strs("loads");
            out.push(TraceGraphRow {
                path: path.to_string(),
                drivers,
                loads,
            });
        }
        Ok(TraceGraph { signals: out })
    }

    /// Read and parse a graph sidecar from disk.
    pub fn read(path: &std::path::Path) -> Result<TraceGraph, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        TraceGraph::parse(&text)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.signals.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    /// Row for a signal path, or `None` if the sidecar does not list it.
    pub fn row(&self, path: &str) -> Option<&TraceGraphRow> {
        self.signals.iter().find(|r| r.path == path)
    }
}

/// One census row: the value-change summary for a single signal.
#[derive(Clone, Debug, PartialEq)]
pub struct CensusRow {
    /// RTL hierarchical path, top-relative.
    pub path: String,
    /// Declared width of the signal's data type in bits.
    pub width: u32,
    /// Number of times the signal's value changed during the run.
    pub change_count: u64,
    /// Time of the first change in seconds, or `None` if never changed.
    pub first_change_s: Option<f64>,
    /// Time of the last change in seconds, or `None` if never changed.
    pub last_change_s: Option<f64>,
}

/// A parsed census sidecar (XEZIM_ACT_TRACE_CENSUS_FILE; default `trace_census.json`).
#[derive(Clone, Debug, Default)]
pub struct TraceCensus {
    pub tick_s: f64,
    pub total_signals: usize,
    pub active_signals: usize,
    pub rows: Vec<CensusRow>,
}

impl TraceCensus {
    /// Parse a census sidecar from its text.
    pub fn parse(src: &str) -> Result<TraceCensus, String> {
        let v =
            parse_json(src).map_err(|e| format!("bad census JSON at {}: {}", e.offset, e.msg))?;
        let obj = JsonObj(v.as_obj().ok_or("census sidecar must be a JSON object")?);
        let tick_s = obj.get_num("tick_s").unwrap_or(0.0);
        let total_signals = obj.get_num("total_signals").unwrap_or(0.0) as usize;
        let active_signals = obj.get_num("active_signals").unwrap_or(0.0) as usize;
        let mut rows = Vec::new();
        if let Some(list) = obj.get_arr("signals") {
            for s in list {
                let row = JsonObj(s.as_obj().ok_or("census row must be an object")?);
                let path = row.get_str("path").ok_or("census row missing \"path\"")?;
                let width = row.get_num("width").unwrap_or(0.0) as u32;
                let change_count = row.get_num("change_count").unwrap_or(-1.0);
                let first = row.get_num("first_change_ns").unwrap_or(-1.0);
                let last = row.get_num("last_change_ns").unwrap_or(-1.0);
                rows.push(CensusRow {
                    path: path.to_string(),
                    width,
                    change_count: if change_count < 0.0 {
                        0
                    } else {
                        change_count as u64
                    },
                    first_change_s: if first < 0.0 {
                        None
                    } else {
                        Some(first * 1e-9)
                    },
                    last_change_s: if last < 0.0 { None } else { Some(last * 1e-9) },
                });
            }
        }
        Ok(TraceCensus {
            tick_s,
            total_signals,
            active_signals,
            rows,
        })
    }

    /// Read and parse a census sidecar from disk.
    pub fn read(path: &std::path::Path) -> Result<TraceCensus, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        TraceCensus::parse(&text)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Census row for a signal path, or `None` if the sidecar does not list it.
    pub fn row(&self, path: &str) -> Option<&CensusRow> {
        self.rows.iter().find(|r| r.path == path)
    }
}

/// Lookup helpers for a parsed JSON object (a `Vec` of members in order).
/// The sidecars are known shapes, so lookups are linear scans; a row holds a
/// handful of members, so hashing would cost more than it saves.
struct JsonObj<'a>(&'a [(String, JsonValue)]);

impl JsonValue {
    fn as_obj(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Obj(m) => Some(m.as_slice()),
            _ => None,
        }
    }
}

impl<'a> JsonObj<'a> {
    fn get(&self, key: &str) -> Option<&'a JsonValue> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn get_str(&self, key: &str) -> Option<&'a str> {
        match self.get(key)? {
            JsonValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn get_num(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            JsonValue::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn get_arr(&self, key: &str) -> Option<&'a [JsonValue]> {
        match self.get(key)? {
            JsonValue::Arr(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    fn get_strs(&self, key: &str) -> Vec<String> {
        match self.get_arr(key) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    JsonValue::Str(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
            None => Vec::new(),
        }
    }
}
