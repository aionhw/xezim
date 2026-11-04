//! Simulator waveforms.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    // ═══════════════════════════════════════════════════════════════
    // VCD dump support ($dumpfile / $dumpvars)
    // ═══════════════════════════════════════════════════════════════

    /// Generate a VCD identifier code from an index (!, ", #, ... multi-char for large designs)
    pub(super) fn vcd_id_code(mut idx: usize) -> String {
        let mut code = String::new();
        loop {
            code.push((b'!' + (idx % 94) as u8) as char);
            idx /= 94;
            if idx == 0 {
                break;
            }
            idx -= 1;
        }
        code
    }

    /// Whether dump writers (VCD / XTrace) offload formatting and I/O
    /// to a background thread. Always considered: dump writing is pure
    /// I/O and benefits even a single-threaded sim by moving ASCII formatting
    /// and file writes off the simulation thread. Set `XEZIM_DUMP_INLINE=1` to
    /// force inline writing (deterministic debugging, or to avoid the thread).
    pub(super) fn dump_writer_threaded(&self) -> bool {
        std::env::var_os("XEZIM_DUMP_INLINE").is_none()
    }

    // ── $dumpvars scope/name filtering (IEEE 1800-2017 §21.7.1.4) ──────
    //
    // The signal table names every object RELATIVE to the top module (`clk`,
    // `u_sub.c`), but a `$dumpvars` scope argument resolves through
    // `resolve_hier_name` to an ABSOLUTE path that INCLUDES the top (`top`,
    // `top.u_sub`). Comparing the two directly never matches, so the extremely
    // common `$dumpvars(0, top);` selected zero signals and produced an empty
    // dump. The same copy-pasted filter backed the FST and XTrace dumps.

    /// Rewrite dump scope arguments into the top-relative form the signal table
    /// uses. `None` means "no filter" — either no arguments at all, or an
    /// argument naming the top module itself (everything lives under it).
    pub(super) fn dump_filter_prefixes(top: &str, scopes: &[String]) -> Option<Vec<String>> {
        if scopes.is_empty() {
            return None;
        }
        let top_dot = format!("{}.", top);
        let mut out = Vec::with_capacity(scopes.len());
        for s in scopes {
            if s == top {
                return None;
            }
            out.push(
                s.strip_prefix(top_dot.as_str())
                    .unwrap_or(s.as_str())
                    .to_string(),
            );
        }
        Some(out)
    }

    /// Whether a top-relative signal name is selected by a normalized filter
    /// list and the `$dumpvars` depth (§21.7.1.4: 0 = all levels below the
    /// scope, N = N levels starting at the scope; a filter that names a SIGNAL
    /// rather than a scope always matches exactly).
    /// Split an FST scope entry `LEVEL:SCOPE` (e.g. `1:top.u1`) into its depth
    /// and scope; an entry without a numeric prefix has depth 0 (all levels).
    /// A hierarchical name cannot start with `<digits>:`, so this is unambiguous.
    pub(super) fn fst_scope_depth(entry: &str) -> (u32, &str) {
        if let Some((lvl, scope)) = entry.split_once(':') {
            if !lvl.is_empty() && lvl.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(d) = lvl.parse::<u32>() {
                    return (d, scope);
                }
            }
        }
        (0, entry)
    }

    pub(super) fn dump_name_selected(name: &str, filters: Option<&[String]>, depth: u32) -> bool {
        // `rest` is the name relative to the selected scope: no dot = level 1.
        let level_ok = |rest: &str| depth == 0 || (rest.matches('.').count() as u32) < depth;
        match filters {
            None => level_ok(name),
            Some(fs) => fs.iter().any(|f| {
                if name == f.as_str() {
                    return true;
                }
                match name
                    .strip_prefix(f.as_str())
                    .and_then(|r| r.strip_prefix('.'))
                {
                    Some(rest) => level_ok(rest),
                    None => false,
                }
            }),
        }
    }

    /// The objects a dump should carry, enumerated from the REAL signal table
    /// (`id_to_name`) rather than the lazily-synced `self.signals` mirror. That
    /// mirror is only filled by `sync_table_to_hashmap()` when `table_modified`
    /// is set, so a purely behavioral design (nothing marks the table dirty
    /// before `$dumpvars`) dumped NOTHING at all.
    ///
    /// §21.7.2.1 admits only nets and variables, so the entries that are not
    /// objects are dropped here: module/interface INSTANCE handles and UNPACKED
    /// STRUCT aggregates (both of which are really SCOPES and were emitted as
    /// stuck-at-x 1-bit wires beside their own scope), enum LITERALS (which
    /// `elaborate` registers as signals), and the base name of an unpacked array
    /// whose elements are dumped individually.
    pub(super) fn dump_signal_names(&self, scopes: &[String], depth: u32) -> Vec<String> {
        let filters = Self::dump_filter_prefixes(&self.module.name, scopes);

        // Enum literals (`RED`/`GRN`/`BLU`) are registered as signals by
        // `elaborate::register_*_enum_members` — they are constants, not objects.
        let mut enum_lits: HashSet<&str> = HashSet::default();
        for members in self.module.enum_members.values() {
            for (nm, _) in members {
                enum_lits.insert(nm.as_str());
            }
        }

        // Non-object table entries, computed over the WHOLE table (never over the
        // selected subset — at `$dumpvars(1, top)` a scope's children are all
        // filtered out, and an instance handle that is no longer the prefix of
        // any selected name would leak straight back in as a 1-bit x wire):
        //
        //   * every dotted PREFIX of a name is a scope, so a same-named entry is
        //     an instance handle or an UNPACKED STRUCT aggregate, not an object.
        //     A PACKED struct is exempt — there the aggregate IS the storage and
        //     its flattened members are slices of it.
        //   * every module/interface INSTANCE path (authoritative, and it also
        //     covers an instance whose module declares nothing at all).
        //   * the base of an unpacked array whose elements are dumped
        //     individually (`mem` beside `mem[0]`…`mem[3]`).
        let n = self.signal_table.len();
        let stored = || {
            self.id_to_name
                .iter()
                .take_while(move |(id, _)| *id < n)
                .map(|(_, name)| name.as_ref())
        };
        // Virtual-name arrays (see `NameMap`): their bases stand in for their
        // elements when collecting scopes — an element's dotted prefixes are
        // its base's.
        let virtual_arrays: Vec<(&str, i64, i64)> = self
            .signal_name_to_id
            .virtual_arrays()
            .filter(|(_, first, _, _)| *first < n)
            .map(|(b, _, lo, hi)| (b.as_ref(), lo, hi))
            .collect();
        let mut scope_names: HashSet<&str> = HashSet::default();
        let mut expanded_bases: HashSet<&str> = HashSet::default();
        for &(base, _, _) in &virtual_arrays {
            let mut cut = 0usize;
            while let Some(p) = base[cut..].find('.') {
                let end = cut + p;
                if !self.module.packed_struct_fields.contains_key(&base[..end]) {
                    scope_names.insert(&base[..end]);
                }
                cut = end + 1;
            }
            expanded_bases.insert(base);
        }
        for name in stored() {
            let mut cut = 0usize;
            while let Some(p) = name[cut..].find('.') {
                let end = cut + p;
                if !self.module.packed_struct_fields.contains_key(&name[..end]) {
                    scope_names.insert(&name[..end]);
                }
                cut = end + 1;
            }
            if let Some(p) = name.find('[') {
                expanded_bases.insert(&name[..p]);
            }
        }
        for inst in &self.module.instances {
            scope_names.insert(inst.path.as_str());
        }

        let mut out: Vec<String> = Vec::new();
        let keep = |name: &str| -> bool {
            !(name.is_empty()
                || enum_lits.contains(name)
                || scope_names.contains(name)
                || expanded_bases.contains(name)
                || xezim_core::elaborate::is_delay_stage_net(name)
                || event_bits::is_edge_alias_net(name))
                && Self::dump_name_selected(name, filters.as_deref(), depth)
        };
        for name in stored() {
            if keep(name) {
                out.push(name.to_string());
            }
        }
        for &(base, lo, hi) in &virtual_arrays {
            for idx in lo..=hi {
                let name = format!("{}[{}]", base, idx);
                if keep(&name) {
                    out.push(name);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The signal-table entry that BACKS each dumped name.
    ///
    /// An instance port whose actual is a whole net (`.din(src_bus)`) is not a
    /// second object: inlining gives the formal its own table entry kept in step
    /// by a port continuous-assign, but `src_bus` and `u_sub.din` are ONE net.
    /// Verilator collapses them (same `$var` identifier code, one value-change
    /// record per change); so does a reference simulator. Dumping them independently doubled the
    /// records of every hierarchical design and showed one net as two signals.
    ///
    /// Resolves alias CHAINS (a port bound to a port bound to a port) up to the
    /// outermost net that is itself in the dump. A chain stops early at a name
    /// the current `$dumpvars` scope/depth filter did not select (so
    /// `$dumpvars(0, top.u_sub)` still dumps `u_sub.din` under its own code), at
    /// a width change, and at a `real`/`event` boundary — a shared code may only
    /// carry one width and one record shape.
    pub(super) fn dump_backing_ids(&self, names: &[String]) -> HashMap<String, usize> {
        let selected: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
        let tbl_id = |n: &str| -> Option<usize> {
            match self.signal_name_to_id.get(n) {
                Some(&i) if i < self.signal_table.len() => Some(i),
                _ => None,
            }
        };
        let mut out: HashMap<String, usize> = HashMap::default();
        for name in names {
            let own = match tbl_id(name.as_str()) {
                Some(i) => i,
                None => continue,
            };
            let mut rep_name: &str = name.as_str();
            let mut rep = own;
            // Bounded walk: a malformed alias cycle must not hang the dump.
            for _ in 0..64 {
                let parent = match self.module.port_aliases.get(rep_name) {
                    Some(p) => p.as_str(),
                    None => break,
                };
                if !selected.contains(parent) {
                    break;
                }
                let pid = match tbl_id(parent) {
                    Some(i) => i,
                    None => break,
                };
                if self.signal_widths[pid] != self.signal_widths[rep]
                    || self.lookup_signal_width(parent) != self.lookup_signal_width(rep_name)
                {
                    break;
                }
                if self.signal_real.get(pid).copied().unwrap_or(false)
                    != self.signal_real.get(rep).copied().unwrap_or(false)
                {
                    break;
                }
                // An `event` emits a pulse, not a level — never fold one into a
                // level signal (or into another event's pulse dedup).
                if self.module.events.contains(parent) || self.module.events.contains(rep_name) {
                    break;
                }
                rep = pid;
                rep_name = parent;
            }
            out.insert(name.clone(), rep);
        }
        out
    }

    /// §21.7.2.1 `var_type` of a dumped object. Every `$var` used to be
    /// hardcoded `wire`.
    pub(super) fn dump_var_kind(&self, name: &str, id: usize) -> VcdVarKind {
        if self.module.events.contains(name) {
            return VcdVarKind::Event;
        }
        if self.signal_real.get(id).copied().unwrap_or(false) {
            return VcdVarKind::Real;
        }
        // An array element (`mem[0]`) inherits its base's declared type.
        let base = match name.find('[') {
            Some(p) => &name[..p],
            None => name,
        };
        match self.module.var_decl_types.get(base) {
            Some(DataType::Real { .. }) => return VcdVarKind::Real,
            Some(DataType::IntegerAtom {
                kind: IntegerAtomType::Integer,
                ..
            }) => return VcdVarKind::Integer,
            Some(DataType::IntegerAtom {
                kind: IntegerAtomType::Time,
                ..
            }) => return VcdVarKind::Time,
            Some(DataType::Simple {
                kind: crate::ast::types::SimpleType::Event,
                ..
            }) => return VcdVarKind::Event,
            _ => {}
        }
        if self.module.nets.contains(base) {
            return VcdVarKind::Wire;
        }
        // §21.7.2.1: a variable with no PROCEDURAL driver — driven only by a
        // continuous assign, or by an instance output port (which inlining
        // lowers to one) — is a net to a viewer, and a reference simulator types it `wire`.
        // §6.5 makes continuous and procedural drivers mutually exclusive on a
        // variable, so a continuous driver is proof there is no procedural one.
        // Typing these `reg` made GTKWave colour a driven net as a register.
        if self.cont_driven.contains(&id) {
            return VcdVarKind::Wire;
        }
        // A collapsed child port shares the parent's id, so the id carries the
        // parent's kind; the child name itself is port-connect-driven → wire.
        if self.collapsed_port_children.contains(base) {
            return VcdVarKind::Wire;
        }
        if self.module.parameters.contains_key(base)
            && self.module.var_decl_types.get(base).is_none()
        {
            return VcdVarKind::Parameter;
        }
        VcdVarKind::Reg
    }

    /// §21.7.2.1 optional bit range on a `$var` reference
    /// (`$var wire 8 ( hi [15:8] $end`). Without it a `logic [15:8]` shows as
    /// `[7:0]` in a viewer, and an ascending `logic [0:7]` loses its bit order.
    ///
    /// An unpacked-array ELEMENT carries the range too — the index in its
    /// reference name selects the element, and the range then gives that
    /// element's own packed bit numbering, exactly as Verilator emits it:
    ///   `$var wire 8 ) mem[0] [7:0] $end`
    /// Suppressing it (the old behaviour) left every `mem[i]` un-ranged.
    pub(super) fn dump_var_range(&self, name: &str, width: u32) -> Option<(i64, i64)> {
        // A scalar has no range.
        if width <= 1 {
            return None;
        }
        let base = match name.find('[') {
            Some(p) => &name[..p],
            None => name,
        };
        let dims = match self.module.var_decl_types.get(base) {
            Some(DataType::IntegerVector { dimensions, .. })
            | Some(DataType::Implicit { dimensions, .. })
            | Some(DataType::TypeReference { dimensions, .. }) => Some(dimensions),
            _ => None,
        };
        if let Some(dims) = dims {
            if dims.len() == 1 {
                if let crate::ast::types::PackedDimension::Range { left, right, .. } = &dims[0] {
                    let params = Some(&self.module.parameters);
                    if let (Some(l), Some(r)) = (
                        super::elaborate::const_eval_i64_with_params(left, params),
                        super::elaborate::const_eval_i64_with_params(right, params),
                    ) {
                        if (l - r).unsigned_abs() + 1 == width as u64 {
                            return Some((l, r));
                        }
                    }
                }
            }
        }
        if self.module.ascending_packed.contains_key(base) {
            Some((0, width as i64 - 1))
        } else {
            Some((width as i64 - 1, 0))
        }
    }

    /// The value to dump for a signal-table id, with `is_real` forced from the
    /// declaration so a `real` always formats as an `r<decimal>` record even if
    /// the stored `Value` lost its flag along the way.
    pub(super) fn dump_value(&self, id: usize) -> Value {
        let mut v = self.signal_table[id].clone();
        if self.signal_real.get(id).copied().unwrap_or(false) {
            v.is_real = true;
        }
        v
    }

    /// `$date` content. The header used to carry no date at all.
    pub(super) fn vcd_date_string() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (days, tod) = ((secs / 86_400) as i64, secs % 86_400);
        // civil_from_days (Howard Hinnant's algorithm), no calendar crate needed.
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            y,
            m,
            d,
            tod / 3600,
            (tod % 3600) / 60,
            tod % 60
        )
    }

    /// Start VCD dumping: open file, write header, record initial values
    pub(super) fn vcd_start_dump(&mut self) {
        // §21.7.1.2: a repeat `$dumpvars` must NOT re-create the file. The old
        // sink's background thread is still draining into the same path, so a
        // second `File::create` interleaves two byte streams into one file and
        // corrupts it. A second call is a checkpoint of the running dump.
        if self.vcd_writer.is_some() {
            self.vcd_dump_all();
            return;
        }
        self.sync_table_to_hashmap();
        let filename = self
            .vcd_file
            .clone()
            .unwrap_or_else(|| "dump.vcd".to_string());
        let file = match std::fs::File::create(&filename) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Warning: cannot create VCD file '{}': {}", filename, e);
                return;
            }
        };
        // A '.zst'/'.zstd' filename suffix zstd-compresses the byte stream
        // (same convention as the XTrace dump).
        let zstd_level = if filename.ends_with(".zst") || filename.ends_with(".zstd") {
            Some(3)
        } else {
            None
        };
        let mut w = match super::vcd_sink::VcdSink::open_file(
            file,
            self.dump_writer_threaded(),
            zstd_level,
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "Warning: cannot create VCD writer for '{}': {}",
                    filename, e
                );
                return;
            }
        };

        let sig_names = self.dump_signal_names(&self.vcd_filter_scopes, self.vcd_dump_depth);
        eprintln!(
            "[VCD] dumping {} signals (filter_scopes={:?}, depth={})",
            sig_names.len(),
            self.vcd_filter_scopes,
            self.vcd_dump_depth
        );

        // Assign VCD identifier codes. Codes are Arc<str> so the per-change
        // clone in vcd_write_changes is a refcount bump, not a heap alloc.
        //
        // §21.7.2.1: "several variables can be mapped to the same identifier
        // code if the variables would always have identical values" — which is
        // exactly a net threaded through instance ports. Names that share a
        // backing signal (see `dump_backing_ids`) therefore share ONE code: each
        // still gets its own `$var` line in its own `$scope`, but the net emits a
        // single value-change record instead of one per hierarchical name.
        let backing = self.dump_backing_ids(&sig_names);
        let mut id_map: HashMap<String, Arc<str>> = HashMap::default();
        let mut code_of_backing: HashMap<usize, Arc<str>> = HashMap::default();
        for name in sig_names.iter() {
            let rep = match backing.get(name.as_str()) {
                Some(&r) => r,
                // No backing signal — never declared, never dumped.
                None => continue,
            };
            let code = match code_of_backing.get(&rep) {
                Some(c) => c.clone(),
                None => {
                    let c: Arc<str> = Arc::from(Self::vcd_id_code(code_of_backing.len()));
                    code_of_backing.insert(rep, c.clone());
                    c
                }
            };
            id_map.insert(name.clone(), code);
        }

        // Write VCD header. The timescale is derived from the sim's actual
        // finest precision (tick_s), not hardcoded — a fixed "1ns" mislabels
        // designs with finer precision (e.g. c910 runs at 100ps).
        let mut hdr: Vec<u8> = Vec::with_capacity(4096);
        let _ = writeln!(hdr, "$date\n  {}\n$end", Self::vcd_date_string());
        let _ = writeln!(hdr, "$version\n  xezim {}\n$end", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(
            hdr,
            "$timescale\n  {}\n$end",
            Self::xtrace_timescale_str(self.module.tick_s)
        );

        // Build hierarchical signal tree from dotted names.
        // Signal "uut.cpu.reg_op1" → hierarchy ["uut", "cpu"], leaf "reg_op1"
        // Signal "clk" → hierarchy [], leaf "clk"
        // Signal "uut.cpuregs[5]" → hierarchy ["uut"], leaf "cpuregs[5]"
        use std::collections::BTreeMap;
        struct VcdVar {
            leaf: String,
            width: u32,
            id: Arc<str>,
            kind: VcdVarKind,
            range: Option<(i64, i64)>,
        }
        struct ScopeNode {
            children: BTreeMap<String, ScopeNode>,
            signals: Vec<VcdVar>,
        }
        impl ScopeNode {
            fn new() -> Self {
                ScopeNode {
                    children: BTreeMap::new(),
                    signals: Vec::new(),
                }
            }
        }

        let vcd_param_as_wire =
            std::env::var("XEZIM_VCD_PARAM_AS_WIRE").ok().as_deref() == Some("1");
        let mut root = ScopeNode::new();
        for name in &sig_names {
            let tbl_id = match self.signal_name_to_id.get(name.as_str()) {
                Some(&i) if i < self.signal_table.len() => i,
                _ => continue,
            };
            let width = self.lookup_signal_width(name).unwrap_or(1);
            let mut kind = self.dump_var_kind(name, tbl_id);
            // Parameters are dumped LRM-compliantly as `$var parameter` — but
            // some waveform viewers shelve those separately and never show
            // them in the waveform pane. `XEZIM_VCD_PARAM_AS_WIRE=1` emits them
            // as a constant-valued `wire` instead, so they trace like any other
            // signal (matching what the FST/FSDB path already does).
            if vcd_param_as_wire && matches!(kind, VcdVarKind::Parameter) {
                kind = VcdVarKind::Wire;
            }
            let range = match kind {
                // A `real` is always declared 64 bits wide with no range, and an
                // `event` is a 1-bit pulse.
                VcdVarKind::Real | VcdVarKind::Event => None,
                _ => self.dump_var_range(name, width),
            };
            let id = id_map[name].clone();
            // Split into hierarchy parts
            let parts: Vec<&str> = name.split('.').collect();
            let (scope_parts, leaf) = if parts.len() > 1 {
                (&parts[..parts.len() - 1], parts[parts.len() - 1])
            } else {
                (&[][..], parts[0])
            };
            // Navigate/create scope tree
            let mut node = &mut root;
            for part in scope_parts {
                node = node
                    .children
                    .entry(part.to_string())
                    .or_insert_with(ScopeNode::new);
            }
            node.signals.push(VcdVar {
                leaf: leaf.to_string(),
                width,
                id,
                kind,
                range,
            });
        }

        // §21.7.2.1 declaration of one object.
        fn emit_var(w: &mut impl Write, v: &VcdVar) {
            match v.kind {
                VcdVarKind::Real => {
                    let _ = writeln!(w, "$var real 64 {} {} $end", v.id, v.leaf);
                }
                VcdVarKind::Event => {
                    let _ = writeln!(w, "$var event 1 {} {} $end", v.id, v.leaf);
                }
                k => match v.range {
                    Some((l, r)) => {
                        let _ = writeln!(
                            w,
                            "$var {} {} {} {} [{}:{}] $end",
                            k.keyword(),
                            v.width,
                            v.id,
                            v.leaf,
                            l,
                            r
                        );
                    }
                    None => {
                        let _ = writeln!(
                            w,
                            "$var {} {} {} {} $end",
                            k.keyword(),
                            v.width,
                            v.id,
                            v.leaf
                        );
                    }
                },
            }
        }

        // Emit VCD scopes recursively
        fn emit_scope(w: &mut impl Write, name: &str, node: &ScopeNode) {
            let _ = writeln!(w, "$scope module {} $end", name);
            for v in &node.signals {
                emit_var(w, v);
            }
            for (child_name, child_node) in &node.children {
                emit_scope(w, child_name, child_node);
            }
            let _ = writeln!(w, "$upscope $end");
        }

        if self.root_is_multi_top() {
            // Each top is a root scope of its own, as it is standing alone;
            // package and compilation-unit variables go under `$unit`.
            if !root.signals.is_empty() {
                let _ = writeln!(hdr, "$scope module $unit $end");
                for v in &root.signals {
                    emit_var(&mut hdr, v);
                }
                let _ = writeln!(hdr, "$upscope $end");
            }
            for (child_name, child_node) in &root.children {
                emit_scope(&mut hdr, child_name, child_node);
            }
        } else {
            let _ = writeln!(hdr, "$scope module {} $end", self.module.name);
            for v in &root.signals {
                emit_var(&mut hdr, v);
            }
            for (child_name, child_node) in &root.children {
                emit_scope(&mut hdr, child_name, child_node);
            }
            let _ = writeln!(hdr, "$upscope $end");
        }
        let _ = writeln!(hdr, "$enddefinitions $end");

        // Build the compact per-cycle trace table: (signal_table index, code).
        // Initial values are written below and seeded into vcd_prev_signals so
        // the first vcd_write_changes only emits actual transitions.
        let mut trace: Vec<(usize, Arc<str>)> = Vec::with_capacity(sig_names.len());
        let mut kinds: Vec<VcdVarKind> = Vec::with_capacity(sig_names.len());
        let mut prev: Vec<Value> = Vec::with_capacity(sig_names.len());

        // §21.7.1.4: the `$dumpvars` checkpoint is the state at the CURRENT sim
        // time. Without the `#t` marker a dump started mid-run (e.g. at t=23) is
        // placed at t=0 by every viewer.
        let _ = writeln!(hdr, "#{}", self.time);
        let _ = writeln!(hdr, "$dumpvars");
        // One trace entry — and one checkpoint record — per BACKING signal, not
        // per name: the aliased names of a port-connected net share a code, so a
        // per-name loop would emit the same record two or three times over.
        let mut traced: HashSet<usize> = HashSet::default();
        for name in &sig_names {
            let sid = match backing.get(name.as_str()) {
                Some(&i) => i,
                _ => continue,
            };
            if !traced.insert(sid) {
                continue;
            }
            let kind = self.dump_var_kind(name, sid);
            let code = id_map[name].clone();
            // An `event` has no level, so it contributes no checkpoint record —
            // it only ever emits a `1<id>` pulse when triggered.
            if kind != VcdVarKind::Event {
                let val = self.dump_value(sid);
                super::vcd_sink::write_vcd_value(&mut hdr, &val, &code);
            }
            prev.push(self.signal_table[sid].clone());
            kinds.push(kind);
            trace.push((sid, code));
        }
        let _ = writeln!(hdr, "$end");

        let nbytes = hdr.len() as u64;
        let _ = w.write_all(&hdr);

        self.vcd_event_last = vec![u64::MAX; trace.len()];
        self.vcd_trace = trace;
        self.vcd_var_kinds = kinds;
        self.vcd_prev_signals = prev;
        self.vcd_writer = Some(w);
        self.vcd_enabled = true;
        self.vcd_bytes = 0;
        self.vcd_limit_hit = false;
        self.vcd_last_time = self.time;
        self.vcd_account_bytes(nbytes);

        // Build the incremental-dump reverse maps (id → trace slots, and the
        // event slots) unless the full scan is forced. When active, every
        // signal write marks the id dirty so vcd_write_changes visits only
        // written signals; see the field docs on `dump_dirty`.
        let force_full = std::env::var("XEZIM_VCD_FULL").ok().as_deref() == Some("1");
        if force_full {
            self.dump_dirty_active = false;
            self.dump_full = true;
        } else {
            let nsig = self.signal_table.len();
            let mut id_to_trace: Vec<Vec<u32>> = vec![Vec::new(); nsig];
            let mut event_indices: Vec<usize> = Vec::new();
            for (idx, &(sid, _)) in self.vcd_trace.iter().enumerate() {
                if self.vcd_var_kinds[idx] == VcdVarKind::Event {
                    event_indices.push(idx);
                }
                if sid < nsig {
                    id_to_trace[sid].push(idx as u32);
                }
            }
            self.vcd_id_to_trace = id_to_trace;
            self.vcd_event_indices = event_indices;
            self.enable_dump_dirty_tracking();
        }
    }

    /// Arm the shared dump dirty set (see the `dump_dirty` field docs). Called
    /// by every writer that wants incremental change detection; idempotent, so
    /// a run dumping VCD + XTrace + FST together shares one mark array and one
    /// dirty list. `XEZIM_DUMP_FULL=1` forces every writer back to a full scan.
    ///
    /// A writer that starts MID-RUN is still correct: it seeds its own
    /// `*_prev_signals` from the current signal table when it opens, so it only
    /// needs to see writes from that point on, which is exactly what the dirty
    /// set carries from the next flush onward.
    pub(super) fn enable_dump_dirty_tracking(&mut self) {
        if std::env::var("XEZIM_DUMP_FULL").ok().as_deref() == Some("1") {
            self.dump_dirty_active = false;
            self.dump_full = true;
            return;
        }
        if self.dump_dirty_active {
            return;
        }
        let nsig = self.signal_table.len();
        self.dump_dirty_mark = vec![0u64; nsig];
        self.dump_dirty.clear();
        self.dump_dirty_epoch = 1;
        self.dump_dirty_active = true;
    }

    /// Build a signal-id → trace-slot reverse map for a writer's trace table.
    /// A signal can occupy several slots (aliases dumped under more than one
    /// hierarchical name), so slots are collected per id rather than assumed 1:1.
    pub(super) fn build_dump_id_to_trace<T>(&self, trace: &[(usize, T)]) -> Vec<Vec<u32>> {
        let nsig = self.signal_table.len();
        let mut id_to_trace: Vec<Vec<u32>> = vec![Vec::new(); nsig];
        for (idx, (sid, _)) in trace.iter().enumerate() {
            if *sid < nsig {
                id_to_trace[*sid].push(idx as u32);
            }
        }
        id_to_trace
    }

    /// The trace slots a writer must re-check this flush: the slots of every
    /// signal written since the last flush, sorted and deduped so record order
    /// within a timestep is byte-identical to the full scan. `None` means the
    /// caller must fall back to scanning every slot.
    pub(super) fn dump_dirty_slots(&self, id_to_trace: &[Vec<u32>]) -> Option<Vec<u32>> {
        if !self.dump_dirty_active || id_to_trace.is_empty() {
            return None;
        }
        let mut slots: Vec<u32> = Vec::with_capacity(self.dump_dirty.len());
        for &sid in &self.dump_dirty {
            let sid = sid as usize;
            if sid < id_to_trace.len() {
                slots.extend_from_slice(&id_to_trace[sid]);
            }
        }
        slots.sort_unstable();
        slots.dedup();
        Some(slots)
    }

    /// Flush every active dump writer for this time slot, then retire the
    /// shared dirty set. The writers only READ `dump_dirty` (they each need the
    /// same list), so the clear + epoch bump happens exactly once, here.
    pub(super) fn dump_write_changes(&mut self) {
        self.vcd_write_changes();
        if self.xtrace_writer.is_some() {
            self.xtrace_write_changes();
        }
        if self.fst_writer.is_some() {
            self.fst_write_changes();
        }
        if self.code_cov.is_some() {
            self.cov_toggle_sample();
        }
        if self.dump_dirty_active {
            self.dump_dirty.clear();
            self.dump_dirty_epoch = self.dump_dirty_epoch.wrapping_add(1);
        }
    }

    /// End of a time slot, before time moves: flush what was written after
    /// the slot's last tick. The end-of-slot VPI callbacks (§38.36.2: a
    /// leftover cbReadWriteSynch, cbAtEndOfSimTime) run after that tick's
    /// flush, and cocotb applies its writes from cbReadWriteSynch, so without
    /// this a value written there reached the dumps and toggle coverage only
    /// at the next slot, merged with that slot's changes: a reset cocotb
    /// wrote at time 0 was never a toggle baseline, so its release did not
    /// count. Under a forced full scan there is no dirty set to tell, so
    /// every slot flushes.
    #[inline]
    pub(super) fn dump_slot_end(&mut self) {
        let due = if self.dump_dirty_active {
            !self.dump_dirty.is_empty()
        } else {
            self.dump_full
        };
        if due {
            self.dump_write_changes();
        }
    }

    // NOTE: `Simulator::vcd_write_value` is GONE. It was a second, divergent
    // copy of `vcd_sink::write_vcd_value` (the two disagreed about `real` and
    // about leading-zero suppression, so a value's spelling depended on whether
    // it went out through the header path or the sink's writer thread). There is
    // now exactly one VCD value formatter, in `vcd_sink`, and every path uses it.

    /// Byte length of the value-change record the sink will format for `val`.
    /// The sink formats batched changes on its own thread, so `$dumplimit`
    /// sizes them here instead of materializing them.
    pub(super) fn vcd_record_len(val: &Value, code: &str) -> u64 {
        let body = if val.is_real {
            22 // "r" + decimal + " "
        } else if val.width == 1 {
            1
        } else {
            2 + val.width as u64 // "b" + bits + " "
        };
        body + code.len() as u64 + 1 // + '\n'
    }

    /// §21.7.1.8 `$dumplimit(n)`: stop dumping once the file reaches `n` bytes.
    pub(super) fn vcd_account_bytes(&mut self, n: u64) {
        if self.vcd_limit_hit {
            return;
        }
        self.vcd_bytes += n;
        let limit = match self.vcd_limit {
            Some(l) => l,
            None => return,
        };
        if self.vcd_bytes >= limit {
            self.vcd_limit_hit = true;
            if let Some(w) = self.vcd_writer.as_mut() {
                let _ = writeln!(
                    w,
                    "$comment $dumplimit of {} bytes reached — dumping stopped $end",
                    limit
                );
                let _ = w.flush();
            }
        }
    }

    /// Hand a fully formatted record block to the sink and bill it to the limit.
    pub(super) fn vcd_emit(&mut self, buf: Vec<u8>) {
        if self.vcd_limit_hit {
            return;
        }
        if let Some(w) = self.vcd_writer.as_mut() {
            let _ = w.write_all(&buf);
        }
        self.vcd_account_bytes(buf.len() as u64);
    }

    /// Write VCD value changes for the current timestep
    pub(super) fn vcd_write_changes(&mut self) {
        if !self.vcd_enabled || self.vcd_writer.is_none() || self.vcd_limit_hit {
            return;
        }

        // vcd_prev_signals is a parallel Vec<Value> (index == position in
        // vcd_trace), so change detection is a single indexed compare.
        let now = self.time;
        let mut changes: Vec<(Arc<str>, Value)> = Vec::new();

        // Per-trace-slot change check, shared by the incremental and full paths.
        // Returns the record to emit, if any, and keeps vcd_prev_signals in step.
        macro_rules! check_slot {
            ($idx:expr_2021) => {{
                let idx = $idx;
                let id = self.vcd_trace[idx].0;
                if self.vcd_var_kinds[idx] == VcdVarKind::Event {
                    // §21.7.2.1: an event has no level — it emits a bare `1<id>`
                    // record at EVERY trigger. Treating it as a level signal
                    // with prev!=cur dedup drops a repeat `->ev` whose 0→1→0
                    // toggle cancels inside one time slot.
                    let fired = self
                        .name_opt(id)
                        .and_then(|n| self.event_triggered_time.get(n))
                        .copied()
                        == Some(now);
                    if fired && self.vcd_event_last[idx] != now {
                        self.vcd_event_last[idx] = now;
                        changes.push((self.vcd_trace[idx].1.clone(), Value::ones(1)));
                    }
                    // Keep the level mirror in step so the toggle never leaks out.
                    self.vcd_prev_signals[idx] = self.signal_table[id].clone();
                } else {
                    let val = &self.signal_table[id];
                    if self.vcd_prev_signals[idx] != *val {
                        let mut out = val.clone();
                        if self.signal_real.get(id).copied().unwrap_or(false) {
                            out.is_real = true;
                        }
                        self.vcd_prev_signals[idx] = val.clone();
                        changes.push((self.vcd_trace[idx].1.clone(), out));
                    }
                }
            }};
        }

        // Incremental: only signals written since the last flush can have
        // changed. Gather their trace slots plus the (few) event slots (events
        // fire independently of signal writes), then emit in trace-index order
        // so the per-timestep record order is byte-identical to the full scan.
        // Still O(written signals), not O(all). The dirty list is retired by
        // `dump_write_changes` once every writer has read it.
        let dirty_slots = self.dump_dirty_slots(&self.vcd_id_to_trace);
        if let Some(mut slots) = dirty_slots {
            if !self.vcd_event_indices.is_empty() {
                slots.extend(self.vcd_event_indices.iter().map(|&i| i as u32));
                slots.sort_unstable();
                slots.dedup();
            }
            for &idx in &slots {
                check_slot!(idx as usize);
            }
        } else {
            // Full scan: walk every dumped trace slot.
            for idx in 0..self.vcd_trace.len() {
                check_slot!(idx);
            }
        }

        if changes.is_empty() {
            return;
        }

        let time_marker = if self.time != self.vcd_last_time {
            self.vcd_last_time = self.time;
            Some(self.time)
        } else {
            None
        };

        let mut nbytes = if time_marker.is_some() { 12 } else { 0 };
        for (code, val) in &changes {
            nbytes += Self::vcd_record_len(val, code);
        }

        if let Some(sink) = self.vcd_writer.as_mut() {
            sink.post_vcd_changes(time_marker, changes);
        }
        self.vcd_account_bytes(nbytes);
    }

    /// Emit a `$dumpall` / `$dumpon` checkpoint (§21.7.1.5, §21.7.1.7): the
    /// CURRENT value of every dumped variable, bracketed by the keyword and
    /// `$end` and stamped with the current time.
    pub(super) fn vcd_checkpoint(&mut self, keyword: &str) {
        if self.vcd_writer.is_none() || self.vcd_limit_hit {
            return;
        }
        let mut buf: Vec<u8> = Vec::new();
        let _ = writeln!(buf, "#{}", self.time);
        let _ = writeln!(buf, "{}", keyword);
        for idx in 0..self.vcd_trace.len() {
            // An event has no level to check-point.
            if self.vcd_var_kinds[idx] == VcdVarKind::Event {
                continue;
            }
            let sid = self.vcd_trace[idx].0;
            let val = self.dump_value(sid);
            let code = self.vcd_trace[idx].1.clone();
            super::vcd_sink::write_vcd_value(&mut buf, &val, &code);
            self.vcd_prev_signals[idx] = self.signal_table[sid].clone();
        }
        let _ = writeln!(buf, "$end");
        self.vcd_last_time = self.time;
        self.vcd_emit(buf);
    }

    /// §21.7.1.5 `$dumpall` — checkpoint every dumped variable.
    pub(super) fn vcd_dump_all(&mut self) {
        if !self.vcd_enabled {
            return;
        }
        self.vcd_write_changes();
        self.vcd_checkpoint("$dumpall");
    }

    /// §21.7.1.6 `$dumpoff` — suspend dumping. The suspended window must be
    /// marked: every dumped variable goes to x (a `real` to z), so a viewer
    /// paints the window as unknown instead of holding a stale, false level.
    /// Merely flipping a bool (the old behaviour) leaves the last value on
    /// screen across the whole off-window.
    pub(super) fn vcd_dump_off(&mut self) {
        if self.vcd_writer.is_none() || !self.vcd_enabled || self.vcd_limit_hit {
            return;
        }
        // Flush anything that changed at this time before freezing.
        self.vcd_write_changes();
        let mut buf: Vec<u8> = Vec::new();
        let _ = writeln!(buf, "#{}", self.time);
        let _ = writeln!(buf, "$dumpoff");
        for idx in 0..self.vcd_trace.len() {
            let code = self.vcd_trace[idx].1.clone();
            match self.vcd_var_kinds[idx] {
                VcdVarKind::Event => {}
                VcdVarKind::Real => {
                    let _ = writeln!(buf, "z{}", code);
                }
                _ => {
                    let width = self.signal_table[self.vcd_trace[idx].0].width;
                    if width == 1 {
                        let _ = writeln!(buf, "x{}", code);
                    } else {
                        let _ = writeln!(buf, "bx {}", code);
                    }
                }
            }
        }
        let _ = writeln!(buf, "$end");
        self.vcd_last_time = self.time;
        self.vcd_emit(buf);
        self.vcd_enabled = false;
    }

    /// §21.7.1.7 `$dumpon` — resume dumping, re-stating the current value of
    /// every dumped variable so the viewer recovers from the x window.
    pub(super) fn vcd_dump_on(&mut self) {
        if self.vcd_writer.is_none() || self.vcd_enabled || self.vcd_limit_hit {
            return;
        }
        self.vcd_enabled = true;
        self.vcd_checkpoint("$dumpon");
    }

    /// Flush and close VCD file
    pub(super) fn vcd_finish(&mut self) {
        if self.vcd_writer.is_some() {
            self.vcd_write_changes();
            // Close the wave window at the final simulation time: without a
            // trailing `#t` a viewer stops at the last TRANSITION, so the tail
            // of the run is missing from the waveform.
            if self.time > self.vcd_last_time && !self.vcd_limit_hit {
                let t = self.time;
                self.vcd_emit(format!("#{}\n", t).into_bytes());
                self.vcd_last_time = t;
            }
        }
        if let Some(ref mut w) = self.vcd_writer {
            let _ = w.flush();
        }
        self.vcd_writer = None;
    }

    // ═══════════════════════════════════════════════════════════════
    // XTrace dump support (XTrace_Specification_v1_0.txt)
    // ═══════════════════════════════════════════════════════════════
    //
    // CONFORMANCE LEVEL 0 (§24 "minimal"): dictionary (§9) + time (§10.1) +
    // signal deltas (§10.2/§10.3), and nothing else — with ONE exception, the
    // §10.4 `X` event record. An SV `event` has no level, so no D/P record can
    // represent it (`->e` three times in a row is three events, not a toggling
    // bit); §10.4/§19.5 define exactly the record for it, and `@capabilities`
    // declares it.
    //
    // The semantic layer is deliberately ABSENT: no transactions (Q/TQ), no
    // assertions (A), no enums (E), no source maps (F/SL), no memory records
    // (MW/MR/MB), no context snapshots (N,ctx). The header says so honestly —
    // `@profile minimal`, `@capabilities signal_delta|events` — rather than
    // claiming the Appendix-A `xezim_ai_debug` profile we do not implement.

    /// Render the dump timescale string (e.g. "1ns", "100ps") from `tick_s`,
    /// the seconds-per-tick of the simulator's finest timescale precision.
    /// `T,+delta` records count these ticks, so the header MUST match — a
    /// hardcoded "1ns" mislabels any design with finer precision.
    pub(super) fn xtrace_timescale_str(tick_s: f64) -> String {
        // `tick_s` is a power of ten (10^precision_exp seconds).
        let exp = tick_s.log10().round() as i32;
        // (base-10 exponent, unit). SystemVerilog units step by 1000×.
        const UNITS: [(i32, &str); 6] = [
            (0, "s"),
            (-3, "ms"),
            (-6, "us"),
            (-9, "ns"),
            (-12, "ps"),
            (-15, "fs"),
        ];
        for &(base, unit) in &UNITS {
            if exp >= base {
                // exp-base is 0/1/2 for 1/10/100 of the unit.
                let value = 10i64.pow((exp - base) as u32);
                return format!("{}{}", value, unit);
            }
        }
        // Finer than 1fs (shouldn't happen): clamp to the smallest unit.
        "1fs".to_string()
    }

    /// XTrace signal ids, in the compact spelling the 0.1.2 producer used (and
    /// the reference traces carry): `a`..`z`, `A`..`Z`, `0`..`9`, then `ab`,
    /// `bb`, ... A dictionary of a few thousand nets stays at 1-2 characters,
    /// and every D/P record pays for it on every line.
    pub(super) fn xtrace_id_code(mut idx: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let base = ALPHABET.len();
        let mut code = String::new();
        loop {
            code.push(ALPHABET[idx % base] as char);
            idx /= base;
            if idx == 0 {
                break;
            }
            idx -= 1;
        }
        code
    }

    /// Format a `Value` as an XTrace value token (§15).
    ///
    /// * `real` (`is_real`): a DECIMAL number — the same spelling the VCD path
    ///   uses (`vcd_sink::vcd_real_string`). §9.3's type list is *recommended*,
    ///   not exhaustive, so a `real` signal type carrying a decimal value is a
    ///   legitimate producer choice; §15.1 explicitly allows decimal "where
    ///   semantically better". The old code never checked `is_real` and emitted
    ///   the raw IEEE-754 bit pattern as an integer, so `real r = 3.25` came out
    ///   as `0x400a000000000000` — which a consumer decodes as the integer
    ///   -4609434218613702656.
    /// * fully-known: `0x<hex>`.
    /// * all-x / all-z: the compact `X` / `Z` forms of §15.3. A 128-bit all-z
    ///   net then costs 1 byte on the line instead of 130; 4-state designs sit
    ///   at x/z for most of reset, and full-width binary made XTrace several
    ///   times LARGER than the equivalent VCD.
    /// * mixed x/z: FULL-WIDTH `0b<bits>`, every bit spelled out. VCD's
    ///   leading-run suppression is deliberately NOT copied: it is legal there
    ///   only because §21.7.2.1 defines a left-EXTENSION rule for a value
    ///   shorter than its `$var` width. XTrace defines no such rule anywhere,
    ///   so a partially collapsed vector would be unparseable (nothing tells a
    ///   consumer how many bits were dropped, or what to refill them with).
    ///
    /// The implementation lives in `vcd_sink` because the XTrace writer THREAD
    /// calls it: rendering values to ASCII is the bulk of a dump's cost and the
    /// background writer exists to absorb it. This wrapper keeps the remaining
    /// simulator-side call sites (the `N,full` snapshot) reading naturally.
    #[inline]
    pub(super) fn xtrace_format_value(val: &Value, is_real: bool, is_string: bool) -> String {
        super::vcd_sink::xtrace_format_value(val, is_real, is_string)
    }

    /// Open the XTrace file and emit the §6 header + `@section dict` + the
    /// initial state snapshot. Called once at the start of `run()` when
    /// `xtrace_file` is set.
    pub(super) fn xtrace_start_dump(&mut self) {
        // Signal count above which an unscoped (whole-design) dump is flagged.
        const XTRACE_UNSCOPED_WARN: usize = 100_000;
        let requested = match self.xtrace_file.clone() {
            Some(f) => f,
            None => return,
        };
        self.sync_table_to_hashmap();
        // Convert the ns time-window (#8) to ticks (same scale as max_time).
        let scale = (1e-9_f64 / self.module.tick_s).round().max(1.0);
        self.xtrace_from_t = (self.xtrace_from_ns as f64 * scale) as u64;
        self.xtrace_to_t = if self.xtrace_to_ns == u64::MAX {
            u64::MAX
        } else {
            (self.xtrace_to_ns as f64 * scale) as u64
        };

        // §6.8 `@compression`: zstd is selected by a `.zst`/`.zstd` suffix or
        // explicitly by `--xtrace-compress zstd` (which then names the file
        // `.zst` so the transport is self-describing). The header MUST declare
        // it — a file that is silently zstd-framed while its own header says
        // `none` is a conformance bug a consumer cannot recover from.
        let opts = xtrace_options();
        let suffix_zstd = requested.ends_with(".zst") || requested.ends_with(".zstd");
        let compress_zstd = suffix_zstd || opts.compress.as_deref() == Some("zstd");
        let filename = if compress_zstd && !suffix_zstd {
            format!("{}.zst", requested)
        } else {
            requested
        };
        let profile = opts.profile.as_deref().unwrap_or("minimal").to_string();

        let file = match std::fs::File::create(&filename) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Warning: cannot create XTrace file '{}': {}", filename, e);
                return;
            }
        };
        let zstd_level = if compress_zstd { Some(3) } else { None };
        let mut w = match super::vcd_sink::VcdSink::open_file(
            file,
            self.dump_writer_threaded(),
            zstd_level,
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "Warning: cannot create XTrace writer for '{}': {}",
                    filename, e
                );
                return;
            }
        };

        // Collect signal names to trace. Shares `$dumpvars`'s enumeration: the
        // REAL signal table (not the lazily-synced `self.signals` mirror, which
        // is empty unless something dirtied the table first) and the same
        // top-relative scope normalization, so `--xtrace-scope top.u_sub` and
        // `--xtrace-scope u_sub` both match. Sorted for deterministic IDs.
        let sig_names: Vec<String> = self.dump_signal_names(&self.xtrace_scopes, 0);
        if !self.xtrace_scopes.is_empty() {
            eprintln!(
                "[XTrace] dumping {} signals (scopes={})",
                sig_names.len(),
                self.xtrace_scopes.len()
            );
        } else if sig_names.len() > XTRACE_UNSCOPED_WARN {
            // No scope filter on a large design: a full dump can run to many
            // GB and minutes of I/O. Warn loudly rather than silently churn.
            eprintln!(
                "[XTrace] WARNING: dumping ALL {} signals (no --xtrace-scope given) — \
                 this may produce a very large file and slow the run. \
                 Pass --xtrace-scope <hier> to restrict the dump.",
                sig_names.len()
            );
        }

        // Module hierarchy (M records): top module is m0; sub-scopes are
        // discovered from dotted signal names in the SAME pass that assigns
        // signal IDs below — each signal's parent path is built exactly once.
        // Under the multi-top wrapper each top is a root path of its own and
        // m0 holds only package / compilation-unit signals.
        let multi_top = self.root_is_multi_top();
        let top_name: &str = if multi_top { "" } else { &self.module.name };
        let mut modules: Vec<(String, String)> = Vec::new();
        let mut module_map: HashMap<String, String> = HashMap::default();
        let top_path = if multi_top {
            "/$unit".to_string()
        } else {
            format!("/{}", top_name)
        };
        modules.push(("m0".to_string(), top_path.clone()));
        module_map.insert(top_path.clone(), "m0".to_string());

        // Port-connected nets resolve to ONE backing signal (see
        // `dump_backing_ids`): `.din(src_bus)` and `src_bus` are one net.
        let xt_backing = self.dump_backing_ids(&sig_names);

        // §19.2: "once introduced, an ID retains its meaning" — so two names may
        // NOT share one signal_id, as the old code had them do (a port-connected
        // net emitted TWO `S` records carrying the SAME id; a parser keyed on the
        // id clobbered one of them). Every NAME gets its own id here, and the
        // names that resolve to the same backing net carry §9.2's
        // `alias=<canonical_signal_id>` instead. Only the canonical id emits
        // value deltas, so the record count is unchanged.
        //
        // The canonical name is the one that IS the backing net (`src_bus`, not
        // `u_sub.din`); when the backing net itself is outside the dump (scope
        // filter), the first name in sorted order stands in for it.
        let mut canon_name: HashMap<usize, String> = HashMap::default();
        for name in sig_names.iter() {
            let Some(&tbl_id) = xt_backing.get(name.as_str()) else {
                continue;
            };
            if self.signal_name_to_id.get(name.as_str()) == Some(&tbl_id) {
                canon_name.entry(tbl_id).or_insert_with(|| name.clone());
            }
        }
        // Ids are assigned over the WHOLE dictionary first: an alias that sorts
        // before its canonical name still has to name it.
        let mut entries: Vec<(&str, usize, String)> = Vec::with_capacity(sig_names.len());
        for name in sig_names.iter() {
            let Some(&tbl_id) = xt_backing.get(name.as_str()) else {
                // Unresolved name: no backing signal to trace.
                continue;
            };
            canon_name.entry(tbl_id).or_insert_with(|| name.clone());
            let sid = Self::xtrace_id_code(entries.len());
            entries.push((name.as_str(), tbl_id, sid));
        }
        let mut canon_sid: HashMap<usize, String> = HashMap::default();
        for (name, tbl_id, sid) in &entries {
            if canon_name.get(tbl_id).map(|c| c.as_str()) == Some(*name) {
                canon_sid.insert(*tbl_id, sid.clone());
            }
        }

        let mut signal_records: Vec<String> = Vec::with_capacity(entries.len());
        // The compact per-NET tables reused at t=0 and every step. Dictionary
        // ids are `Arc<str>` so posting a change to the writer thread clones a
        // refcount, not the string — this happens millions of times per dump.
        let mut trace: Vec<(usize, Arc<str>)> = Vec::new();
        let mut reals: Vec<bool> = Vec::new();
        let mut strings: Vec<bool> = Vec::new();
        let mut events: Vec<(usize, Arc<str>)> = Vec::new();
        for (name, tbl_id, sid) in &entries {
            // Walk the parent path once, creating M records on demand; the
            // deepest path component is this signal's module.
            let parts: Vec<&str> = name.split('.').collect();
            let (mod_id, leaf) = if parts.len() > 1 {
                let mut path = if multi_top {
                    String::new()
                } else {
                    format!("/{}", top_name)
                };
                let mut mid = "m0".to_string();
                for part in &parts[..parts.len() - 1] {
                    path.push('/');
                    path.push_str(part);
                    mid = if let Some(existing) = module_map.get(&path) {
                        existing.clone()
                    } else {
                        let m = format!("m{}", modules.len());
                        module_map.insert(path.clone(), m.clone());
                        modules.push((m.clone(), path.clone()));
                        m
                    };
                }
                (mid, parts[parts.len() - 1].to_string())
            } else {
                ("m0".to_string(), (*name).to_string())
            };

            // Kind is read from the name's OWN table entry (a real/event is
            // never folded into an alias group by `dump_backing_ids`).
            let own_id = self
                .signal_name_to_id
                .get(*name)
                .copied()
                .unwrap_or(*tbl_id);
            let kind = self.dump_var_kind(name, own_id);
            let is_event = kind == VcdVarKind::Event;
            let is_real = kind == VcdVarKind::Real;
            // A `string` signal is stored as a wide bit vector, so `dump_var_kind`
            // sees a plain vector; recognise it from the string set (keyed by
            // both the dotted name and its leaf, since the set may hold either).
            let leaf_name = name.rsplit('.').next().unwrap_or(name);
            let is_string = !is_event
                && !is_real
                && (self.string_signals.contains(*name) || self.string_signals.contains(leaf_name));
            let width = if is_real {
                64
            } else {
                self.lookup_signal_width(name).unwrap_or(1)
            };
            // §9.3's type list is *recommended*, not exhaustive. `real` and
            // `event` are producer types beyond it: a real carries a decimal
            // value (§15.1), an event carries no value at all and shows up only
            // as an §10.4 `X` record. Per §8.7/§19.10 a consumer that knows
            // neither still parses the record and simply sees a signal that
            // never changes. `str` (§9.3) carries a §15.4 quoted literal.
            let type_str = if is_event {
                "event".to_string()
            } else if is_real {
                "real".to_string()
            } else if is_string {
                "str".to_string()
            } else if width == 1 {
                "bit".to_string()
            } else if self.lookup_signal_signed(name) {
                format!("s{}", width)
            } else {
                format!("u{}", width)
            };

            // §9.2 attributes. `enc=` and `width=` were both missing: a consumer
            // could not tell how the values were encoded, and had to infer the
            // width from the type name (impossible for `bit`/`real`).
            let mut attrs = vec![
                if is_event {
                    "enc=event".to_string()
                } else {
                    "enc=delta".to_string()
                },
                format!("width={}", width),
            ];
            // A bit-range whose LSB is not 0 (`logic [15:8] hi`) loses its
            // offset in a bare `width=8` — the bits silently renumber to [7:0],
            // and an ascending `[0:7]` loses its bit ORDER as well. §9.2 defines
            // no range attribute, but §8.7 says consumers ignore (or preserve)
            // unknown attributes, so `range=<msb>:<lsb>` is the safe way to
            // carry it: a consumer that knows it renumbers correctly, one that
            // does not is no worse off than before. Emitted only when it says
            // something a plain `width=` does not (i.e. lsb != 0).
            if !is_real && !is_event && !is_string {
                if let Some((l, r)) = self.dump_var_range(name, width) {
                    if r != 0 {
                        attrs.push(format!("range={}:{}", l, r));
                    }
                }
            }
            let canonical = canon_sid.get(tbl_id) == Some(sid);
            if !canonical {
                attrs.push(format!("alias={}", canon_sid[tbl_id]));
            } else if is_event {
                events.push((*tbl_id, Arc::from(sid.as_str())));
            } else {
                trace.push((*tbl_id, Arc::from(sid.as_str())));
                reals.push(is_real);
                strings.push(is_string);
            }

            signal_records.push(format!(
                "S,{},{},{},{},{}",
                sid,
                mod_id,
                leaf,
                type_str,
                attrs.join(",")
            ));
        }

        // Header per §6. The §18 BNF orders `@producer` after `@profile`, but
        // the spec's own Appendix-A example (and every xezim-produced trace in
        // the wild, including the 0.1.2 reference dumps) puts it third; the
        // directives are name-keyed, so this is the interoperable spelling.
        let _ = writeln!(w, "@xtrace 1.1");
        let _ = writeln!(w, "@format text");
        let _ = writeln!(w, "@producer xezim {}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(
            w,
            "@timescale {}",
            Self::xtrace_timescale_str(self.module.tick_s)
        );
        let _ = writeln!(w, "@design {}", top_name);
        // §6.5: what we actually emit is Level 0, i.e. the `minimal` profile.
        // `--xtrace-profile` can relabel it (the 0.1.2 traces say `raw_delta`),
        // but the DEFAULT never over-claims.
        let _ = writeln!(w, "@profile {}", profile);
        // §6.7: declare only record families we really produce — signal deltas
        // (§10.2/§10.3) and events (§10.4). Not transactions, not assertions.
        let _ = writeln!(
            w,
            "@capabilities {}",
            if compress_zstd {
                "signal_delta|events|compression_zstd"
            } else {
                "signal_delta|events"
            }
        );
        let _ = writeln!(
            w,
            "@compression {}",
            if compress_zstd { "zstd" } else { "none" }
        );
        // §6.9: unknown records/attributes (our `range=`, `real`/`event` types)
        // must not fail a consumer.
        let _ = writeln!(w, "@extensions ignore_unknown");
        // §8.3 comment recording which signals this dump carries. The 0.1.2
        // producer wrote `# xtrace-signals debug` for a hardcoded leaf-name
        // heuristic (pc/irq/instr/...) that silently dropped everything else;
        // signal selection is `--xtrace-scope` now, so the comment reports THAT.
        if self.xtrace_scopes.is_empty() {
            let _ = writeln!(w, "# xtrace-signals all");
        } else {
            let _ = writeln!(w, "# xtrace-signals scope={}", self.xtrace_scopes.join("|"));
        }
        let _ = writeln!(w);

        // Dictionary section.
        let _ = writeln!(w, "@section dict");
        for (mid, path) in &modules {
            let _ = writeln!(w, "M,{},{}", mid, path);
        }
        for rec in &signal_records {
            let _ = writeln!(w, "{}", rec);
        }
        let _ = writeln!(w);

        // Trace section. The opening `T,+0` and `N,full` snapshot are emitted
        // LAZILY, on the first in-window `xtrace_write_changes` call — by then
        // the time-0 initial blocks have run and settled, so the checkpoint
        // carries real values (matching the 0.1.2 reference producer) instead
        // of the pre-init all-X image that a redundant same-time P then had to
        // correct. See `xtrace_emit_pending_snapshot`.
        let _ = writeln!(w, "@section trace");

        self.xtrace_event_last = vec![u64::MAX; events.len()];
        self.xtrace_events = events;
        self.xtrace_real = reals;
        self.xtrace_string = strings;
        // prev only needs the right LENGTH here; the pending snapshot re-reads
        // signal_table and reseeds it from settled values.
        self.xtrace_prev_signals = trace
            .iter()
            .map(|(id, _)| self.signal_table[*id].clone())
            .collect();
        // Incremental change detection: narrow each flush to the slots of
        // signals actually written (see the `dump_dirty` field docs). Shared
        // with the VCD and FST writers, so a run dumping several formats builds
        // one dirty list.
        self.xtrace_id_to_trace = self.build_dump_id_to_trace(&trace);
        self.enable_dump_dirty_tracking();
        self.xtrace_trace = trace;
        self.xtrace_writer = Some(w);
        self.xtrace_last_time = self.time;
        self.xtrace_snapshot_pending = true;
    }

    /// Emit the deferred t=0 (or window-start) `N,full` checkpoint from the
    /// SETTLED signal values, chunked per §A.13, and reseed `xtrace_prev_signals`
    /// so subsequent writes are pure deltas. Called once, from the first
    /// in-window `xtrace_write_changes`.
    pub(super) fn xtrace_emit_pending_snapshot(&mut self) {
        self.xtrace_snapshot_pending = false;
        let delta = self.time - self.xtrace_last_time;
        self.xtrace_last_time = self.time;

        let mut snap_parts: Vec<String> = Vec::with_capacity(self.xtrace_trace.len());
        for idx in 0..self.xtrace_trace.len() {
            let (tbl_id, sid) = &self.xtrace_trace[idx];
            let val = self.signal_table[*tbl_id].clone();
            snap_parts.push(format!(
                "{}={}",
                sid,
                Self::xtrace_format_value(&val, self.xtrace_real[idx], self.xtrace_string[idx])
            ));
            self.xtrace_prev_signals[idx] = val;
        }

        let w = self.xtrace_writer.as_mut().unwrap();
        // A lone `T` is a valid trace_record (§18); emit it so the section is
        // never empty. The `N,full` only follows if there is something to
        // snapshot — a design (or `--xtrace-scope`) with no traced signals must
        // NOT emit a bare `N,full,` (trailing comma, empty payload), which is a
        // malformed §10.6 record.
        let _ = writeln!(w, "T,+{}", delta);
        if snap_parts.is_empty() {
            return;
        }
        // Long N,full lines are split into 16-signal chunks per the emitter
        // pattern in §A.13 to keep parser memory bounded; continuation chunks
        // reuse the P record form.
        if snap_parts.len() <= 16 {
            let _ = writeln!(w, "N,full,{}", snap_parts.join(","));
        } else {
            let mut first = true;
            for chunk in snap_parts.chunks(16) {
                if first {
                    let _ = writeln!(w, "N,full,{}", chunk.join(","));
                    first = false;
                } else {
                    let _ = writeln!(w, "P,{}", chunk.join(","));
                }
            }
        }
    }

    /// Per-cycle XTrace emit. Writes a T record (if time advanced), the signal
    /// deltas as packed (P) or single (D) records, and one §10.4 `X` record per
    /// event triggered in this time slot.
    pub(super) fn xtrace_write_changes(&mut self) {
        if self.xtrace_writer.is_none() {
            return;
        }

        // Time-window gating (#8).
        if self.time > self.xtrace_to_t {
            // Past the window: finalize the dump now so no further trace I/O
            // happens for the rest of the run.
            self.xtrace_finish();
            return;
        }
        if self.time < self.xtrace_from_t {
            // Before the window: emit nothing.
            //
            // KNOWN LIMITATION (`--xtrace-from N` with N > 0): the pending
            // checkpoint is emitted at the first in-window *write* (the first
            // signal change at or after N), stamped and valued at that time —
            // not at the boundary N with the state accumulated up to N. So the
            // interval [N, first-change) is not represented. A precise fix needs
            // the state as-of N (different source than the settled-value read
            // used for the common N == 0 case), so it is deliberately left for a
            // dedicated pass rather than risk the N == 0 path. N == 0 (the
            // default, no window) is exact.
            return;
        }

        // First in-window write: emit the deferred N,full checkpoint from the
        // now-settled values, then fall through so any same-slot events still
        // record. The snapshot reseeds prev, so the change loop below finds
        // nothing to re-emit.
        if self.xtrace_snapshot_pending {
            self.xtrace_emit_pending_snapshot();
        }

        // Change detection. `xtrace_prev_signals` is parallel to `xtrace_trace`,
        // so each check is one indexed compare with no name hashing — but the
        // FULL scan is O(all traced signals) per time slot, and an unscoped dump
        // traces every net in the design (1.56 M on c906), which cost ~19x the
        // simulation itself. The shared dirty set (see `dump_dirty`) narrows the
        // walk to the slots of signals actually written since the last flush.
        // Values are collected UNFORMATTED: the sink's writer thread renders
        // them (`vcd_sink::write_xtrace_timestep`).
        let mut changes: Vec<(Arc<str>, Value, bool, bool)> = Vec::new();
        macro_rules! check_xt_slot {
            ($idx:expr_2021) => {{
                let idx = $idx;
                let id = self.xtrace_trace[idx].0;
                let val = &self.signal_table[id];
                if self.xtrace_prev_signals[idx] != *val {
                    changes.push((
                        self.xtrace_trace[idx].1.clone(),
                        val.clone(),
                        self.xtrace_real[idx],
                        self.xtrace_string[idx],
                    ));
                    self.xtrace_prev_signals[idx] = val.clone();
                }
            }};
        }
        match self.dump_dirty_slots(&self.xtrace_id_to_trace) {
            Some(slots) => {
                for &idx in &slots {
                    check_xt_slot!(idx as usize);
                }
            }
            None => {
                for idx in 0..self.xtrace_trace.len() {
                    check_xt_slot!(idx);
                }
            }
        }

        // §19.5 events. An SV `event` is a pulse, not a level: `->e1` three
        // times in a row is THREE events. Tracing it as a 1-bit level (the old
        // behaviour) emitted 0x1, 0x0, 0x1 — the second trigger read back as
        // "no event", and a trigger whose 0→1→0 toggle cancelled inside one
        // time slot emitted nothing at all. The trigger times are already
        // tracked for VCD (`event_triggered_time`), so reuse them. Events fire
        // independently of signal writes, so this list is always scanned in
        // full — it is tiny next to the trace table.
        let now = self.time;
        let mut fired: Vec<Arc<str>> = Vec::new();
        for idx in 0..self.xtrace_events.len() {
            let id = self.xtrace_events[idx].0;
            let triggered = self
                .name_opt(id)
                .and_then(|n| self.event_triggered_time.get(n))
                .copied()
                == Some(now);
            if triggered && self.xtrace_event_last[idx] != now {
                self.xtrace_event_last[idx] = now;
                fired.push(self.xtrace_events[idx].1.clone());
            }
        }

        if changes.is_empty() && fired.is_empty() {
            return;
        }

        let time_delta = if self.time != self.xtrace_last_time {
            let delta = self.time - self.xtrace_last_time;
            self.xtrace_last_time = self.time;
            Some(delta)
        } else {
            None
        };

        let w = self.xtrace_writer.as_mut().unwrap();
        w.post_xtrace_changes(super::vcd_sink::XtraceTimestep {
            time_delta,
            changes,
            events: fired,
        });

        // Periodic durable flush so a crash/SIGKILL leaves a readable partial
        // dump (Drop does not run under `panic = "abort"`). Bounds the loss to
        // the last XTRACE_FLUSH_EVERY change-steps; the clean exit path still
        // does a final flush in `xtrace_finish`.
        const XTRACE_FLUSH_EVERY: u32 = 4096;
        self.xtrace_dirty_steps += 1;
        if self.xtrace_dirty_steps >= XTRACE_FLUSH_EVERY {
            self.xtrace_dirty_steps = 0;
            let w = self.xtrace_writer.as_mut().unwrap();
            let _ = w.flush();
        }
    }

    /// Close the trace section and flush. Called once from `run()` after
    /// the event loop drains.
    pub(super) fn xtrace_finish(&mut self) {
        if self.xtrace_writer.is_none() {
            return;
        }
        // Close the trace at the final simulation time. Without a trailing T the
        // stream stops at the last CHANGE, so a run to t=40 whose last toggle was
        // at t=30 produces a trace that simply ends at 30 — the consumer cannot
        // tell a quiet tail from a truncated file. (The VCD path emits the same
        // closing `#t` marker for the same reason.) A lone `T` is a well-formed
        // trace_record per §18, and §19.3 gives it exactly this meaning: time
        // advanced, nothing changed.
        //
        // `xtrace_write_changes` re-enters here when the run passes
        // `--xtrace-to`; the window guard below keeps that path from appending a
        // T record past the end of the window.
        if self.time <= self.xtrace_to_t {
            self.xtrace_write_changes();
            if self.time > self.xtrace_last_time {
                let delta = self.time - self.xtrace_last_time;
                self.xtrace_last_time = self.time;
                if let Some(ref mut w) = self.xtrace_writer {
                    let _ = writeln!(w, "T,+{}", delta);
                }
            }
        }
        if let Some(ref mut w) = self.xtrace_writer {
            let _ = writeln!(w);
            let _ = writeln!(w, "@section end");
            let _ = w.flush();
        }
        self.xtrace_writer = None;
    }

    // ═══════════════════════════════════════════════════════════════
    // FST dump support (GTKWave binary format via the `fst-writer` crate)
    // ═══════════════════════════════════════════════════════════════

    /// Render a Value as the FST bit-string: full width, MSB first, using
    /// '0'/'1'/'x'/'z'. Width-1 yields a single byte.
    ///
    /// The implementation lives in `fst_sink` because the WRITER thread calls
    /// it — rendering is the bulk of a dump's per-change cost, and the
    /// background writer exists to absorb it. This wrapper keeps the
    /// simulator-side call site (the t=0 header snapshot) reading naturally.
    #[inline]
    pub(super) fn fst_format_value(val: &Value, width: u32) -> Vec<u8> {
        super::fst_sink::fst_format_declared_value(val, width)
    }

    /// Resolve string leaves without confusing indexed hierarchy scopes with
    /// variable selects. Unpacked struct leaves may have no type entry of
    /// their own, so start at the longest declared prefix and walk members.
    pub(super) fn fst_signal_is_string(&self, name: &str) -> bool {
        fn resolved<'a>(sim: &'a Simulator, mut dt: &'a DataType) -> &'a DataType {
            for _ in 0..64 {
                let DataType::TypeReference { name, .. } = dt else {
                    break;
                };
                let Some(next) =
                    super::elaborate::lookup_scoped_type(name, &sim.module.typedef_types)
                else {
                    break;
                };
                dt = next;
            }
            dt
        }
        if self.string_signals.contains(name) {
            return true;
        }
        let mut ends = Vec::new();
        let mut depth = 0usize;
        for (i, ch) in name.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                '.' if depth == 0 => ends.push(i),
                _ => {}
            }
        }
        ends.push(name.len());
        for end in ends.into_iter().rev() {
            let prefix = &name[..end];
            let leaf_start = prefix.rfind('.').map_or(0, |i| i + 1);
            let Some(leaf) = Self::flat_single_base(&prefix[leaf_start..]) else {
                continue;
            };
            let base = &prefix[..leaf_start + leaf.len()];
            if end == name.len() && self.string_signals.contains(base) {
                return true;
            }
            let Some(mut dt) = self
                .module
                .var_decl_types
                .get(prefix)
                .or_else(|| self.module.var_decl_types.get(base))
            else {
                continue;
            };
            let mut known = true;
            for (member, _) in Self::split_flat_path(name[end..].trim_start_matches('.')) {
                if member.is_empty() {
                    continue;
                }
                let DataType::Struct(su) = resolved(self, dt) else {
                    known = false;
                    break;
                };
                let Some(m) = su
                    .members
                    .iter()
                    .find(|m| m.declarators.iter().any(|d| d.name.name == member))
                else {
                    known = false;
                    break;
                };
                dt = &m.data_type;
            }
            if known {
                return Self::is_string_data_type(resolved(self, dt));
            }
        }
        false
    }

    /// Open the FST file, write the hierarchy (scopes + vars), transition to
    /// the body writer, and emit the t=0 snapshot. Mirrors `vcd_start_dump`:
    /// same scope-filter semantics, tick_s-derived timescale, and net dedup
    /// (aliased signal_table ids share one FST id via the `alias` parameter).
    pub(super) fn fst_start_dump(&mut self) {
        let filename = match self.fst_file.clone() {
            Some(f) => f,
            None => return,
        };
        self.sync_table_to_hashmap();

        // Same enumeration + top-relative scope normalization as `$dumpvars`
        // (see `dump_signal_names`): the old copy read the empty `self.signals`
        // mirror and compared absolute filter paths against relative names.
        // A scope may carry a depth, `LEVEL:SCOPE` (§21.7.1.4: 0 = every
        // level below the scope, N = N levels starting at it). Scopes with the
        // same depth are selected together; the union keeps first-seen order.
        let mut sig_names: Vec<String> = if self.fst_scopes.is_empty() {
            self.dump_signal_names(&self.fst_scopes, 0)
        } else {
            let mut groups: Vec<(u32, Vec<String>)> = Vec::new();
            for sc in &self.fst_scopes {
                let (depth, scope) = Self::fst_scope_depth(sc);
                match groups.iter_mut().find(|(d, _)| *d == depth) {
                    Some((_, g)) => g.push(scope.to_string()),
                    None => groups.push((depth, vec![scope.to_string()])),
                }
            }
            let mut seen: HashSet<String> = HashSet::default();
            let mut out = Vec::new();
            for (depth, scopes) in &groups {
                for n in self.dump_signal_names(scopes, *depth) {
                    if seen.insert(n.clone()) {
                        out.push(n);
                    }
                }
            }
            out
        };
        // The current FST writer only represents fixed-width vectors and
        // reals. A SystemVerilog string has no fixed maximum length (§6.16);
        // declaring it as a 1024-bit vector corrupts or aborts a later growth.
        // Omit unsupported variables explicitly instead of truncating them.
        let mut omitted_strings = Vec::new();
        sig_names.retain(|name| {
            let is_string = self.fst_signal_is_string(name);
            if is_string {
                omitted_strings.push(name.clone());
            }
            !is_string
        });
        if !omitted_strings.is_empty() {
            eprintln!(
                "Warning: FST skips unsupported variable-length string variables: {}",
                omitted_strings.join(", ")
            );
        }

        eprintln!(
            "[FST] dumping {} signals (scopes={})",
            sig_names.len(),
            self.fst_scopes.len()
        );

        // Name every `--fst-scope` that selected nothing. The common cause is a
        // generate block: the scopes are `gen[0]`/`gen[1]`, so a bare
        // `--fst-scope top.gen` matches no signal — silently, since the count
        // line above only reports the total.
        if !self.fst_scopes.is_empty() {
            let scopes: Vec<String> = self
                .fst_scopes
                .iter()
                .map(|sc| Self::fst_scope_depth(sc).1.to_string())
                .collect();
            for sc in &scopes {
                if !self
                    .dump_signal_names(std::slice::from_ref(sc), 0)
                    .is_empty()
                {
                    continue;
                }
                // Only offer the generate hint when the indexed form actually
                // exists, so a plain typo is not told to add "[0]".
                let indexed = format!("{}[0]", sc);
                if !self
                    .dump_signal_names(std::slice::from_ref(&indexed), 0)
                    .is_empty()
                {
                    eprintln!(
                        "Warning: --fst-scope '{}' matched no signals; a generate block is \
                         named per index — did you mean '{}'?",
                        sc, indexed
                    );
                } else {
                    eprintln!("Warning: --fst-scope '{}' matched no signals", sc);
                }
            }
        }

        // An FST with no variables is not a valid file — GTKWave and `fst2vcd`
        // both refuse to open it — so writing one leaves the user a broken
        // artifact instead of a diagnostic. Skip the file entirely.
        if sig_names.is_empty() {
            eprintln!(
                "Warning: no signals selected for the FST dump; '{}' not written",
                filename
            );
            self.fst_file = None;
            return;
        }

        // Timescale exponent (e.g. -9 for 1ns, -12 for 1ps) from tick_s.
        let ts_exp = self.module.tick_s.log10().round() as i8;
        let info = fst_writer::FstInfo {
            start_time: 0,
            timescale_exponent: ts_exp,
            version: format!("xezim {}", env!("CARGO_PKG_VERSION")),
            date: String::new(),
            file_type: fst_writer::FstFileType::Verilog,
        };
        let (mut header, fst_target) = match super::fst_sink::open_fst(&filename, &info) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Warning: cannot create FST file '{}': {:?}", filename, e);
                return;
            }
        };

        // Nested scope tree from dotted names (same shape as VCD's ScopeNode),
        // storing the backing signal_table index per leaf.
        struct FstNode {
            children: BTreeMap<String, FstNode>,
            // (display name, width, signal-table id, encoding, declaration kind, event)
            signals: Vec<(String, u32, usize, FstSignalType, FstVarType, bool)>,
        }
        impl FstNode {
            fn new() -> Self {
                FstNode {
                    children: BTreeMap::new(),
                    signals: Vec::new(),
                }
            }
        }
        let mut root = FstNode::new();
        // A port bound to a whole net is the SAME net (see `dump_backing_ids`) —
        // it reuses the parent's signal id, so `header.var` declares it as an FST
        // alias of the parent instead of a second signal with its own changes.
        let fst_backing = self.dump_backing_ids(&sig_names);
        for name in &sig_names {
            let tbl_id = match fst_backing.get(name.as_str()) {
                Some(&i) => i,
                _ => continue,
            };
            let width = self.lookup_signal_width(name).unwrap_or(1);
            let own_id = self
                .signal_name_to_id
                .get(name.as_str())
                .copied()
                .unwrap_or(tbl_id);
            let kind = self.dump_var_kind(name, own_id);
            let signal_type = if kind == VcdVarKind::Real {
                FstSignalType::real()
            } else {
                FstSignalType::bit_vec(width.max(1))
            };
            let var_type = match kind {
                VcdVarKind::Wire => FstVarType::Wire,
                VcdVarKind::Reg => FstVarType::Reg,
                VcdVarKind::Integer => FstVarType::Integer,
                VcdVarKind::Time => FstVarType::Time,
                VcdVarKind::Real => FstVarType::Real,
                VcdVarKind::Event => FstVarType::Event,
                VcdVarKind::Parameter => FstVarType::Parameter,
            };
            let parts: Vec<&str> = name.split('.').collect();
            let (scope_parts, leaf) = if parts.len() > 1 {
                (&parts[..parts.len() - 1], parts[parts.len() - 1])
            } else {
                (&[][..], parts[0])
            };
            let mut node = &mut root;
            for part in scope_parts {
                node = node
                    .children
                    .entry(part.to_string())
                    .or_insert_with(FstNode::new);
            }
            let display_name = match kind {
                VcdVarKind::Real | VcdVarKind::Event => leaf.to_string(),
                _ => match self.dump_var_range(name, width) {
                    Some((left, right)) => format!("{leaf} [{left}:{right}]"),
                    None => leaf.to_string(),
                },
            };
            node.signals.push((
                display_name,
                width,
                tbl_id,
                signal_type,
                var_type,
                kind == VcdVarKind::Event,
            ));
        }

        // DFS-emit scopes/vars, collecting (signal_table idx, FstSignalId).
        let mut trace: Vec<(usize, (FstSignalId, u32))> = Vec::new();
        let mut events: Vec<(usize, FstSignalId)> = Vec::new();
        let mut id_to_fst: HashMap<usize, FstSignalId> = HashMap::default();
        fn emit<W: std::io::Write + std::io::Seek>(
            header: &mut FstHeaderWriter<W>,
            name: &str,
            node: &FstNode,
            trace: &mut Vec<(usize, (FstSignalId, u32))>,
            events: &mut Vec<(usize, FstSignalId)>,
            id_to_fst: &mut HashMap<usize, FstSignalId>,
        ) {
            let _ = header.scope(name, "", FstScopeType::Module);
            for (leaf, width, tbl_id, signal_type, var_type, is_event) in &node.signals {
                // Aliased nets (same signal_table id) reuse one FST id.
                let alias = id_to_fst.get(tbl_id).copied();
                let fid = match header.var(
                    leaf,
                    *signal_type,
                    *var_type,
                    FstVarDirection::Implicit,
                    alias,
                ) {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                if alias.is_none() {
                    id_to_fst.insert(*tbl_id, fid);
                    if *is_event {
                        events.push((*tbl_id, fid));
                    } else {
                        let encoding_width = if *var_type == FstVarType::Real {
                            0
                        } else {
                            (*width).max(1)
                        };
                        trace.push((*tbl_id, (fid, encoding_width)));
                    }
                }
            }
            for (child_name, child) in &node.children {
                emit(header, child_name, child, trace, events, id_to_fst);
            }
            let _ = header.up_scope();
        }
        if self.root_is_multi_top() {
            // One root scope per top, as in the VCD header.
            if !root.signals.is_empty() {
                let unit = FstNode {
                    children: BTreeMap::new(),
                    signals: std::mem::take(&mut root.signals),
                };
                emit(
                    &mut header,
                    "$unit",
                    &unit,
                    &mut trace,
                    &mut events,
                    &mut id_to_fst,
                );
            }
            for (child_name, child) in &root.children {
                emit(
                    &mut header,
                    child_name,
                    child,
                    &mut trace,
                    &mut events,
                    &mut id_to_fst,
                );
            }
        } else {
            let top_name = self.module.name.clone();
            emit(
                &mut header,
                &top_name,
                &root,
                &mut trace,
                &mut events,
                &mut id_to_fst,
            );
        }

        let mut body = match header.finish() {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "Warning: cannot finish FST header for '{}': {:?}",
                    filename, e
                );
                return;
            }
        };

        // t=0 snapshot + seed prev.
        let _ = body.time_change(self.time);
        let mut prev: Vec<Value> = Vec::with_capacity(trace.len());
        for (tbl_id, (fid, width)) in &trace {
            let val = self.dump_value(*tbl_id);
            let _ = body.signal_change(*fid, &Self::fst_format_value(&val, *width));
            prev.push(self.signal_table[*tbl_id].clone());
        }
        for (_, fid) in &events {
            let _ = body.signal_change(*fid, b"0");
        }

        // Incremental change detection, shared with the VCD and XTrace writers
        // (see the `dump_dirty` field docs).
        self.fst_id_to_trace = self.build_dump_id_to_trace(&trace);
        self.enable_dump_dirty_tracking();
        self.fst_trace = trace;
        self.fst_prev_signals = prev;
        self.fst_event_last = vec![u64::MAX; events.len()];
        self.fst_events = events;
        self.fst_writer = Some(if self.dump_writer_threaded() {
            super::fst_sink::FstSink::threaded(body, fst_target, self.time)
        } else {
            super::fst_sink::FstSink::inline(body, fst_target, self.time)
        });
    }

    /// Per-timestep FST emit: `time_change` once (if anything changed), then a
    /// `signal_change` per changed net. Periodically flushes a value-change
    /// block for crash safety (parity with the XTrace durable flush).
    pub(super) fn fst_write_changes(&mut self) {
        if self.fst_writer.is_none() {
            return;
        }
        // Narrow the walk to the slots of signals actually written since the
        // last flush; the full scan is O(all traced nets) per time slot, which
        // on an unscoped dump dominates everything else (see `dump_dirty`).
        // Values are collected UNFORMATTED: the sink's writer thread renders
        // them (`fst_sink::fst_format_declared_value`). A `Value` <=64 bits wide is
        // inline storage, so this clone is a 24-byte memcpy with no allocation
        // — strictly cheaper than the per-bit `Vec<u8>` render it replaces.
        let mut changes: Vec<(FstSignalId, Value, u32)> = Vec::new();
        macro_rules! check_fst_slot {
            ($idx:expr_2021) => {{
                let idx = $idx;
                let (tbl_id, (fid, width)) = self.fst_trace[idx];
                let val = &self.signal_table[tbl_id];
                if self.fst_prev_signals[idx] != *val {
                    changes.push((fid, self.dump_value(tbl_id), width));
                    self.fst_prev_signals[idx] = val.clone();
                }
            }};
        }
        match self.dump_dirty_slots(&self.fst_id_to_trace) {
            Some(slots) => {
                for &idx in &slots {
                    check_fst_slot!(idx as usize);
                }
            }
            None => {
                for idx in 0..self.fst_trace.len() {
                    check_fst_slot!(idx);
                }
            }
        }
        let now = self.time;
        for idx in 0..self.fst_events.len() {
            let id = self.fst_events[idx].0;
            let fired = self
                .name_opt(id)
                .and_then(|name| self.event_triggered_time.get(name))
                .copied()
                == Some(now);
            if fired && self.fst_event_last[idx] != now {
                self.fst_event_last[idx] = now;
                let fid = self.fst_events[idx].1;
                changes.push((fid, Value::ones(1), 1));
                changes.push((fid, Value::zero(1), 1));
            }
        }
        if changes.is_empty() {
            return;
        }
        // Packing, block compression and the periodic crash-safe block flush
        // all happen inside the sink — on its writer thread by default.
        let time = self.time;
        self.fst_writer
            .as_mut()
            .unwrap()
            .post(super::fst_sink::FstTimestep { time, changes });
    }

    /// Finalize and close the FST file. Blocks until the writer thread has
    /// written the trailer, so the dump is complete when this returns.
    pub(super) fn fst_finish(&mut self) {
        if self.fst_writer.is_none() {
            return;
        }
        // Flush this slot's changes before closing. `run()` calls the three
        // `*_finish` methods after the `final` blocks, and a `final` block may
        // write signals — the VCD and XTrace paths pick those up because their
        // finish routines re-run change detection, and FST silently dropped
        // them. Anything written since the last `dump_write_changes` belongs in
        // the dump.
        self.fst_write_changes();
        // Close the wave window at the final simulation time. Without a
        // trailing time record a viewer stops at the last TRANSITION, so a run
        // to t=200 whose last toggle was at t=20 produces a dump that simply
        // ends at 20 and a quiet tail is indistinguishable from a truncated
        // file. VCD emits a closing `#t` and XTrace a closing `T,+delta` for
        // exactly this reason; FST never did.
        let time = self.time;
        if let Some(sink) = self.fst_writer.as_mut() {
            sink.post(super::fst_sink::FstTimestep {
                time,
                changes: Vec::new(),
            });
        }
        // `finish()` joined the writer thread, so the last block and the final
        // header are in the dump (and the time tables repaired, see
        // `fst_sink::repair_time_tables`).
        if let Some(sink) = self.fst_writer.take() {
            sink.finish();
            // `--fst-repack`: gzip-wrap the finished dump the way GTKWave's
            // repack-on-close does, now that the trailer is on disk.
            if let (Some(path), Some(level)) =
                (&self.fst_file, super::fst_sink::fst_repack_level())
            {
                let t0 = std::time::Instant::now();
                match super::fst_sink::repack_on_close(path, level) {
                    Ok((before, after)) => eprintln!(
                        "[FST] repacked {}: {} -> {} bytes ({:.2}x) in {:.2}s",
                        path,
                        before,
                        after,
                        before as f64 / after.max(1) as f64,
                        t0.elapsed().as_secs_f64()
                    ),
                    Err(e) => eprintln!("[FST] repack of {} failed: {}", path, e),
                }
            }
        }
    }
}
