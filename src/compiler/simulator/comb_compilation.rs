//! Simulator comb compilation.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn prepared_comb_cache_eligible(&self) -> bool {
        self.prepared_comb_cache_path.is_some()
            // Instrumented entries hold per-run counters.
            && self.code_cov.is_none()
            && self.module.udp_instances.is_empty()
            && self.module.timing_checks.is_empty()
            && self.sdf_delays.is_empty()
            // Region fusion (XEZIM_REGIONS) rewrites the entry list; a cache
            // written under one setting must not be replayed under the other.
            && !matches!(std::env::var("XEZIM_REGIONS").as_deref(), Ok("1"))
            // Vector coalescing also rewrites the entry list and remains an
            // opt-in experiment, so keep its prepared form out of the shared
            // cache until the mode becomes part of the cache key.
            && !matches!(std::env::var("XEZIM_VEC_COALESCE").as_deref(), Ok("1"))
    }

    pub(super) fn drop_comb_source_ast(&mut self) {
        self.module.continuous_assigns = Vec::new();
        self.module.pending_cont_assign = Vec::new();
        self.module.always_blocks = Vec::new();
        #[cfg(all(target_env = "gnu", not(miri)))]
        unsafe {
            libc::malloc_trim(0);
        }
        // Hand the pages freed by the comb build (the source ASTs, the
        // ordering graph, the build's temporaries) back to the OS now: the
        // allocator would otherwise keep them resident through the whole
        // simulation (64k-cell DRAM: 1373 -> 857 MB steady RSS). The callee
        // skips a small process, where purging only splits huge pages.
        xezim_core::release_free_memory();
    }

    pub(super) fn prepared_comb_cache_is_valid(&self, cache: &PreparedCombCache) -> bool {
        let signal_count = self.signal_table.len();
        let entry_count = cache.entries.len();
        cache.signal_count == signal_count
            && cache.dep_offsets.len() == signal_count + 1
            && cache.dep_offsets.first().copied() == Some(0)
            && cache.dep_offsets.windows(2).all(|pair| pair[0] <= pair[1])
            && cache.dep_offsets.last().copied().map(|n| n as usize)
                == Some(cache.dep_entries.len())
            && cache
                .dep_entries
                .iter()
                .all(|&idx| (idx as usize) < entry_count)
            && cache.unresolved_idx.iter().all(|&idx| idx < entry_count)
            && cache.time0_idx.iter().all(|&idx| idx < entry_count)
            && cache
                .entries
                .iter()
                .chain(cache.time0_deferred.iter())
                .all(|entry| {
                    entry
                        .cold
                        .read_signal_ids
                        .iter()
                        .chain(entry.cold.write_signal_ids.iter())
                        .all(|&id| id < signal_count)
                })
            && cache.cont_driven.iter().all(|&id| id < signal_count)
    }

    pub(super) fn try_load_prepared_comb_cache(&mut self) -> bool {
        if !self.prepared_comb_cache_eligible() {
            return false;
        }
        let path = self.prepared_comb_cache_path.as_ref().unwrap().clone();
        let result = (|| -> Result<PreparedCombCache, String> {
            use std::io::Read;
            let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut reader = std::io::BufReader::new(file);
            let mut magic = [0u8; PREPARED_COMB_MAGIC.len()];
            reader.read_exact(&mut magic).map_err(|e| e.to_string())?;
            if &magic != PREPARED_COMB_MAGIC {
                return Err("bad format marker".to_string());
            }
            bincode::deserialize_from(reader).map_err(|e| e.to_string())
        })();
        let cache = match result {
            Ok(cache) if self.prepared_comb_cache_is_valid(&cache) => cache,
            Ok(_) => {
                eprintln!(
                    "[CACHE] prepared-comb rejected invalid payload: {}",
                    path.display()
                );
                let _ = std::fs::remove_file(&path);
                return false;
            }
            Err(err) if path.exists() => {
                eprintln!(
                    "[CACHE] prepared-comb unreadable ({}): {}",
                    err,
                    path.display()
                );
                let _ = std::fs::remove_file(&path);
                return false;
            }
            Err(_) => return false,
        };

        // The settle loop walks this CSR without bounds checks: a cache file
        // whose prefix sum is not monotone, does not end at the entry count,
        // or names an entry past the table is rejected and rebuilt.
        {
            let n_entries = cache.entries.len();
            let offs = &cache.dep_offsets;
            let csr_ok = offs.first().is_none_or(|&f| f == 0)
                && offs.windows(2).all(|w| w[0] <= w[1])
                && offs
                    .last()
                    .is_none_or(|&l| l as usize == cache.dep_entries.len())
                && cache.dep_entries.iter().all(|&e| (e as usize) < n_entries);
            if !csr_ok {
                eprintln!(
                    "[CACHE] prepared-comb dependency table inconsistent; rebuilding: {}",
                    path.display()
                );
                let _ = std::fs::remove_file(&path);
                return false;
            }
        }
        self.comb_entries = cache.entries;
        self.ts_comb.clear();
        self.comb_dep_offsets = cache.dep_offsets;
        self.comb_dep_entries = cache.dep_entries;
        self.comb_dep_edges.clear();
        self.gate_lane_valid = false;
        self.comb_unresolved_idx = cache.unresolved_idx;
        self.comb_time0_idx = cache.time0_idx;
        self.comb_time0_deferred = cache.time0_deferred;
        self.cont_driven = cache.cont_driven;
        self.drop_comb_source_ast();
        eprintln!(
            "[CACHE] prepared-comb hit: {} entries from {}",
            self.comb_entries.len(),
            path.display()
        );
        true
    }

    pub(super) fn write_prepared_comb_cache(&self) {
        if !self.prepared_comb_cache_eligible() {
            return;
        }
        let path = self.prepared_comb_cache_path.as_ref().unwrap();
        let Some(parent) = path.parent() else {
            return;
        };
        if let Err(err) = std::fs::create_dir_all(parent) {
            eprintln!("[CACHE] prepared-comb store skipped: {}", err);
            return;
        }
        let tmp = path.with_extension(format!("xezcomb.tmp.{}", std::process::id()));
        let result = (|| -> Result<(), String> {
            let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            let mut writer = std::io::BufWriter::new(file);
            writer
                .write_all(PREPARED_COMB_MAGIC)
                .map_err(|e| e.to_string())?;
            let payload = PreparedCombCacheRef {
                signal_count: self.signal_table.len(),
                entries: &self.comb_entries,
                dep_offsets: &self.comb_dep_offsets,
                dep_entries: &self.comb_dep_entries,
                unresolved_idx: &self.comb_unresolved_idx,
                time0_idx: &self.comb_time0_idx,
                time0_deferred: &self.comb_time0_deferred,
                cont_driven: &self.cont_driven,
            };
            bincode::serialize_into(&mut writer, &payload).map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            drop(writer);
            std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => eprintln!(
                "[CACHE] prepared-comb stored: {} entries in {}",
                self.comb_entries.len(),
                path.display()
            ),
            Err(err) => {
                let _ = std::fs::remove_file(&tmp);
                eprintln!("[CACHE] prepared-comb store skipped: {}", err);
            }
        }
    }

    pub(super) fn build_comb_entries(&mut self) {
        // Entry indices are about to change — drop stale two-state bodies.
        self.ts_comb.clear();
        self.packed_array_copy_values.clear();
        if self.try_load_prepared_comb_cache() {
            return;
        }
        let mut entries = Vec::new();

        // Continuous assigns: drain by-value so that the per-CA AST
        // (lhs + rhs Expression trees) is freed as each entry is processed
        // — peak memory tracks bytecode size, not the full Vec of source
        // ASTs. Chains the eager-elab module.continuous_assigns with the
        // lazy-prefix pending_cont_assign vec; pending entries are
        // materialized one at a time inside the iterator.
        //
        // Measured on c910: serialized continuous_assigns is ~394 MB
        // (501 K entries with hierarchical-name expressions). In-memory
        // (with allocator overhead + Box<Expression> per node) is roughly
        // 4 GB. Streaming drain releases per-CA allocations as we iterate.
        let cas = std::mem::take(&mut self.module.continuous_assigns);
        let pending_ca = std::mem::take(&mut self.module.pending_cont_assign);
        // §10.6.2: a whole-ARRAY continuous assign (`assign dst = src;` between
        // unpacked arrays) has no single signal to drive — the ELEMENTS are the
        // signals — so it compiled to a phantom scalar and silently drove
        // nothing, leaving `dst` x for the whole run. Elaboration expands the
        // module-scope ones, but a SUB-MODULE's assigns arrive here as
        // `pending_cont_assign` and never pass through that path, so expand
        // them at the one point every continuous assign is guaranteed to cross.
        // The array table is snapshotted because the loop below needs `&mut
        // self`; it is names→ranges only, not ASTs.
        let arrays_snapshot = self.module.arrays.clone();
        let descending_snapshot = self.module.descending_arrays.clone();
        let decl_dims_snapshot = self.module.unpacked_decl_dims.clone();
        // §23.9/§6.10: a HIERARCHICAL continuous assign can target a
        // sub-instance INPUT PORT (`assign uut.mid.core.i_clk = tb_clk;`
        // with the port chain above unconnected). Identity port connects
        // collapse by SUBSTITUTION, and different readers may have bound at
        // DIFFERENT levels of the chain (the deep flop's clock bound one
        // level up). Fan the drive out to EVERY name in the alias chain so
        // all of them stay coherent. Width-guarded; chains are short and
        // hierarchy overrides rare, so the duplication is negligible.
        let alias_snapshot = self.module.port_aliases.clone();
        let alias_widths: HashMap<String, u32> = alias_snapshot
            .iter()
            .flat_map(|(k, v)| [k.clone(), v.clone()])
            .filter_map(|n| {
                self.module
                    .signals
                    .get(&n)
                    .map(|sig| (n.clone(), sig.width))
            })
            .collect();
        // Hoisted out of the loop to reuse capacity across iterations.
        // Avoids ~2 × N HashMap allocs/drops where N = cont_assign count
        // (63K on c906, 501K on c910). Clear() preserves the bucket array.
        let mut reads: HashSet<String> = HashSet::default();
        let mut writes: HashSet<String> = HashSet::default();
        let mut ca_compile_fail: HashMap<&'static str, usize> = HashMap::default();
        let mut ca_compile_fail_samples = 0usize;
        let cont_loop_t0 = std::time::Instant::now();
        // Read once, not per assign (a `getenv` scan per assign is
        // measurable at 150k assigns).
        let dump_ca_reads = std::env::var_os("XEZIM_DUMP_CA_READS").is_some();
        let dump_unresolved = std::env::var_os("XEZIM_DUMP_UNRESOLVED").is_some();
        for mut ca in cas
            .into_iter()
            .chain({
                let params_snapshot = self.module.parameters.clone();
                pending_ca
                    .into_iter()
                    .map(move |p| p.materialize(&params_snapshot))
            })
            .flat_map(|ca| {
                match crate::compiler::elaborate::whole_array_assign_parts_dir(
                    &ca.lhs,
                    &ca.rhs,
                    ca.delay,
                    &arrays_snapshot,
                    Some(&descending_snapshot),
                    Some(&decl_dims_snapshot),
                ) {
                    Some(parts) => parts,
                    None => vec![ca],
                }
            })
            .flat_map(|ca| {
                let chain: Option<Vec<String>> = match &ca.lhs.kind {
                    // ONLY a user-written HIERARCHY REFERENCE (multi-segment
                    // path `a.b.c`). Inlined port-connect assigns carry their
                    // instance-scoped name as ONE dotted segment — fanning
                    // those out would double-drive the parent net of every
                    // identity connection in the design.
                    ExprKind::Ident(h)
                        if h.path.len() > 1 && h.path.iter().all(|s| s.selects.is_empty()) =>
                    {
                        let flat = Self::resolve_hier_name_static(h, &self.module);
                        if flat.contains('.') && alias_snapshot.contains_key(&flat) {
                            let w0 = alias_widths.get(&flat).copied();
                            let mut names = vec![flat.clone()];
                            let mut cur = flat;
                            for _ in 0..64 {
                                let Some(p) = alias_snapshot.get(&cur) else {
                                    break;
                                };
                                if alias_widths.get(p).copied() != w0 || names.contains(p) {
                                    break;
                                }
                                names.push(p.clone());
                                cur = p.clone();
                            }
                            (names.len() > 1).then_some(names)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                match chain {
                    None => vec![ca],
                    Some(names) => names
                        .into_iter()
                        .map(|n| {
                            let mut c = ca.clone();
                            c.lhs = Expression::new(
                                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                    root: None,
                                    path: vec![crate::ast::expr::HierPathSegment {
                                        name: crate::ast::Identifier {
                                            name: n,
                                            span: crate::ast::Span::dummy(),
                                        },
                                        selects: Vec::new(),
                                    }],
                                    span: crate::ast::Span::dummy(),
                                    cached_signal_id: std::cell::Cell::new(None),
                                    cached_resolved_name: std::cell::OnceCell::new(),
                                }),
                                ca.lhs.span,
                            );
                            c
                        })
                        .collect(),
                }
            })
        {
            let explicit_delay = ca.delay;
            // Captured before `ca.lhs` may be moved into the CombItem below.
            let ca_span = ca.lhs.span;
            // Statement coverage counts this assignment's evaluations: keep it
            // on a form that runs a counter (compiled, or interpreted).
            let cov_counter = if self.code_cov.is_some() {
                Self::cov_contassign(
                    &self.module,
                    &mut self.code_cov,
                    &mut self.code_cov_hits,
                    &mut ca,
                )
            } else {
                None
            };
            // §21.7.2.1 `var_type`: an object whose only driver is continuous is
            // a `wire` in a dump (see `cont_driven`). Recorded here — this is the
            // last point at which the continuous assigns still exist as ASTs.
            // Only a WHOLE-name LHS counts: `assign a[3] = …` leaves the rest of
            // `a` to other drivers, so the object is not purely continuous.
            if let ExprKind::Ident(lhs_hier) = &ca.lhs.kind {
                if lhs_hier.path.iter().all(|s| s.selects.is_empty()) {
                    let n = Self::resolve_hier_name_static(lhs_hier, &self.module);
                    // A collapsed identity connect is a dropped self-copy, not
                    // a driver of the shared id — the PARENT keeps its own kind.
                    if !self.collapsed_port_children.contains(n.as_str()) {
                        if let Some(&id) = self.signal_name_to_id.get(n.as_str()) {
                            self.cont_driven.insert(id);
                        }
                    }
                }
            }
            // A whole-name identity assign (`assign dst = src`, the shape of
            // every port connect) resolves both ids up front: when a collapsed
            // identity port net made it a self-copy (see
            // `collapse_identity_port_nets`) it is dropped here, before any of
            // the read/write/scope analysis below, none of which it needs.
            let ident_copy_ids: Option<(usize, usize)> =
                if explicit_delay == 0 && cov_counter.is_none() {
                    if let (ExprKind::Ident(lhs_hier), ExprKind::Ident(rhs_hier)) =
                        (&ca.lhs.kind, &ca.rhs.kind)
                    {
                        let dst_name = Self::resolve_hier_name_static(lhs_hier, &self.module);
                        let src_name = Self::resolve_hier_name_static(rhs_hier, &self.module);
                        match (
                            self.signal_name_to_id.get(dst_name.as_str()),
                            self.signal_name_to_id.get(src_name.as_str()),
                        ) {
                            (Some(&dst_id), Some(&src_id)) => Some((dst_id, src_id)),
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
            if ident_copy_ids.is_some_and(|(dst_id, src_id)| dst_id == src_id) {
                continue;
            }
            // So is an element connect of an unpacked-array port collapsed
            // onto its actual (see `collapse_array_port_nets`).
            if ca.rhs_parent_scoped
                && explicit_delay == 0
                && cov_counter.is_none()
                && matches!(ca.lhs.kind, ExprKind::Index { .. })
            {
                let cell = |e: &Expression| {
                    let (base, idx) = Self::const_elem_ref(e, &self.module)?;
                    self.elem_cell(&base, &idx)
                };
                if let (Some(dst), Some(src)) = (cell(&ca.lhs), cell(&ca.rhs)) {
                    if dst == src {
                        continue;
                    }
                }
            }
            reads.clear();
            writes.clear();
            Self::collect_expr_reads(&ca.rhs, &self.module, &mut reads);
            Self::collect_lhs_writes(&ca.lhs, &self.module, &mut writes);
            // Substituted port actuals whose bare name collides with a child
            // declaration arrive ROOT-marked (core rewrite_expr_impl, issue
            // #128: `.v_node(out_val)` into a module with its own `out_val`).
            // `collect_expr_reads` flattens the root away, so gather the
            // rooted names separately: such a read resolves ABSOLUTELY, and
            // the scope-candidate machinery below must not requalify it back
            // into the child — that self-loop (dep AND eval on `b0.out_val`)
            // silently severed the parent feedback path.
            let mut rooted_reads: HashSet<String> = HashSet::default();
            Self::collect_rooted_ident_names(&ca.rhs, &mut rooted_reads);
            // A bare LHS that IS a registered top-level signal resolves
            // absolutely — force no scope hint (both inferences would
            // otherwise scope a multi-driven net's folded
            // `b = $__wres(u1.b, u2.b)` into one INSTANCE, writing u2.b
            // instead of b and leaving the net z forever).
            // The base ident of the LHS, looking through ONE select layer —
            // `t[1:0] = …` and `t[i] = …` anchor on `t` just as `t = …` does.
            let lhs_base_hier: Option<&crate::ast::expr::HierarchicalIdentifier> =
                match &ca.lhs.kind {
                    ExprKind::Ident(h) => Some(h),
                    ExprKind::Index { expr: b, .. } | ExprKind::RangeSelect { expr: b, .. } => {
                        match &b.kind {
                            ExprKind::Ident(h) => Some(h),
                            _ => None,
                        }
                    }
                    _ => None,
                };
            // Applies to select-form LHS too: a port hop whose actual shares
            // the SUBMODULE PORT's name (`.t(t[1:0])` in the testbench, tb
            // signal also `t`) built `t[1:0] = u_t.t` with scope hint `u_t`,
            // so the write resolved scope-first to `u_t.t` — a self-assign —
            // and the testbench signal stayed X forever.
            let lhs_is_absolute = lhs_base_hier.is_some_and(|h| {
                h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && !h.path[0].name.name.contains('.')
                    && self
                        .signal_name_to_id
                        .contains_key(h.path[0].name.name.as_str())
            });
            let scope_hint = if lhs_is_absolute || ca.rhs_parent_scoped {
                // A port-connection assign's RHS names live in the PARENT
                // scope by construction (§23.3.3) — the child hint turned
                // `.d(d ^ 1)` with a same-named parent net into the
                // self-loop `u1.d = u1.d ^ 1` (x forever).
                None
            } else {
                self.infer_contassign_scope_hint(&ca.lhs, &ca.rhs)
                    .or_else(|| self.infer_scope_from_rw_sets(&writes, &reads))
            };
            // §20.3: `$time` in the assign counts the timeunit of the module
            // the `assign` is written in. A submodule's assign is elaborated
            // with fully qualified names and often has NO resolution hint, so
            // record its source instance when the hint would not name it.
            let ts_scope = ca
                .origin
                .as_ref()
                .map(|(_, sc)| sc.as_str())
                .filter(|sc| *sc != scope_hint.as_deref().unwrap_or(""))
                .map(str::to_string);
            // Detect identity assigns: assign dst = src (simple signal-to-signal copy)
            let direct_copy = if let Some((dst_id, src_id)) = ident_copy_ids {
                let width = self.signal_widths[dst_id];
                let delay = self.sdf_delays.get(dst_id).copied().unwrap_or(0);
                if width == self.signal_widths[src_id] {
                    if width <= 64 && delay == 0 {
                        Some(CombItem::FastDirectCopy { dst_id, src_id })
                    } else if self
                        .module_paths
                        .as_ref()
                        .is_some_and(|mp| mp.is_path_net(dst_id))
                    {
                        // §30.4 path-delayed output: the general
                        // cont-assign path cancels a pending update
                        // when the source reverts (§28.16 pulse
                        // rejection); DirectCopy does not.
                        None
                    } else {
                        Some(CombItem::DirectCopy {
                            dst_id,
                            src_id,
                            width,
                        })
                    }
                } else if delay == 0 {
                    // Width-mismatched bare ident copy (e.g. a wider
                    // parent net driving a narrower input port).
                    // Resolve both ids statically here instead of
                    // falling through to the bytecode compiler, whose
                    // scope_hint can mis-resolve a parent-scope RHS to a
                    // same-named local port signal (self-assign -> X).
                    // DirectCopy resizes the source to `width` (the
                    // destination width). (pr2224949)
                    Some(CombItem::DirectCopy {
                        dst_id,
                        src_id,
                        width,
                    })
                } else {
                    None
                }
            } else {
                None
            };

            // Resolve write targets, retrying with scope_hint for bare names.
            let wids: Vec<usize> = writes
                .iter()
                .filter_map(|w| {
                    if let Some(&id) = self.signal_name_to_id.get(w.as_str()) {
                        return Some(id);
                    }
                    if let Some(scope) = &scope_hint {
                        let qualified = format!("{}.{}", scope, w);
                        if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                            return Some(id);
                        }
                    }
                    None
                })
                .collect();

            // For bare-identifier LHS, compile the RHS and use BlockingAssign
            // (whole-signal write). For BitSelect/PartSelect/Concat LHS, use
            // compile_cont_assign_lhs which routes through compile_blocking_target
            // so only the addressed sub-range is updated — essential for yosys
            // gate-level netlists whose per-bit assigns `assign d[0] = expr;`
            // would otherwise clobber the whole vector wire.
            let lhs_is_bare_ident = matches!(ca.lhs.kind, ExprKind::Ident(_));
            // A packed-struct member LHS (`assign s.m0 = …`) parses as a 2+-
            // segment `Ident` whose base names a container with a registered
            // field layout (after submodule inlining the scope is baked into
            // the first segment, e.g. `Ident(["d1.s", "m0"])`). It resolves to
            // no leaf signal, so `wids` is empty and it would fall to the AST
            // interpreter — where its read dependency mis-resolves bare-first
            // to the wrong (top-scope) input and never re-triggers when the
            // real scoped input changes. Detect it here so it is routed through
            // the bytecode sub-range path (which splices into the container),
            // and record the container id as the write target so downstream
            // readers re-fire when the member changes.
            let struct_member_base_id: Option<usize> = if let ExprKind::Ident(h) = &ca.lhs.kind {
                if h.path.len() >= 2 && h.path.iter().all(|s| s.selects.is_empty()) {
                    let member = h.path.last().unwrap().name.name.as_str();
                    let base: String = h.path[..h.path.len() - 1]
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let base_id =
                        self.signal_name_to_id
                            .get(base.as_str())
                            .copied()
                            .or_else(|| {
                                scope_hint.as_ref().and_then(|sc| {
                                    self.signal_name_to_id
                                        .get(format!("{}.{}", sc, base).as_str())
                                        .copied()
                                })
                            });
                    let has_field = self
                        .module
                        .packed_struct_fields
                        .get(base.as_str())
                        .or_else(|| {
                            scope_hint.as_ref().and_then(|sc| {
                                self.module
                                    .packed_struct_fields
                                    .get(&format!("{}.{}", sc, base))
                            })
                        })
                        .is_some_and(|f| f.iter().any(|(m, _, _)| m == member));
                    base_id.filter(|_| has_field)
                } else {
                    None
                }
            } else {
                None
            };
            // Ensure the container is in the write set so the settle DAG links
            // this entry to downstream readers of the whole struct.
            let wids: Vec<usize> = if let Some(bid) = struct_member_base_id {
                if wids.contains(&bid) {
                    wids
                } else {
                    let mut w = wids;
                    w.push(bid);
                    w
                }
            } else {
                wids
            };
            // Try fused-gate fast path first: recognizes yosys patterns like
            // `assign d[0] = a & b` or `assign d[0] = ~(a & b)` — executes
            // without VM dispatch, just bit reads + 4-state combinator + set_bit.
            let fused = if explicit_delay == 0 && cov_counter.is_none() {
                self.try_build_fused_gate(&ca.lhs, &ca.rhs, scope_hint.as_deref())
            } else {
                None
            };
            // `assign #(rise, fall[, turnoff]) net = …` (§10.3.3): the lowered
            // item carries the rise delay; the fall / turn-off values go to
            // the per-net maps consulted by `schedule_delayed_with_delay`,
            // the one point where both the target and the new value are
            // known. Whole-net targets only, like the gate form (§28.11).
            if explicit_delay > 0 && wids.len() == 1 && lhs_is_bare_ident {
                if let Some(f) = ca.delay_fall {
                    if f != explicit_delay {
                        self.gate_fall_delay_by_id.insert(wids[0], f);
                    }
                }
                if let Some(t) = ca.delay_off {
                    if t != explicit_delay {
                        self.gate_off_delay_by_id.insert(wids[0], t);
                    }
                }
            }
            // Packed cells have no per-element dependency epochs. Retain
            // these port copies as AST entries so their evaluator can poll
            // the source without redriving an unchanged value (§38.34).
            let packed_array_copy = ca.rhs_parent_scoped
                && Self::const_elem_ref(&ca.rhs, &self.module)
                    .and_then(|(base, idx)| self.elem_cell(&base, &idx))
                    .is_some_and(is_packed_id);
            let item = if explicit_delay > 0 || packed_array_copy {
                CombItem::ContAssign {
                    lhs: Box::new(ca.lhs.clone()),
                    rhs: Box::new(ca.rhs.clone()),
                    delay: explicit_delay,
                }
            } else if let Some(op) = fused {
                CombItem::FusedGate { op }
            } else if let Some(dc_val) = direct_copy {
                dc_val
            } else if wids.len() == 1
                && lhs_is_bare_ident
                && struct_member_base_id.is_none()
                // A specify/SDF path delay on the destination must route the
                // write through the delayed-update scheduler. The bytecode's
                // writeback commits immediately, so compiling here dropped the
                // delay for any non-trivial RHS: the DirectCopy/fused-gate
                // paths already refuse a delayed dst, but a cell whose output
                // is `assign y = a & b;` (every behavioral gate model) took
                // this branch and its `(a => y) = 3` specify was ignored.
                && self.sdf_delays.get(wids[0]).copied().unwrap_or(0) == 0
            {
                let dst_id = wids[0];
                let width = self.signal_widths[dst_id];
                let mut compiler = super::bytecode::BytecodeCompiler::new(
                    &self.signal_name_to_id,
                    &self.signal_signed,
                    &self.signal_widths,
                    &self.module.arrays,
                    &self.widths,
                );
                compiler.set_scope_hint(scope_hint.clone());
                compiler.set_params(&self.module.parameters);
                compiler.set_param_leaf_idx(self.param_leaf_index());
                compiler.set_cast_widths(&self.cast_widths);
                compiler.set_packed_struct_fields(
                    self.bc_packed_fields
                        .as_ref()
                        .unwrap_or(&self.module.packed_struct_fields),
                );
                compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
                compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
                compiler.set_typedef_types(&self.module.typedef_types);
                compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
                compiler.set_assoc_arrays(&self.module.associative_arrays);
                compiler.set_packed_full_dims(&self.module.packed_full_dims);
                compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
                compiler.set_arrays_2d(&self.module.arrays_2d);
                compiler
                    .set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
                compiler.set_array_first_id(&self.array_first_id);
                // Without the function table, `compile_pure_call` cannot even
                // LOOK UP a callee, so any RHS containing a function call
                // bailed to the AST interpreter (`Expr_Call`) — dragging the
                // hottest RTL idiom there is (`assign x = f(sel)` decode
                // helpers) onto the slowest path. Every other compiler
                // instantiation passes it; this one simply never did.
                compiler.set_functions(&self.module.functions);
                compiler.top_module_name = Some(self.module.name.clone());
                if let Some(c) = cov_counter {
                    compiler.emit_cov_hit(c);
                }
                if compiler.compile_cont_assign(&ca.rhs, dst_id, width) {
                    CombItem::CompiledContAssign {
                        compiled: compiler.finish(),
                    }
                } else {
                    if std::env::var("XEZIM_COMPILE_FAIL_STATS").is_ok()
                        && !self.cont_assign_lhs_is_noop(&ca.lhs)
                    {
                        *ca_compile_fail
                            .entry(compiler.bail_reason.unwrap_or("unknown"))
                            .or_insert(0) += 1;
                        if ca_compile_fail_samples < 8 && !self.cont_assign_lhs_is_noop(&ca.lhs) {
                            let lhs_base = Self::base_ident_name(&ca.lhs);
                            eprintln!(
                                "[COMPILE_FAIL_SAMPLE] reason={} lhs_w={} base={:?} array={:?} array2d={:?} signal={:?} lhs={:?} rhs={:?}",
                                compiler.bail_reason.unwrap_or("unknown"),
                                width,
                                lhs_base,
                                lhs_base.as_ref().and_then(|n| self.module.arrays.get(n)),
                                lhs_base.as_ref().and_then(|n| self.module.arrays_2d.get(n)),
                                lhs_base
                                    .as_ref()
                                    .and_then(|n| self.signal_name_to_id.get(n.as_str()))
                                    .map(|id| (*id, self.signal_widths[*id])),
                                ca.lhs.kind,
                                ca.rhs.kind
                            );
                            ca_compile_fail_samples += 1;
                        }
                    }
                    if self.cont_assign_lhs_is_noop(&ca.lhs) {
                        CombItem::Noop
                    } else {
                        CombItem::ContAssign {
                            lhs: Box::new(ca.lhs),
                            rhs: Box::new(ca.rhs),
                            delay: 0,
                        }
                    }
                }
            } else if !lhs_is_bare_ident || struct_member_base_id.is_some() {
                // Sub-range LHS: try bytecode compile so bit/range writes run
                // at VM speed instead of through the interpreted assign_value.
                // Packed-struct member LHS (`s.m0`) also routes here — the
                // compiler splices it into the container's bit range.
                let mut compiler = super::bytecode::BytecodeCompiler::new(
                    &self.signal_name_to_id,
                    &self.signal_signed,
                    &self.signal_widths,
                    &self.module.arrays,
                    &self.widths,
                );
                compiler.set_scope_hint(scope_hint.clone());
                compiler.set_params(&self.module.parameters);
                compiler.set_param_leaf_idx(self.param_leaf_index());
                compiler.set_cast_widths(&self.cast_widths);
                compiler.set_packed_struct_fields(
                    self.bc_packed_fields
                        .as_ref()
                        .unwrap_or(&self.module.packed_struct_fields),
                );
                compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
                compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
                compiler.set_typedef_types(&self.module.typedef_types);
                compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
                compiler.set_assoc_arrays(&self.module.associative_arrays);
                compiler.set_packed_full_dims(&self.module.packed_full_dims);
                compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
                compiler.set_arrays_2d(&self.module.arrays_2d);
                compiler
                    .set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
                compiler.set_packed_struct_fields(
                    self.bc_packed_fields
                        .as_ref()
                        .unwrap_or(&self.module.packed_struct_fields),
                );
                compiler.set_array_first_id(&self.array_first_id);
                // Same omission as the single-dest site above: without the
                // function table a concat-LHS decode assign
                // (`assign {a,b} = ({N{hit}} & f(w)) | ...`) could never
                // inline its calls and stayed on the AST interpreter.
                compiler.set_functions(&self.module.functions);
                compiler.top_module_name = Some(self.module.name.clone());
                if let Some(c) = cov_counter {
                    compiler.emit_cov_hit(c);
                }
                let lhs_w = compiler.infer_lhs_width_pub(&ca.lhs);
                if lhs_w > 0 && compiler.compile_cont_assign_lhs(&ca.lhs, &ca.rhs, lhs_w) {
                    CombItem::CompiledContAssign {
                        compiled: compiler.finish(),
                    }
                } else {
                    if std::env::var("XEZIM_COMPILE_FAIL_STATS").is_ok()
                        && !self.cont_assign_lhs_is_noop(&ca.lhs)
                    {
                        *ca_compile_fail
                            .entry(compiler.bail_reason.unwrap_or("unknown"))
                            .or_insert(0) += 1;
                        if ca_compile_fail_samples < 8 && !self.cont_assign_lhs_is_noop(&ca.lhs) {
                            let lhs_base = Self::base_ident_name(&ca.lhs);
                            eprintln!(
                                "[COMPILE_FAIL_SAMPLE] reason={} lhs_w={} base={:?} array={:?} array2d={:?} signal={:?} lhs={:?} rhs={:?}",
                                compiler.bail_reason.unwrap_or("unknown"),
                                lhs_w,
                                lhs_base,
                                lhs_base.as_ref().and_then(|n| self.module.arrays.get(n)),
                                lhs_base.as_ref().and_then(|n| self.module.arrays_2d.get(n)),
                                lhs_base
                                    .as_ref()
                                    .and_then(|n| self.signal_name_to_id.get(n.as_str()))
                                    .map(|id| (*id, self.signal_widths[*id])),
                                ca.lhs.kind,
                                ca.rhs.kind
                            );
                            ca_compile_fail_samples += 1;
                        }
                    }
                    if self.cont_assign_lhs_is_noop(&ca.lhs) {
                        CombItem::Noop
                    } else {
                        CombItem::ContAssign {
                            lhs: Box::new(ca.lhs),
                            rhs: Box::new(ca.rhs),
                            delay: 0,
                        }
                    }
                }
            } else {
                CombItem::ContAssign {
                    lhs: Box::new(ca.lhs),
                    rhs: Box::new(ca.rhs),
                    delay: 0,
                }
            };

            // Resolve reads, retrying with scope_hint prefix for bare local names.
            // Without this, references like `mem_valid` from a top-level cont_assign
            // would not match `testbench.mem_valid` in signal_name_to_id, and the
            // entry would be marked has_unresolved_reads=true and re-fired every
            // settle iteration.
            let mut rids: Vec<usize> = Vec::with_capacity(reads.len());
            let mut unresolved_count = 0usize;
            // Secondary scope for read resolution, derived from the entry's
            // already-resolved WRITE names. An inlined member-fanout assign
            // (`assign out[k] = port.f;`) can end up with scope_hint=None
            // (absolute LHS / failed inference) while its bare RHS base name
            // ALSO exists at TB level (port named like its connected signal):
            // the dep then landed only on the TB signal, the runtime read the
            // instance-scoped port copy, and the entry never re-fired when
            // that copy updated — the member fanout froze on the pre-update
            // value. Probing the write-side scope registers the scoped id
            // too (superset, dedup'd).
            let write_scope: Option<String> = if scope_hint.is_none() {
                wids.iter()
                    .filter_map(|&id| {
                        let n = self.name_for_id(id);
                        n.rfind('.').map(|p| n[..p].to_string())
                    })
                    .next()
            } else {
                None
            };
            // Top-module name (e.g. "tb") used to detect and strip
            // top-prefixed cross-hierarchical references. Some E902 modules
            // emit `tb.x_soc.biu_pad_hwdata`-style names through xezim's
            // port-rewriting; the signal table stores top-level instances
            // without the top-name prefix (`x_soc.biu_pad_hwdata`), so the
            // lookup misses unless we strip "<top_name>." up front.
            let top_prefix = format!("{}.", self.module.name);
            // Bytecode-compiled entries (CompiledContAssign / FusedGate)
            // resolve bare reads SCOPE-FIRST (bytecode.rs `resolve_ident`):
            // a local `<scope>.<name>` shadows a same-named top signal.
            // The dependency edge MUST resolve to the same id the entry
            // actually reads, else the entry never re-triggers when its
            // true input changes. Direct-copy / fused / delayed-cont-assign
            // entries resolve their reads the bare way (resolve_hier_name_
            // static) — for those, keep bare-first so we don't mis-scope a
            // port-connection source like `u.a = a` (where `a` is genuinely
            // the top signal, not `<scope>.a`).
            let entry_is_bytecode = matches!(&item, CombItem::CompiledContAssign { .. });
            for r in &reads {
                let mut found = false;
                // Root-marked read: the name is absolute by construction —
                // never scope-qualify it (bytecode resolve_ident and the AST
                // resolver both honor the root the same way).
                let read_is_rooted = rooted_reads.contains(r.as_str());
                if entry_is_bytecode && !r.contains('.') && !read_is_rooted {
                    if let Some(scope) = &scope_hint {
                        let q = format!("{}.{}", scope, r);
                        if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                            rids.push(id);
                            continue;
                        }
                    }
                }
                if let Some(&id) = self.signal_name_to_id.get(r.as_str()) {
                    rids.push(id);
                    // An AST-evaluated entry resolves a bare name SCOPE-FIRST at
                    // run time (`resolve_hier_name`), so depending only on the
                    // top-level match is wrong: the entry evaluates `<scope>.r`
                    // but is woken by a change to `r`. It then latched whatever
                    // the scoped signal held on the first settle — X — and never
                    // re-fired when the port copy actually arrived. Register the
                    // scoped id too; a superset sensitivity only costs an extra
                    // evaluation, while missing one loses the value entirely.
                    if !entry_is_bytecode && !r.contains('.') && !read_is_rooted {
                        for sc in [scope_hint.as_ref(), write_scope.as_ref()] {
                            if let Some(scope) = sc {
                                if let Some(&sid) = self
                                    .signal_name_to_id
                                    .get(format!("{}.{}", scope, r).as_str())
                                {
                                    if sid != id && !rids.contains(&sid) {
                                        rids.push(sid);
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                // Bytecode lookup also qualifies unresolved multi-segment
                // relative names with scope_hint. Keep the dependency id in
                // lockstep with the signal id the VM will read; otherwise a
                // successfully compiled entry remains "unresolved" and is
                // needlessly evaluated on every settle call.
                if (!entry_is_bytecode || r.contains('.')) && !read_is_rooted {
                    if let Some(scope) = &scope_hint {
                        let qualified = format!("{}.{}", scope, r);
                        if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                            rids.push(id);
                            found = true;
                        }
                    }
                }
                // Top-prefix strip fallback: `<top>.<rest>` → `<rest>`. Fixes
                // E902 cross-hierarchical refs that prefix-rewriting in
                // elaborate.rs left with the top-module name baked in,
                // even though the simulator stores top-level instance
                // signals without that prefix.
                if !found {
                    if let Some(id) = self.resolve_read_name(r, &top_prefix) {
                        rids.push(id);
                        found = true;
                        // resolve_read_name may have matched via BASE-prefix
                        // stripping ("port.f" -> TB-level "port"). The AST
                        // runtime resolves scope-first, so when the entry's
                        // scope (or its WRITE-side scope, for inlined entries
                        // whose scope_hint inference failed) holds a
                        // same-named port copy, register that id too — else
                        // the entry fires on the TB signal, reads the stale
                        // scoped copy, and never re-fires when the copy
                        // updates (member-fanout assigns froze on the
                        // pre-update port value).
                        let mut cands: Vec<&str> = vec![r.as_str()];
                        {
                            let mut b: &str = r.as_str();
                            while let Some(p) = b.rfind('.') {
                                b = &b[..p];
                                cands.push(b);
                            }
                        }
                        for sc in [scope_hint.as_ref(), write_scope.as_ref()] {
                            if let Some(scope) = sc {
                                for c in &cands {
                                    let q = format!("{}.{}", scope, c);
                                    if let Some(&sid) = self.signal_name_to_id.get(q.as_str()) {
                                        if !rids.contains(&sid) {
                                            rids.push(sid);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // A MEMBER read ("p.lo") has no signal of its own — it is a
                // slice of its base. Depend on the BASE signal so the entry
                // re-fires when it changes; dropping the read entirely left
                // pattern CAs (`assign r = '{hi: p.lo, …}`) frozen at their
                // time-0 value.
                if !found && r.contains('.') {
                    let mut base: &str = r.as_str();
                    while let Some(pos) = base.rfind('.') {
                        base = &base[..pos];
                        // Register BOTH the instance-scoped and the bare/top
                        // resolution of the base — same superset rule as the
                        // whole-read path above. When a TB signal and the
                        // instance port SHARE a name, scoped-or-bare picked
                        // one id; the entry then fired only on that one and
                        // read the OTHER (stale) copy — a member-fanout
                        // assign latched the pre-update port value and never
                        // re-fired when the port copy arrived. `write_scope`
                        // covers the scope_hint=None inlined case.
                        for sc in [scope_hint.as_ref(), write_scope.as_ref()] {
                            if let Some(scope) = sc {
                                let q = format!("{}.{}", scope, base);
                                if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                                    if !rids.contains(&id) {
                                        rids.push(id);
                                    }
                                    found = true;
                                }
                            }
                        }
                        if let Some(id) = self.resolve_read_name(base, &top_prefix) {
                            if !rids.contains(&id) {
                                rids.push(id);
                            }
                            found = true;
                        }
                        if found {
                            break;
                        }
                    }
                }
                if !found {
                    // Last chance: the FULL runtime resolver, which knows the
                    // §23.8/§23.10.1 upward walk a bound module's references
                    // need (`assign x = target_child.sig;` inside a bind).
                    // The static attempts above left such reads unresolved,
                    // so the entry evaluated with an EMPTY read set — right
                    // values, wrong times (it only re-ran when unrelated
                    // settle activity happened to occur).
                    let hier = HierarchicalIdentifier {
                        root: None,
                        path: r
                            .split('.')
                            .map(|seg| HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: seg.to_string(),
                                    span: crate::ast::Span::dummy(),
                                },
                                selects: Vec::new(),
                            })
                            .collect(),
                        span: crate::ast::Span::dummy(),
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    };
                    let saved_hint = self.name_resolve_hint.borrow().clone();
                    *self.name_resolve_hint.borrow_mut() = scope_hint.clone();
                    let full = self.resolve_hier_name(&hier);
                    *self.name_resolve_hint.borrow_mut() = saved_hint;
                    if let Some(&id) = self.signal_name_to_id.get(full.as_ref()) {
                        if !rids.contains(&id) {
                            rids.push(id);
                        }
                        found = true;
                    }
                }
                if !found {
                    // A whole UNPACKED-ARRAY read (`assign y = '{a[0], a[1]}`
                    // naming `a`, or a port carrying one) has no signal under
                    // the BASE name — the array occupies a contiguous id range
                    // recorded in `array_first_id`. Every lookup above works on
                    // `signal_name_to_id`, so the base stayed unresolved, which
                    // pins `has_unresolved_reads` and re-evaluates the entry on
                    // EVERY settle call for the life of the run. Measured on
                    // Ibex CoreMark: one such entry (ex_block's
                    // `alu_imd_val_q` reading `imd_val_q_i`) was interpreted
                    // 1.53M times for 6.2s — ~17% of all settle time.
                    //
                    // A cont-assign's RHS resolves in the SAME scope as its
                    // LHS, so the LHS's own hierarchical prefix is the most
                    // reliable qualifier; the inferred `scope_hint` can point
                    // at a submodule that merely happens to share the port
                    // name.
                    let lhs_scope: Option<&str> = writes
                        .iter()
                        .next()
                        .and_then(|w| w.rsplit_once('.').map(|(head, _)| head));
                    let mut cands: Vec<String> = Vec::with_capacity(4);
                    if let Some(ls) = lhs_scope {
                        cands.push(format!("{}.{}", ls, r));
                    }
                    if let Some(scope) = &scope_hint {
                        cands.push(format!("{}.{}", scope, r));
                    }
                    cands.push(r.clone());
                    for cand in &cands {
                        let Some(&(first_id, lo, hi)) = self.array_first_id.get(cand.as_str())
                        else {
                            continue;
                        };
                        // A packed-arena array has no per-cell signal ids to
                        // depend on: leave the read UNRESOLVED so the entry
                        // re-evaluates on every settle — exactly how a read of
                        // the same memory through a dynamic index (whose
                        // per-element names never resolve) is tracked.
                        if is_packed_id(first_id) {
                            break;
                        }
                        let count = (hi - lo + 1).max(0) as usize;
                        for k in 0..count {
                            let id = first_id + k;
                            if id < self.signal_table.len() && !rids.contains(&id) {
                                rids.push(id);
                            }
                        }
                        found = true;
                        break;
                    }
                    if !found {
                        // A TYPE name is not a signal and can never resolve.
                        // Several cast lowerings leak the cast's type into the
                        // read set (`ibex_mubi_t'(x)`, `cpu_ctrl_sts_part_t'(x)`
                        // on Ibex), which pinned `has_unresolved_reads` and
                        // re-evaluated the entry on every settle call forever.
                        // Recognising type-ness here covers every AST shape a
                        // cast can take, rather than special-casing one.
                        // Reached only after every signal lookup failed, so a
                        // name that is BOTH a type and a signal already
                        // resolved as the signal above.
                        let leaf = r.rsplit('.').next().unwrap_or(r.as_str());
                        if self.module.typedef_types.contains_key(leaf)
                            || self.module.typedefs.contains_key(leaf)
                        {
                            continue;
                        }
                    }
                    if !found && dump_unresolved {
                        let leaf = r.rsplit('.').next().unwrap_or(r.as_str());
                        let near: Vec<&str> = self
                            .array_first_id
                            .keys()
                            .filter(|k| k.rsplit('.').next() == Some(leaf))
                            .map(|k| k.as_ref())
                            .take(4)
                            .collect();
                        eprintln!(
                            "[UNRES-ARR] read {r:?} not an array under {cands:?};                              array_first_id keys with leaf {leaf:?}: {near:?}"
                        );
                    }
                }
                if !found {
                    unresolved_count += 1;
                }
            }
            if dump_ca_reads {
                let rn: Vec<&str> = rids.iter().map(|&i| self.name_for_id(i)).collect();
                eprintln!(
                    "[CA-READS] scope_hint={:?} reads={:?} rids={:?} lhs_abs={}",
                    scope_hint, reads, rn, lhs_is_absolute
                );
            }
            let has_unresolved_reads = unresolved_count > 0;
            if has_unresolved_reads && dump_unresolved {
                let unresolved: Vec<&String> = reads
                    .iter()
                    .filter(|r| {
                        if self.signal_name_to_id.contains_key((*r).as_str()) {
                            return false;
                        }
                        if let Some(scope) = &scope_hint {
                            if self
                                .signal_name_to_id
                                .contains_key(format!("{}.{}", scope, r).as_str())
                            {
                                return false;
                            }
                        }
                        true
                    })
                    .collect();
                eprintln!(
                    "[UNRES] cont_assign scope={:?} unresolved={:?} resolved={}/{}",
                    scope_hint,
                    unresolved,
                    rids.len(),
                    reads.len()
                );
            }
            if matches!(&item, CombItem::Noop) {
                continue;
            }
            let cov = cov_counter.filter(|_| matches!(&item, CombItem::ContAssign { .. }));
            entries.push(CombEntry {
                item,
                cold: Box::new(CombEntryCold {
                    scope_hint,
                    ts_scope,
                    read_signal_ids: rids,
                    write_signal_ids: wids,
                    span: ca_span,
                    cov,
                }),
                has_unresolved_reads,
                defer_at_time0: false,
            });
        }

        // §6.6.1 vs a DRIVEN net: an undriven net reads z, but a net that has a
        // continuous driver is not undriven — its value is whatever that driver
        // produces, and until the driver first resolves that is UNKNOWN, not
        // high-impedance. The distinction is invisible for a zero-delay driver
        // (the time-0 settle overwrites it immediately) but shows through the
        // whole delay window of a DELAYED one, which is exactly a gate-level
        // netlist: `and #(2,5) g(o, a, b);` leaves `o` readable for 5 ticks
        // before it first resolves, and it should read x there.
        //
        // A combinational CYCLE (`assign p = q; assign q = p;`) keeps this
        // x forever — measured: the reference does the SAME (cycles read x,
        // driven or not). `alias` is NOT a cycle — it unifies names onto one
        // net (see `alias_pairs`).
        //
        // Only all-z signals are touched, so a net given a real initial value
        // (supply0/1) keeps it. A tri0/tri1 net is no different from a wire
        // here: its pull is only the weak fallback where no driver drives, so
        // with a driver it is x as well until that driver resolves (measured
        // on the reference: `tri0 t; assign #2 t = a;` reads x until t=2).
        let cont_driven: Vec<usize> = self.cont_driven.iter().copied().collect();
        let has_pulled = self.module.resolved_net_kinds.values().any(|k| {
            matches!(
                k,
                xezim_core::elaborate::ResolvedNetKind::Tri0
                    | xezim_core::elaborate::ResolvedNetKind::Tri1
            )
        });
        for id in cont_driven {
            if self.signal_real[id] {
                continue;
            }
            let w = self.signal_widths[id];
            if w == 0 {
                continue;
            }
            let cur = &self.signal_table[id];
            let all_z = (0..w as usize).all(|i| cur.get_bit(i) == LogicBit::Z);
            let pulled = has_pulled
                && !all_z
                && self
                    .name_opt(id)
                    .and_then(|n| self.module.resolved_net_kinds.get(n))
                    .is_some_and(|k| {
                        matches!(
                            k,
                            xezim_core::elaborate::ResolvedNetKind::Tri0
                                | xezim_core::elaborate::ResolvedNetKind::Tri1
                        )
                    });
            if all_z || pulled {
                let mut v = Value::new(w); // all-x
                v.is_signed = self.signal_signed[id];
                self.signal_table[id] = v;
                self.sync_mirror(id);
            }
        }

        // Always @* and always_comb blocks. Reuse `reads`/`writes` from the
        // continuous_assigns loop above (clear-not-realloc). always_latch is
        // wired into the same path: the `if (cond) lhs <= rhs;` body is
        // level-sensitive on its conditions just like an always_comb, and
        // NBA semantics preserve the LHS when the condition is false (the
        // standard latch behaviour). Symptom of missing this: cv32e40p's
        // `cv32e40p_sim_clock_gate` clk_en stayed X, so the gated core
        // clock never propagated and the controller FSM never advanced.
        if std::env::var("XEZIM_COMPILE_PHASES").is_ok() {
            eprintln!(
                "[COMPILE-PHASE]   cont-assign compile loop: {:.1}ms ({} assigns)",
                cont_loop_t0.elapsed().as_secs_f64() * 1000.0,
                entries.len()
            );
        }
        for ab in &self.module.always_blocks {
            if matches!(
                ab.kind,
                AlwaysKind::AlwaysComb | AlwaysKind::Always | AlwaysKind::AlwaysLatch
            ) {
                let is_always_comb =
                    matches!(ab.kind, AlwaysKind::AlwaysComb | AlwaysKind::AlwaysLatch);
                reads.clear();
                writes.clear();
                if !is_always_comb {
                    SKIP_FN_BODY_READS.with(|c| c.set(true));
                }
                Self::collect_stmt_reads(&ab.stmt, &self.module, &mut reads, &mut writes);
                SKIP_FN_BODY_READS.with(|c| c.set(false));
                // Top-prefix-strip helper: `<top>.<rest>` → `<rest>`. Used
                // to resolve hierarchical refs whose path includes the
                // top-module name (e.g. `tb.x_soc.foo`) where the signal
                // table stores top-level instance signals without that
                // prefix (`x_soc.foo`). Same fix as the cont_assign read
                // resolver above — without it, deeply-nested CPU
                // always_ff blocks that observe cross-hierarchical
                // signals via xezim's port-rewritten absolute paths
                // never get the source signal in their dep graph and
                // miss re-triggering. Symptom on E902: CPU stalls after
                // 2 instructions because the bus-arbiter always_ff
                // doesn't re-fire when its sensitivity inputs change.
                let top_prefix = format!("{}.", self.module.name);
                let resolve_one =
                    |name: &str| -> Option<usize> { self.resolve_read_name(name, &top_prefix) };
                // The block's own inlining scope is authoritative; infer from
                // the read/write sets only when it has none (top module).
                // This mirrors the edge-block path, which has preferred
                // `block.scope` for the same reason. Inference picks the first
                // scope that explains the r/w names, so with two instances of
                // one module BOTH comb blocks came back as the FIRST
                // instance's scope — `u_beta`'s block reported `u_alpha`,
                // which mis-scoped its bare names and made `%m` name the wrong
                // instance.
                let scope_hint = if !ab.scope.is_empty() {
                    Some(ab.scope.clone())
                } else {
                    self.infer_scope_from_rw_sets(&writes, &reads)
                };
                if std::env::var("XEZIM_AB_DBG").is_ok() {
                    let unresolved: Vec<&String> = reads
                        .iter()
                        .filter(|r| {
                            !self.signal_name_to_id.contains_key(r.as_str())
                                && !self
                                    .signal_name_to_id
                                    .contains_key(format!("{}.{}", self.module.name, r).as_str())
                        })
                        .collect();
                    eprintln!(
                        "[ABDBG] always-block writes={:?} reads={:?} -> scope={:?} unresolved={:?}",
                        writes, reads, scope_hint, unresolved
                    );
                }
                let wids: Vec<usize> = writes
                    .iter()
                    .filter_map(|w| resolve_one(w.as_str()))
                    .collect();
                // For comb-sensitivity purposes, exclude signals that are written by
                // this block. Loop variables and local temps are written-then-read
                // within a single execution; external re-triggering on them would
                // cause infinite settle loops (e.g. `for (j = 0; j < N; j++)` in an
                // always @* block).
                //
                // §9.2.2.2.1: the exclusion must also hold at MEMBER granularity —
                // `out = in; if (out.vld) out.f = …;` reads `out.vld`, which is not
                // string-equal to any write ("out"/"out.f") but IS a read of a
                // variable this block writes. Keeping it as a dependency makes the
                // block re-queue itself every settle pass (the pass-through write
                // then the field write commit two different values per eval), which
                // churns to the settle limit. Strip a read whose '.'-base prefix is
                // in the write set.
                let sens_reads: HashSet<String> = reads
                    .iter()
                    .filter(|r| {
                        if writes.contains(*r) {
                            return false;
                        }
                        let mut base: &str = r.as_str();
                        while let Some(pos) = base.rfind('.') {
                            base = &base[..pos];
                            if writes.contains(base) {
                                return false;
                            }
                        }
                        true
                    })
                    .cloned()
                    .collect();
                // A bare read may resolve BOTH to a top-level signal and to
                // this block's instance-scoped one ("trig" vs "u_s.trig") —
                // the bytecode resolves scope-FIRST at runtime, so the dep
                // graph must include the scoped id or the block fires on the
                // wrong signal's change and never re-fires when its actual
                // input (fed by the port's continuous assign) settles later.
                // Register BOTH: a superset sensitivity is always safe.
                let mut rids: Vec<usize> = Vec::new();
                for r in sens_reads.iter() {
                    if let Some(scope) = &scope_hint {
                        let q = format!("{}.{}", scope, r);
                        if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                            if !rids.contains(&id) {
                                rids.push(id);
                            }
                        }
                    }
                    // The block's own INSTANCE scope is authoritative even
                    // when the read/write leaf inference above found nothing
                    // (e.g. every leaf is a loop variable). A port named like
                    // its connected signal ("grant" → tb `grant`) resolves at
                    // runtime to the instance's port copy `dut.grant`, which
                    // updates a settle-cascade step LATER than the top signal
                    // — without the scoped id in the dep graph the block reads
                    // one NBA application behind (a FIFO level tracker lagged
                    // its reference model by one clock).
                    if !ab.scope.is_empty() && Some(&ab.scope) != scope_hint.as_ref() {
                        let q = format!("{}.{}", ab.scope, r);
                        if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                            if !rids.contains(&id) {
                                rids.push(id);
                            }
                        }
                    }
                    let before = rids.len();
                    if let Some(id) = resolve_one(r.as_str()) {
                        if !rids.contains(&id) {
                            rids.push(id);
                        }
                    }
                    // §23.8 / §23.10.1: a dotted read that resolved nowhere
                    // may be an UPWARD reference (a bound module reading its
                    // host through the host's module name — `host_mod.clk`
                    // from a monitor bound under a `host_mod` instance). Run
                    // the full hierarchical resolution anchored at the
                    // block's own scope, or the entry gets no dependency
                    // edge at all and never re-fires after time 0.
                    if rids.len() == before && r.contains('.') {
                        let span = crate::ast::Span::dummy();
                        let ident = crate::ast::expr::HierarchicalIdentifier {
                            root: None,
                            path: vec![crate::ast::expr::HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: r.clone(),
                                    span,
                                },
                                selects: Vec::new(),
                            }],
                            span,
                            cached_signal_id: std::cell::Cell::new(None),
                            cached_resolved_name: std::cell::OnceCell::new(),
                        };
                        let saved = self.name_resolve_hint.borrow().clone();
                        if !ab.scope.is_empty() {
                            *self.name_resolve_hint.borrow_mut() = Some(ab.scope.clone());
                        }
                        let resolved = self.resolve_hier_name(&ident);
                        *self.name_resolve_hint.borrow_mut() = saved;
                        if resolved != *r {
                            if let Some(&id) = self.signal_name_to_id.get(resolved.as_ref()) {
                                if !rids.contains(&id) {
                                    rids.push(id);
                                }
                            }
                        }
                    }
                    // A MEMBER read ("req.vld") has no signal of its own —
                    // depend on the BASE signal, mirroring the cont-assign
                    // path. Without this, a block whose ONLY reads of a
                    // struct are member-granular got NO dependency on it and
                    // never re-fired on its changes (a multi-slot queue write
                    // enable computed from req.vld/req.id missed every
                    // request; state updates silently froze). Reads whose
                    // base the block writes were already stripped above.
                    if rids.len() == before && r.contains('.') {
                        let mut base: &str = r.as_str();
                        while let Some(pos) = base.rfind('.') {
                            base = &base[..pos];
                            let mut found = false;
                            if let Some(scope) = &scope_hint {
                                let q = format!("{}.{}", scope, base);
                                if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                                    if !rids.contains(&id) {
                                        rids.push(id);
                                    }
                                    found = true;
                                }
                            }
                            if !ab.scope.is_empty() && Some(&ab.scope) != scope_hint.as_ref() {
                                let q = format!("{}.{}", ab.scope, base);
                                if let Some(&id) = self.signal_name_to_id.get(q.as_str()) {
                                    if !rids.contains(&id) {
                                        rids.push(id);
                                    }
                                    found = true;
                                }
                            }
                            if let Some(id) = resolve_one(base) {
                                if !rids.contains(&id) {
                                    rids.push(id);
                                }
                                found = true;
                            }
                            if found {
                                break;
                            }
                        }
                    }
                }
                if std::env::var("XEZIM_DUMP_COMB_SENS").is_ok() {
                    let names: Vec<&str> = rids.iter().map(|&id| self.name_for_id(id)).collect();
                    eprintln!(
                        "[COMB-SENS] scope='{}' reads={:?} writes={:?} sens_reads={:?} rids={:?}",
                        ab.scope, reads, writes, sens_reads, names
                    );
                }
                // Unresolved reads in always @* are usually parameters, genvars,
                // typedefs, or loop-local integer variables — none of which change
                // at runtime. Don't mark the block as has_unresolved_reads for
                // those cases; it causes the block to fire every settle iteration
                // and can produce infinite settle loops when the block itself
                // writes temporary variables (loop indices, scratch regs).
                // The exception is a read of a memory cell stored without an
                // element name (packed arena, unnamed bulk array): it changes
                // at run time but no dependency edge can name it, so the block
                // re-evaluates on every settle — the way a continuous assign
                // reading one is tracked (§10.3; `packed_store` keeps such
                // settles running).
                let has_unresolved_reads = sens_reads
                    .iter()
                    .any(|r| self.read_names_unnamed_cell(r, scope_hint.as_deref(), &ab.scope));
                // Try bytecode-compiling the comb always block. On success
                // the settle path skips exec_statement entirely and runs
                // the flat Insn stream via exec_insns.
                let item = {
                    let mut compiler = super::bytecode::BytecodeCompiler::new(
                        &self.signal_name_to_id,
                        &self.signal_signed,
                        &self.signal_widths,
                        &self.module.arrays,
                        &self.widths,
                    );
                    compiler.set_scope_hint(scope_hint.clone());
                    compiler.set_tasks(&self.module.tasks);
                    compiler.set_functions(&self.module.functions);
                    compiler.set_params(&self.module.parameters);
                    compiler.set_param_leaf_idx(self.param_leaf_index());
                    compiler.set_cast_widths(&self.cast_widths);
                    compiler.set_packed_struct_fields(
                        self.bc_packed_fields
                            .as_ref()
                            .unwrap_or(&self.module.packed_struct_fields),
                    );
                    compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
                    compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
                    compiler.set_typedef_types(&self.module.typedef_types);
                    compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
                    compiler.set_assoc_arrays(&self.module.associative_arrays);
                    compiler.set_packed_full_dims(&self.module.packed_full_dims);
                    compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
                    compiler.set_arrays_2d(&self.module.arrays_2d);
                    compiler.set_collection_denies(
                        &self.module.dynamic_arrays,
                        &self.module.queue_vars,
                    );
                    compiler.set_array_first_id(&self.array_first_id);
                    compiler.set_string_signals(&self.module.string_signals);
                    compiler.set_signal_real(&self.signal_real);
                    compiler.top_module_name = Some(self.module.name.clone());
                    // Enable AST fallback so partially-unsupported constructs
                    // compile to StmtFallback insns instead of failing the
                    // whole block. Simple parts still benefit from bytecode
                    // signal-ID pre-resolution.
                    compiler.set_ast_fallback(true);
                    if compiler.compile_stmt(&ab.stmt) {
                        CombItem::CompiledAlwaysBlock {
                            compiled: compiler.finish(),
                            is_always_comb,
                        }
                    } else {
                        CombItem::AlwaysBlock {
                            stmt: Box::new(ab.stmt.clone()),
                            is_always_comb,
                        }
                    }
                };
                let _is_pure_observer = wids.is_empty();
                entries.push(CombEntry {
                    item,
                    cold: Box::new(CombEntryCold {
                        scope_hint,
                        // An always block's hint is its own instance scope.
                        ts_scope: None,
                        read_signal_ids: rids,
                        write_signal_ids: wids,
                        span: ab.stmt.span,
                        cov: None,
                    }),
                    has_unresolved_reads,
                    // §9.2.2.1: a level-only `always @(sensitivity)` block
                    // suspends at its event control initially and runs only on
                    // a sensitivity CHANGE — declaration init is not an event
                    // (§6.8). xezim routes such blocks through the comb-settle
                    // path, which would fire them once at t0 against their X
                    // init. For a block that WRITES a signal this is
                    // observationally harmless (X-in -> X-out, recomputed on
                    // the first real change), so keep firing it (combinational
                    // modeling relies on t0 settle). For a pure OBSERVER
                    // (no signal write: `always @(y) $display(...)`) or one
                    // that can $finish, the t0 fire is a spurious side effect a
                    // reference simulator does not produce — defer it.
                    // §9.2.2.1/§9.2.2.2.2: a PLAIN `always` (star or list)
                    // suspends at its event control first — it does NOT run
                    // at time 0; only always_comb/always_latch do. The old
                    // compromise fired writer blocks at t0 anyway; the
                    // reference leaves their outputs X until a real input
                    // change, and now so do we.
                    defer_at_time0: !is_always_comb,
                });
            }
        }

        // Add UDPs before ordering so library-cell chains participate in the
        // same writer-before-reader schedule as continuous assignments.
        self.build_udp_entries(&mut entries);
        self.build_timing_check_entries(&mut entries);
        self.push_timing_watch_entries(&mut entries);
        // Stateful and generic UDP runtimes have observable initialization and
        // event ordering. Keep their established post-combinational position;
        // only proven-pure fused gates participate in the topological pass.
        // Timing checks ride along, so a `&&&` condition reads settled logic.
        let mut deferred_udp_entries = Vec::new();
        let mut orderable_entries = Vec::with_capacity(entries.len());
        for entry in entries.drain(..) {
            if matches!(
                entry.item,
                CombItem::Udp { .. } | CombItem::UdpBatch { .. } | CombItem::TimingCheck { .. }
            ) {
                deferred_udp_entries.push(entry);
            } else {
                orderable_entries.push(entry);
            }
        }
        entries = orderable_entries;

        // Topologically reorder `entries` so that writers come before readers
        // where possible. This collapses feedforward chains to 1 settle iter.
        // Cycles are broken arbitrarily; feedback still needs multi-iter.
        let num_signals = self.signal_table.len();
        {
            let n = entries.len();
            // writers_by_sig was previously `Vec<Vec<usize>>` sized to
            // num_signals — a HashMap is cheaper here because at most
            // a few percent of signal_ids are ever written by a comb
            // entry (most are pure inputs / clock / RAM cells).
            // 36 M × 24 B Vec<usize> headers = 864 MB transient on c910;
            // HashMap drops to a few hundred MB at most.
            let mut writers_by_sig: HashMap<usize, Vec<usize>> = HashMap::default();
            for (idx, entry) in entries.iter().enumerate() {
                for &sig_id in &entry.cold.write_signal_ids {
                    if sig_id < num_signals {
                        writers_by_sig.entry(sig_id).or_default().push(idx);
                    }
                }
            }
            // Build forward graph: for each entry B, predecessors = writers of its read signals.
            // Store successors as a compact CSR adjacency list. The previous
            // `Vec<Vec<usize>>` paid a Vec header for every comb entry and
            // produced many small allocations on large flattened designs.
            // A single net with thousands of combinational writers is never real
            // RTL (one driver per net is the norm). It arises when a parameter
            // underflows so a `for i: assign bus[i] = …` generate-loop's index
            // select collapses — `bus` clamped to 1 bit makes every `bus[i]`
            // alias the SAME signal id, yielding W writers on one net and a
            // W × R dependency-edge explosion (black-parrot's lru_decode.mask_o:
            // 10000 × 20000 = 200 M edges → 1.6 GB → OOM). Cap the predecessor
            // fan-in from any such degenerate net; its entries still converge via
            // the unresolved-read re-fire path, just without precise topo order.
            const MAX_WRITERS_PER_NET: usize = 1024;
            // A net with W writers and R readers contributes W × R
            // writer -> reader edges. A vector written bit by bit and read
            // whole by many entries makes that product explode (a DRAM mat's
            // 128 per-row `rs[r]` drivers × 4,480 readers: 27 M edges at 64k
            // cells, most of the compile's peak memory). Such a net gets ONE
            // join node instead: writers -> join -> readers, W + R edges that
            // carry exactly the same ordering constraints transitively. Join
            // nodes are ordered like entries but never emitted. A reader that
            // is itself one of the net's writers keeps its direct edges (a
            // join edge back to it would close a false cycle). Nets below the
            // product threshold keep the plain edges, so ordinary graphs, and
            // their tie-breaks in cyclic regions, are unchanged.
            const TOPO_JOIN_MIN_EDGES: usize = 1 << 14;
            let topo_join_on = !matches!(std::env::var("XEZIM_TOPO_JOIN").as_deref(), Ok("0"));
            // Keyed by a type of its own: sharing the monomorphized maps the
            // run-time paths use (`HashMap<usize, u32>` is the NBA index)
            // moved their inlining and cost the simulation ~1% on c906.
            #[derive(Clone, Copy, PartialEq, Eq, Hash)]
            struct NetKey(usize);
            // A multi-writer net's (writers, readers), then its join node:
            // u32::MAX until the first reader creates it.
            let mut join_of: HashMap<NetKey, u32> = HashMap::default();
            if topo_join_on {
                let mut counts: HashMap<NetKey, (usize, usize)> = HashMap::default();
                for entry in entries.iter() {
                    for &sid in &entry.cold.write_signal_ids {
                        if sid < num_signals {
                            counts.entry(NetKey(sid)).or_insert((0, 0)).0 += 1;
                        }
                    }
                }
                for entry in entries.iter() {
                    for &sid in &entry.cold.read_signal_ids {
                        if let Some(c) = counts.get_mut(&NetKey(sid)) {
                            c.1 += 1;
                        }
                    }
                }
                for (key, (w, r)) in counts {
                    if (2..=MAX_WRITERS_PER_NET).contains(&w)
                        && w.saturating_mul(r) >= TOPO_JOIN_MIN_EDGES
                    {
                        join_of.insert(key, u32::MAX);
                    }
                }
            }
            // `writers` is ascending (pushed in entry order).
            fn is_writer(writers: &[usize], b: usize) -> bool {
                let (mut lo, mut hi) = (0usize, writers.len());
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if writers[mid] < b {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                lo < writers.len() && writers[lo] == b
            }
            let mut indeg = vec![0usize; n];
            // Use a temporary visited matrix row-by-row to avoid double-counting edges.
            let mut seen_pred: Vec<u32> = vec![u32::MAX; n];
            let mut edges: Vec<(u32, u32)> = Vec::new();
            let mut capped_warned = false;
            for b in 0..n {
                for &sid in &entries[b].cold.read_signal_ids {
                    if sid >= num_signals {
                        continue;
                    }
                    if let Some(writers) = writers_by_sig.get(&sid) {
                        if writers.len() > MAX_WRITERS_PER_NET {
                            if !capped_warned {
                                capped_warned = true;
                                std::eprintln!(
                                    "[xezim][warning] net sid={} has {} combinational \
                                    writers (> {}); skipping its dependency edges — likely a \
                                    parameter-underflow generate-loop artifact",
                                    sid,
                                    writers.len(),
                                    MAX_WRITERS_PER_NET
                                );
                            }
                            continue;
                        }
                        if let Some(slot) = join_of.get_mut(&NetKey(sid)) {
                            if !is_writer(writers, b) {
                                if *slot == u32::MAX {
                                    let j = indeg.len();
                                    *slot = j as u32;
                                    indeg.push(0);
                                    seen_pred.push(u32::MAX);
                                    let mut prev = usize::MAX;
                                    for &a in writers {
                                        if a != prev {
                                            edges.push((a as u32, j as u32));
                                            indeg[j] += 1;
                                            prev = a;
                                        }
                                    }
                                }
                                let j = *slot as usize;
                                if seen_pred[j] != b as u32 {
                                    seen_pred[j] = b as u32;
                                    edges.push((j as u32, b as u32));
                                    indeg[b] += 1;
                                }
                                continue;
                            }
                        }
                        for &a in writers {
                            if a == b {
                                continue;
                            }
                            if seen_pred[a] == b as u32 {
                                continue;
                            }
                            seen_pred[a] = b as u32;
                            edges.push((a as u32, b as u32));
                            indeg[b] += 1;
                        }
                    }
                }
            }
            drop(join_of);
            drop(seen_pred);
            let n_nodes = indeg.len();
            if std::env::var_os("XEZIM_COMPILE_PHASES").is_some() {
                eprintln!(
                    "[COMPILE-PHASE]   topo-order graph: {} entries, {} join nodes, {} edges ({:.1} MB)",
                    n,
                    n_nodes - n,
                    edges.len(),
                    edges.capacity() as f64 * 8.0 / 1048576.0
                );
            }
            crate::rss_trace("bce: topo edges built");
            let mut succ_offsets = vec![0u32; n_nodes + 1];
            for &(a, _) in &edges {
                succ_offsets[a as usize + 1] += 1;
            }
            for i in 1..succ_offsets.len() {
                succ_offsets[i] += succ_offsets[i - 1];
            }
            let mut succ_entries = vec![0u32; edges.len()];
            let mut succ_cursor = succ_offsets[..n_nodes].to_vec();
            for (a, b) in edges {
                let pos = succ_cursor[a as usize] as usize;
                succ_entries[pos] = b;
                succ_cursor[a as usize] += 1;
            }
            // Kahn's topological sort with a degree-bucketed selector. Always
            // emits the lowest-indegree unplaced node: indegree 0 = a normal
            // ready node; indegree >0 only when the remaining graph is fully
            // cyclic, where the min-indegree node is the cheapest cycle edge to
            // break (fewest residual back-edges → fewest extra settle passes).
            //
            // The previous version scanned all n entries for the min on every
            // stall (O(n) per cycle break → O(n²) on designs with many small
            // combinational loops). Buckets give the same min-indegree choice
            // in O(V+E): `buckets[d]` holds candidates believed to have indegree
            // `d`; entries are lazily invalidated (a node placed, or whose
            // indegree has since dropped, is skipped on pop). `min_d` tracks the
            // lowest bucket that may hold a live node and only moves down when a
            // decrement creates a lower-degree node.
            let mut new_order: Vec<usize> = Vec::with_capacity(n);
            let mut placed = vec![false; n_nodes];
            let max_d = indeg.iter().copied().max().unwrap_or(0);
            let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); max_d + 1];
            for i in 0..n_nodes {
                buckets[indeg[i]].push(i as u32);
            }
            let mut min_d = 0usize;
            let mut n_placed = 0usize;
            while n_placed < n_nodes {
                // Advance to the lowest bucket holding a live (unplaced,
                // current-indegree-matches) node, discarding stale entries.
                let mut a = usize::MAX;
                while min_d <= max_d {
                    while let Some(&top) = buckets[min_d].last() {
                        let t = top as usize;
                        if placed[t] || indeg[t] != min_d {
                            buckets[min_d].pop();
                        } else {
                            a = t;
                            buckets[min_d].pop();
                            break;
                        }
                    }
                    if a != usize::MAX {
                        break;
                    }
                    min_d += 1;
                }
                if a == usize::MAX {
                    break;
                }
                placed[a] = true;
                n_placed += 1;
                if a < n {
                    new_order.push(a);
                }
                let lo = succ_offsets[a] as usize;
                let hi = succ_offsets[a + 1] as usize;
                for &b_u32 in &succ_entries[lo..hi] {
                    let b = b_u32 as usize;
                    if placed[b] {
                        continue;
                    }
                    if indeg[b] > 0 {
                        indeg[b] -= 1;
                        let nb = indeg[b];
                        buckets[nb].push(b as u32);
                        if nb < min_d {
                            min_d = nb;
                        }
                    }
                }
            }
            if std::env::var_os("XEZIM_COMPILE_PHASES").is_some() {
                let h = new_order.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &i| {
                    (h ^ i as u64).wrapping_mul(0x0100_0000_01b3)
                });
                eprintln!("[COMPILE-PHASE]   topo-order hash: {h:016x}");
            }
            // Apply permutation only if we got a real full permutation.
            let valid_permutation = if new_order.len() == n {
                let mut seen = vec![false; n];
                let mut ok = true;
                for &i in &new_order {
                    if i >= n || seen[i] {
                        ok = false;
                        break;
                    }
                    seen[i] = true;
                }
                ok
            } else {
                false
            };
            if valid_permutation {
                let mut permuted: Vec<CombEntry> = Vec::with_capacity(n);
                // Take entries out via swap_remove would mangle indices; rebuild by index.
                // Use an Option<CombEntry> trick.
                let mut slots: Vec<Option<CombEntry>> = entries.into_iter().map(Some).collect();
                for &i in &new_order {
                    permuted.push(slots[i].take().unwrap());
                }
                entries = permuted;
            } else if !new_order.is_empty() && n > 0 {
                eprintln!(
                    "[xezim][comb] skipping invalid reorder permutation: len={} entries={}",
                    new_order.len(),
                    n
                );
            }
        }
        entries.extend(deferred_udp_entries);

        // Collapse replicated exact-width copies with a common source into a
        // single worklist entry. Only single-writer destinations qualify: a
        // multiply-driven destination can depend on procedural ordering and
        // must retain its original entry position. Four copies is the minimum
        // that amortizes the boxed destination list and fanout dispatch.
        let mut writer_counts = vec![0u32; num_signals];
        for entry in &entries {
            for &dst in &entry.cold.write_signal_ids {
                if dst < writer_counts.len() {
                    writer_counts[dst] = writer_counts[dst].saturating_add(1);
                }
            }
        }
        let mut copies_by_src: HashMap<usize, Vec<(usize, usize)>> = HashMap::default();
        for (eidx, entry) in entries.iter().enumerate() {
            if let CombItem::FastDirectCopy { dst_id, src_id } = &entry.item {
                if dst_id != src_id && writer_counts.get(*dst_id).copied() == Some(1) {
                    copies_by_src
                        .entry(*src_id)
                        .or_default()
                        .push((eidx, *dst_id));
                }
            }
        }
        let mut fanout_replacement: Vec<Option<(usize, Box<[usize]>)>> = vec![None; entries.len()];
        let mut fanout_skip = vec![false; entries.len()];
        let mut fanout_groups = 0usize;
        let mut fanout_copies = 0usize;
        for (src_id, copies) in copies_by_src {
            if copies.len() < 4 {
                continue;
            }
            let first = copies[0].0;
            let dst_ids: Vec<usize> = copies.iter().map(|(_, dst)| *dst).collect();
            for &(eidx, _) in copies.iter().skip(1) {
                fanout_skip[eidx] = true;
            }
            fanout_groups += 1;
            fanout_copies += dst_ids.len();
            fanout_replacement[first] = Some((src_id, dst_ids.into_boxed_slice()));
        }
        if fanout_groups != 0 {
            let mut grouped = Vec::with_capacity(entries.len() - fanout_copies + fanout_groups);
            for (eidx, mut entry) in entries.into_iter().enumerate() {
                if fanout_skip[eidx] {
                    continue;
                }
                if let Some((src_id, dst_ids)) = fanout_replacement[eidx].take() {
                    entry.cold.read_signal_ids.clear();
                    entry.cold.read_signal_ids.push(src_id);
                    entry.cold.write_signal_ids.clear();
                    entry.cold.write_signal_ids.extend_from_slice(&dst_ids);
                    entry.has_unresolved_reads = false;
                    entry.cold.scope_hint = None;
                    entry.item = CombItem::FastDirectFanout { src_id, dst_ids };
                }
                grouped.push(entry);
            }
            entries = grouped;
            sim_dbg_eprintln!(
                "[OPT] grouped {} direct copies into {} fanout entries",
                fanout_copies,
                fanout_groups
            );
        }

        writer_counts.fill(0);
        for entry in &entries {
            for &dst in &entry.cold.write_signal_ids {
                if dst < writer_counts.len() {
                    writer_counts[dst] = writer_counts[dst].saturating_add(1);
                }
            }
        }
        let mut bufs_by_src: HashMap<(u32, u32, bool), Vec<(usize, BitRef)>> = HashMap::default();
        for (eidx, entry) in entries.iter().enumerate() {
            if let CombItem::FusedGate {
                op: FusedGate::Buf1 { dst, src, invert },
            } = &entry.item
            {
                let dst_id = dst.sig_id as usize;
                if *dst != *src && writer_counts.get(dst_id).copied() == Some(1) {
                    bufs_by_src
                        .entry((src.sig_id, src.bit, *invert))
                        .or_default()
                        .push((eidx, *dst));
                }
            }
        }
        let mut buf_replacement: Vec<Option<(BitRef, Box<[BitRef]>, bool)>> =
            vec![None; entries.len()];
        let mut buf_skip = vec![false; entries.len()];
        let mut buf_groups = 0usize;
        let mut buf_copies = 0usize;
        for ((sig_id, bit, invert), copies) in bufs_by_src {
            if copies.len() < 4 {
                continue;
            }
            let first = copies[0].0;
            let dsts: Vec<BitRef> = copies.iter().map(|(_, dst)| *dst).collect();
            for &(eidx, _) in copies.iter().skip(1) {
                buf_skip[eidx] = true;
            }
            buf_groups += 1;
            buf_copies += dsts.len();
            buf_replacement[first] =
                Some((BitRef { sig_id, bit }, dsts.into_boxed_slice(), invert));
        }
        if buf_groups != 0 {
            let mut grouped = Vec::with_capacity(entries.len() - buf_copies + buf_groups);
            for (eidx, mut entry) in entries.into_iter().enumerate() {
                if buf_skip[eidx] {
                    continue;
                }
                if let Some((src, dsts, invert)) = buf_replacement[eidx].take() {
                    entry.cold.read_signal_ids.clear();
                    entry.cold.read_signal_ids.push(src.sig_id as usize);
                    entry.cold.write_signal_ids.clear();
                    entry
                        .cold
                        .write_signal_ids
                        .extend(dsts.iter().map(|dst| dst.sig_id as usize));
                    entry.has_unresolved_reads = false;
                    entry.cold.scope_hint = None;
                    entry.item = CombItem::FusedBufFanout { src, dsts, invert };
                }
                grouped.push(entry);
            }
            entries = grouped;
            sim_dbg_eprintln!(
                "[OPT] grouped {} fused buffers into {} fanout entries",
                buf_copies,
                buf_groups
            );
        }

        // Batch AND/NAND gates with a frequently shared operand. This is the
        // structural form of a clock-gate bank (`gclk_i = clk & enable_i`),
        // but the transformation is valid for arbitrary one-bit logic and
        // therefore needs no hierarchy or signal-name heuristic.
        let mut and_operand_uses: HashMap<BitRef, usize> = HashMap::default();
        for entry in &entries {
            if let CombItem::FusedGate {
                op:
                    FusedGate::Bin2 {
                        dst,
                        a,
                        b,
                        op: GateBin::And,
                        ..
                    },
            } = &entry.item
            {
                if dst != a
                    && dst != b
                    && a != b
                    && writer_counts.get(dst.sig_id as usize).copied() == Some(1)
                {
                    *and_operand_uses.entry(*a).or_default() += 1;
                    *and_operand_uses.entry(*b).or_default() += 1;
                }
            }
        }
        let mut and_groups: HashMap<(BitRef, bool), Vec<(usize, FusedAndBranch)>> =
            HashMap::default();
        for (eidx, entry) in entries.iter().enumerate() {
            if let CombItem::FusedGate {
                op:
                    FusedGate::Bin2 {
                        dst,
                        a,
                        b,
                        op: GateBin::And,
                        invert,
                    },
            } = &entry.item
            {
                if dst == a
                    || dst == b
                    || a == b
                    || writer_counts.get(dst.sig_id as usize).copied() != Some(1)
                {
                    continue;
                }
                let a_uses = and_operand_uses.get(a).copied().unwrap_or(0);
                let b_uses = and_operand_uses.get(b).copied().unwrap_or(0);
                let (common, other) = if a_uses >= b_uses { (*a, *b) } else { (*b, *a) };
                and_groups
                    .entry((common, *invert))
                    .or_default()
                    .push((eidx, FusedAndBranch { dst: *dst, other }));
            }
        }
        let mut and_replacement: Vec<Option<(BitRef, Box<[FusedAndBranch]>, bool)>> =
            vec![None; entries.len()];
        let mut and_skip = vec![false; entries.len()];
        let mut and_batch_count = 0usize;
        let mut and_gate_count = 0usize;
        for ((common, invert), branches) in and_groups {
            if branches.len() < 4 {
                continue;
            }
            let first = branches[0].0;
            let packed: Vec<FusedAndBranch> = branches.iter().map(|(_, branch)| *branch).collect();
            for &(eidx, _) in branches.iter().skip(1) {
                and_skip[eidx] = true;
            }
            and_batch_count += 1;
            and_gate_count += packed.len();
            and_replacement[first] = Some((common, packed.into_boxed_slice(), invert));
        }
        if and_batch_count != 0 {
            let mut grouped = Vec::with_capacity(entries.len() - and_gate_count + and_batch_count);
            for (eidx, mut entry) in entries.into_iter().enumerate() {
                if and_skip[eidx] {
                    continue;
                }
                if let Some((common, branches, invert)) = and_replacement[eidx].take() {
                    entry.cold.read_signal_ids.clear();
                    entry.cold.read_signal_ids.push(common.sig_id as usize);
                    for branch in branches.iter() {
                        let id = branch.other.sig_id as usize;
                        if !entry.cold.read_signal_ids.contains(&id) {
                            entry.cold.read_signal_ids.push(id);
                        }
                    }
                    entry.cold.write_signal_ids.clear();
                    entry
                        .cold
                        .write_signal_ids
                        .extend(branches.iter().map(|branch| branch.dst.sig_id as usize));
                    entry.has_unresolved_reads = false;
                    entry.cold.scope_hint = None;
                    entry.item = CombItem::FusedAndFanout {
                        common,
                        branches,
                        invert,
                    };
                }
                grouped.push(entry);
            }
            entries = grouped;
            sim_dbg_eprintln!(
                "[OPT] grouped {} fused AND/NAND gates into {} shared-input batches",
                and_gate_count,
                and_batch_count
            );
        }

        // A constant source slice assigned directly to an equally sized
        // destination slice does not need a VM register. Use the existing
        // four-state vector path so the read, partial write, observers, and
        // dependency propagation happen in one entry operation. Restrict the
        // first rollout to descending, at-most-word-sized ranges; those are
        // the common bus-lane form and require no bit-order transformation.
        if matches!(std::env::var("XEZIM_RANGE_COPY").as_deref(), Ok("1")) {
            use super::bytecode::Insn as I;

            let mut lowered = 0usize;
            for entry in entries.iter_mut() {
                if entry.has_unresolved_reads || entry.defer_at_time0 {
                    continue;
                }
                let CombItem::CompiledContAssign { compiled } = &entry.item else {
                    continue;
                };
                let [
                    I::LoadSignalRange(reg, src, src_hi, src_lo),
                    I::BlockingAssignRange(dst, dst_hi, dst_lo, value_reg),
                ] = compiled.instructions.as_slice()
                else {
                    continue;
                };
                if reg != value_reg || src_hi < src_lo || dst_hi < dst_lo {
                    continue;
                }
                let src_width = src_hi - src_lo + 1;
                let dst_width = dst_hi - dst_lo + 1;
                if src_width != dst_width || src_width == 0 || src_width > 64 {
                    continue;
                }
                let src_id = *src as usize;
                let dst_id = *dst as usize;
                if self.signal_widths.get(src_id).is_none_or(|&w| *src_hi >= w)
                    || self
                        .signal_widths
                        .get(dst_id)
                        .is_none_or(|&w| w > 64 || *dst_hi >= w)
                    || self.signal_real.get(src_id).copied().unwrap_or(false)
                    || self.signal_real.get(dst_id).copied().unwrap_or(false)
                    || self.signal_two_state.get(dst_id).copied().unwrap_or(false)
                    || self.sdf_delays.get(dst_id).copied().unwrap_or(0) != 0
                {
                    continue;
                }
                let overlaps = src_id == dst_id && *src_lo <= *dst_hi && *dst_lo <= *src_hi;
                if overlaps {
                    continue;
                }
                entry.item = CombItem::VectorGate {
                    op: VectorGate::Buf {
                        dst: VectorRef {
                            sig_id: *dst,
                            lo: *dst_lo,
                        },
                        src: VectorRef {
                            sig_id: *src,
                            lo: *src_lo,
                        },
                        width: src_width as u8,
                        invert: false,
                    },
                };
                lowered += 1;
            }
            eprintln!(
                "[RANGE-COPY] lowered {} constant-range assignments",
                lowered
            );
        }

        // Amalgamate straight-line compiled cont-assigns that share one read
        // set (typically the N same-shape port connects of N instances all
        // fed by one testbench net). One worklist dispatch then evaluates the
        // whole wave via a single concatenated instruction stream instead of
        // N pops + N exec setups. Members are independent by construction:
        // identical read sets and no member writing into the shared read set
        // means no member observes another's output, so concatenation order
        // (ascending entry index) cannot change the fixpoint.
        {
            let mut ca_groups: HashMap<Vec<usize>, Vec<usize>> = HashMap::default();
            for (eidx, e) in entries.iter().enumerate() {
                let CombItem::CompiledContAssign { compiled } = &e.item else {
                    continue;
                };
                if e.has_unresolved_reads || e.cold.read_signal_ids.is_empty() {
                    continue;
                }
                // Straight-line only: no branch targets to rebase, and no AST
                // fallbacks (those would need per-member scope hints).
                use super::bytecode::Insn as I;
                if compiled.instructions.iter().any(|i| {
                    matches!(
                        i,
                        I::Jump(..)
                            | I::BranchIfFalse(..)
                            | I::BranchUnlessZero(..)
                            | I::BranchIfSignalFalse(..)
                            | I::StmtFallback(..)
                            | I::EvalExprFallback(..)
                    )
                }) {
                    continue;
                }
                if e.cold
                    .write_signal_ids
                    .iter()
                    .any(|w| e.cold.read_signal_ids.contains(w))
                {
                    continue;
                }
                let mut key = e.cold.read_signal_ids.clone();
                key.sort_unstable();
                key.dedup();
                ca_groups.entry(key).or_default().push(eidx);
            }
            let mut batches: Vec<Vec<usize>> =
                ca_groups.into_values().filter(|v| v.len() >= 4).collect();
            // Deterministic order regardless of map iteration.
            batches.sort_unstable_by_key(|v| v[0]);
            if !batches.is_empty() {
                let mut ca_skip = vec![false; entries.len()];
                let mut ca_batch_members = 0usize;
                for members in &batches {
                    let first = members[0];
                    let mut combined = match &entries[first].item {
                        CombItem::CompiledContAssign { compiled } => compiled.clone(),
                        _ => unreachable!("filtered above"),
                    };
                    let mut writes = entries[first].cold.write_signal_ids.clone();
                    for &m in &members[1..] {
                        let CombItem::CompiledContAssign { compiled } = &entries[m].item else {
                            unreachable!("filtered above")
                        };
                        combined
                            .instructions
                            .extend(compiled.instructions.iter().cloned());
                        combined.num_regs = combined.num_regs.max(compiled.num_regs);
                        for &w in &entries[m].cold.write_signal_ids {
                            if !writes.contains(&w) {
                                writes.push(w);
                            }
                        }
                        ca_skip[m] = true;
                    }
                    ca_batch_members += members.len();
                    entries[first].item = CombItem::CompiledContAssign { compiled: combined };
                    entries[first].cold.write_signal_ids = writes;
                }
                let batch_count = batches.len();
                let mut grouped =
                    Vec::with_capacity(entries.len() + batch_count - ca_batch_members);
                for (eidx, entry) in entries.into_iter().enumerate() {
                    if !ca_skip[eidx] {
                        grouped.push(entry);
                    }
                }
                entries = grouped;
                sim_dbg_eprintln!(
                    "[OPT] amalgamated {} compiled cont-assigns into {} shared-read batches",
                    ca_batch_members,
                    batch_count
                );
            }
        }

        // Chain fusion: A writes s; B is s's ONLY comb reader and A its only
        // writer — merge B's body into A's entry so the s hop skips the
        // whole dispatch round trip (write -> mark dirty -> worklist ->
        // re-dispatch). The ibex cone census: 51.8% of entries sit on such
        // chains; fusing removes ~32% of entry dispatches. `s` is still
        // WRITTEN (edge blocks / waiters / VCD see it); only its comb-CSR
        // hop disappears, because the merged entry's read set drops `s`.
        // B re-runs whenever A fires even if `s` did not change — comb
        // bodies are idempotent, so that trades a possible extra eval for
        // the dispatch. OPT-IN (XEZIM_FUSE_CHAINS=1): measured on ibex the
        // guarded merge is timing-neutral-to-slightly-negative — the
        // dispatch-plan cache already collected the cheap dispatches, and
        // the safety guards (no sens widening, no feedback; the unguarded
        // form was 11x SLOWER from union-sensitivity over-triggering)
        // exclude most census candidates. Kept for designs with long
        // pass-through chains.
        if matches!(std::env::var("XEZIM_FUSE_CHAINS").as_deref(), Ok("1")) {
            let n = entries.len();
            let mergeable = |e: &CombEntry| {
                matches!(
                    e.item,
                    CombItem::CompiledContAssign { .. } | CombItem::CompiledAlwaysBlock { .. }
                ) && !match &e.item {
                    CombItem::CompiledContAssign { compiled }
                    | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled.has_fallback,
                    _ => false,
                }
            };
            // writer/reader maps over COMB entries only.
            let mut writer: HashMap<usize, Option<usize>> = HashMap::default();
            for (i, e) in entries.iter().enumerate() {
                for &w in &e.cold.write_signal_ids {
                    writer.entry(w).and_modify(|v| *v = None).or_insert(Some(i));
                }
            }
            let mut readers: HashMap<usize, Vec<usize>> = HashMap::default();
            for (i, e) in entries.iter().enumerate() {
                for &r in &e.cold.read_signal_ids {
                    readers.entry(r).or_default().push(i);
                }
            }
            // next[a] = (b, link_signal) when a's write s links uniquely to b.
            let mut next: Vec<Option<(usize, usize)>> = vec![None; n];
            let mut has_pred: Vec<bool> = vec![false; n];
            for (i, e) in entries.iter().enumerate() {
                if !mergeable(&entries[i]) {
                    continue;
                }
                let mut link: Option<(usize, usize)> = None;
                let mut ok = true;
                for &sw in &e.cold.write_signal_ids {
                    let rs = readers.get(&sw).map(|v| v.as_slice()).unwrap_or(&[]);
                    let uniq: Vec<usize> = {
                        let mut u: Vec<usize> = rs.iter().copied().filter(|&r| r != i).collect();
                        u.sort_unstable();
                        u.dedup();
                        u
                    };
                    if uniq.len() == 1 {
                        let b = uniq[0];
                        if writer.get(&sw) == Some(&Some(i))
                            && mergeable(&entries[b])
                            && !entries[b].cold.write_signal_ids.contains(&sw)
                        {
                            match link {
                                None => link = Some((b, sw)),
                                // Two write signals both linking to the SAME
                                // successor is fine; different successors is
                                // not a simple chain — skip.
                                Some((pb, _)) if pb == b => {}
                                _ => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                    } else if !uniq.is_empty() {
                        // A write with multiple comb readers keeps its normal
                        // dispatch; it does not block linking via another
                        // single-reader write, but B must see EVERY A-write it
                        // reads through the merge, which the union handles.
                    }
                }
                if ok {
                    if let Some((b, sw)) = link {
                        if b != i {
                            next[i] = Some((b, sw));
                        }
                    }
                }
            }
            for &nx in next.iter() {
                if let Some((b, _)) = nx {
                    has_pred[b] = true;
                }
            }
            // A successor with TWO predecessors would be duplicated into
            // both — disallow: only keep a link when it is b's only one.
            {
                let mut pred_count = vec![0u32; n];
                for &nx in next.iter() {
                    if let Some((b, _)) = nx {
                        pred_count[b] += 1;
                    }
                }
                for slot in next.iter_mut() {
                    if let Some((b, _)) = *slot {
                        if pred_count[b] > 1 {
                            *slot = None;
                        }
                    }
                }
                has_pred = vec![false; n];
                for &nx in next.iter() {
                    if let Some((b, _)) = nx {
                        has_pred[b] = true;
                    }
                }
            }
            let mut absorbed = vec![false; n];
            let mut n_links = 0usize;
            for head in 0..n {
                if has_pred[head] || next[head].is_none() || absorbed[head] {
                    continue;
                }
                // Walk the chain from its head, absorbing successors.
                let mut cur = head;
                let mut in_chain: HashSet<usize> = HashSet::default();
                in_chain.insert(head);
                while let Some((b, sw)) = next[cur] {
                    if absorbed[b] || in_chain.contains(&b) {
                        break;
                    }
                    // GUARDS (the unguarded merge measured 11x SLOWER on
                    // ibex: union sensitivity made every B-input change run
                    // A+B, and B->A feedback re-triggered merged entries):
                    //  1. no sens widening — B's other reads must already be
                    //     in the head's read set, so the merged entry fires
                    //     exactly when the head fired before;
                    //  2. no feedback — nothing B writes may be read by the
                    //     merged entry.
                    {
                        let he = &entries[head];
                        let be = &entries[b];
                        let widen = be
                            .cold
                            .read_signal_ids
                            .iter()
                            .any(|r| *r != sw && !he.cold.read_signal_ids.contains(r));
                        let feedback = be.cold.write_signal_ids.iter().any(|w| {
                            he.cold.read_signal_ids.contains(w)
                                || be.cold.read_signal_ids.contains(w)
                        });
                        if widen || feedback {
                            break;
                        }
                    }
                    // Merge b into head.
                    let (b_compiled, b_reads, b_writes) = {
                        let be = &entries[b];
                        let c = match &be.item {
                            CombItem::CompiledContAssign { compiled }
                            | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled.clone(),
                            _ => break,
                        };
                        (
                            c,
                            be.cold.read_signal_ids.clone(),
                            be.cold.write_signal_ids.clone(),
                        )
                    };
                    {
                        let he = &mut entries[head];
                        let hc = match &mut he.item {
                            CombItem::CompiledContAssign { compiled }
                            | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled,
                            _ => unreachable!(),
                        };
                        hc.instructions
                            .extend(b_compiled.instructions.iter().cloned());
                        hc.num_regs = hc.num_regs.max(b_compiled.num_regs);
                        hc.has_fallback |= b_compiled.has_fallback;
                        for r in b_reads {
                            if r != sw && !he.cold.read_signal_ids.contains(&r) {
                                he.cold.read_signal_ids.push(r);
                            }
                        }
                        for w in b_writes {
                            if !he.cold.write_signal_ids.contains(&w) {
                                he.cold.write_signal_ids.push(w);
                            }
                        }
                    }
                    absorbed[b] = true;
                    in_chain.insert(b);
                    n_links += 1;
                    cur = b;
                }
            }
            if n_links > 0 {
                let mut kept = Vec::with_capacity(n - n_links);
                for (i, entry) in entries.into_iter().enumerate() {
                    if !absorbed[i] {
                        kept.push(entry);
                    }
                }
                entries = kept;
                sim_dbg_eprintln!(
                    "[OPT] chain-fused {} comb entries into their producers",
                    n_links
                );
            }
        }

        // Dead-cone census (XEZIM_DEAD_CENSUS=1, analysis-only). Their
        // hanging-logic elimination is a default, production-grade pass in the
        // reference simulator
        // and xezim has no equivalent. Count comb entries whose outputs
        // nothing observable reads, closed transitively.
        //
        // CAVEAT, stated in the output: reads from PROCESS/AST code (initial
        // blocks, delayed always, tasks, $display args) and from dump/VPI are
        // not modelled here, so this is an UPPER BOUND. It exists to decide
        // whether a precise implementation is worth building.
        if std::env::var("XEZIM_DEAD_CENSUS").is_ok() {
            use super::bytecode::Insn as DI;
            let mut observed: std::collections::HashSet<usize> = std::collections::HashSet::new();
            let mut opaque = 0usize;
            for (bi, cb) in self.compiled_edge_blocks.iter().enumerate() {
                if let Some(b) = self.edge_blocks.get(bi) {
                    for t in &b.resolved_sensitivities {
                        observed.insert(t.signal_id);
                    }
                }
                match cb {
                    None => opaque += 1,
                    Some(cb) if cb.has_fallback => opaque += 1,
                    Some(cb) => {
                        for ins in cb.instructions.iter() {
                            match ins {
                                DI::LoadSignal(_, sig)
                                | DI::LoadSignalSigned(_, sig)
                                | DI::LoadSignalBit(_, sig, _)
                                | DI::LoadSignalRange(_, sig, _, _)
                                | DI::BranchIfSignalFalse(sig, _, _) => {
                                    observed.insert(*sig as usize);
                                }
                                DI::NbaAssignArrayRead(_, _, idx, _) => {
                                    observed.insert(*idx as usize);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            let mut alive: Vec<bool> = vec![true; entries.len()];
            let mut dead_rounds = 0usize;
            for _round in 0..32 {
                let mut reads: std::collections::HashSet<usize> = observed.clone();
                for (i, e) in entries.iter().enumerate() {
                    if !alive[i] {
                        continue;
                    }
                    for &r in &e.cold.read_signal_ids {
                        reads.insert(r);
                    }
                }
                let mut changed = false;
                for (i, e) in entries.iter().enumerate() {
                    if !alive[i] || e.cold.write_signal_ids.is_empty() {
                        continue;
                    }
                    if e.cold.write_signal_ids.iter().all(|w| !reads.contains(w)) {
                        alive[i] = false;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
                dead_rounds += 1;
            }
            let dead = alive.iter().filter(|a| !**a).count();
            eprintln!(
                "[DEAD-CENSUS] reader-visibility probe: initial_blocks={} always_blocks={} pending_initial={} pending_always={} tasks={} fns={} | dumps vcd={} fst={} xtrace={}",
                self.module.initial_blocks.len(),
                self.module.always_blocks.len(),
                self.module.pending_initial.len(),
                self.module.pending_always.len(),
                self.module.tasks.len(),
                self.module.functions.len(),
                self.vcd_file.is_some(),
                self.fst_file.is_some(),
                self.xtrace_file.is_some(),
            );
            eprintln!(
                "[DEAD-CENSUS] UPPER BOUND (process/AST/dump reads NOT modelled): dead={} of {} comb entries ({:.1}%), {} rounds, opaque_edge={}",
                dead,
                entries.len(),
                100.0 * dead as f64 / entries.len().max(1) as f64,
                dead_rounds,
                opaque
            );
        }
        // Block-splitting census (XEZIM_SPLIT_CENSUS=1, analysis-only).
        // Splitting combinational always blocks into single-statement blocks
        // is a default pass in the reference simulator, and it is the one gap
        // that survives the per-firing filter: a block whose statements read
        // disjoint inputs currently re-runs ALL of them whenever ANY input
        // changes. Splitting narrows each piece's sensitivity.
        //
        // Payoff proxy: for a block with read set R and top-level statements
        // s_i with read sets R_i, an input change today costs the whole
        // block; after splitting it costs only the statements that read it.
        // `work_ratio` = sum|R_i| / (n * |R|) approximates the post-split
        // cost as a fraction of today's, under uniform input activity.
        if std::env::var("XEZIM_SPLIT_CENSUS").is_ok() {
            let mut comb_always = 0usize;
            let mut multi_stmt = 0usize;
            let mut splittable = 0usize; // >=2 stmts AND some disjointness
            let mut legal_split = 0usize; // ... AND legal to split (see below)
            let mut stmts_total = 0usize;
            let mut ratio_sum = 0.0f64;
            let mut ratio_n = 0usize;
            let mut best: Vec<(f64, usize, usize)> = Vec::new(); // ratio, stmts, |R|
            for ab in self.module.always_blocks.iter() {
                let is_comb = matches!(
                    ab.kind,
                    crate::ast::decl::AlwaysKind::AlwaysComb
                        | crate::ast::decl::AlwaysKind::AlwaysLatch
                        | crate::ast::decl::AlwaysKind::Always
                );
                if !is_comb {
                    continue;
                }
                comb_always += 1;
                let StatementKind::SeqBlock { stmts, .. } = &ab.stmt.kind else {
                    continue;
                };
                if stmts.len() < 2 {
                    continue;
                }
                multi_stmt += 1;
                stmts_total += stmts.len();
                let mut whole: HashSet<String> = HashSet::default();
                let mut w = HashSet::default();
                Self::collect_stmt_reads(&ab.stmt, &self.module, &mut whole, &mut w);
                if whole.is_empty() {
                    continue;
                }
                let mut per_sum = 0usize;
                let mut disjoint = false;
                let mut prev: Vec<HashSet<String>> = Vec::new();
                let mut prev_w: Vec<HashSet<String>> = Vec::new();
                // A split is only LEGAL when the statements are independent:
                // no two write the same signal (else the split creates a
                // multi-driver — the `y = '0; if (c) y = a;` default-
                // assignment idiom is exactly this), and no statement reads
                // what another writes (intra-block dataflow).
                let mut legal = true;
                for st in stmts.iter() {
                    let mut r: HashSet<String> = HashSet::default();
                    let mut w2: HashSet<String> = HashSet::default();
                    Self::collect_stmt_reads(st, &self.module, &mut r, &mut w2);
                    per_sum += r.len();
                    if prev.iter().any(|p| p.is_disjoint(&r)) {
                        disjoint = true;
                    }
                    for pw in prev_w.iter() {
                        if !pw.is_disjoint(&w2) || !pw.is_disjoint(&r) {
                            legal = false;
                        }
                    }
                    for pr in prev.iter() {
                        if !pr.is_disjoint(&w2) {
                            legal = false;
                        }
                    }
                    prev.push(r);
                    prev_w.push(w2);
                }
                if legal {
                    legal_split += 1;
                }
                let ratio = per_sum as f64 / (stmts.len() as f64 * whole.len() as f64);
                ratio_sum += ratio;
                ratio_n += 1;
                if disjoint {
                    splittable += 1;
                }
                best.push((ratio, stmts.len(), whole.len()));
            }
            best.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            eprintln!(
                "[SPLIT-CENSUS] comb always blocks={} | multi-statement={} | disjoint reads={} | LEGALLY splittable={} | total top-level stmts={}",
                comb_always, multi_stmt, splittable, legal_split, stmts_total
            );
            eprintln!(
                "[SPLIT-CENSUS] mean work_ratio after split = {:.3} (1.0 = no gain, lower is better) over {} blocks; best 5: {:?}",
                if ratio_n > 0 {
                    ratio_sum / ratio_n as f64
                } else {
                    1.0
                },
                ratio_n,
                &best[..best.len().min(5)]
            );
        }
        // Constant-propagation census (XEZIM_CONST_CENSUS=1, analysis-only).
        // Constant propagation through continuous assigns is a default pass
        // in the reference simulator, whose own c906 log reports 8,429 values
        // through CAs". Size the same population here: nets whose driving
        // comb entry reads NOTHING (so its output never changes after t0),
        // transitively closed, then count the reader LOAD sites those
        // constants would turn into immediates.
        if std::env::var("XEZIM_CONST_CENSUS").is_ok() {
            use super::bytecode::Insn as CI;
            // Signals any EDGE block writes are not constant. An edge block
            // without a compiled body could write anything, so treat the
            // whole census as unreliable if one exists (report it).
            let mut edge_written: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            let mut opaque_edge = 0usize;
            for cb in self.compiled_edge_blocks.iter() {
                match cb {
                    None => opaque_edge += 1,
                    Some(cb) if cb.has_fallback => opaque_edge += 1,
                    Some(cb) => {
                        for ins in cb.instructions.iter() {
                            match ins {
                                CI::NbaAssign(sig, ..)
                                | CI::NbaAssignConst(sig, ..)
                                | CI::NbaAssignRange(sig, ..)
                                | CI::NbaAssignRangeDyn(sig, ..)
                                | CI::NbaAssignBitDyn(sig, ..)
                                | CI::NbaAssignArrayRead(sig, ..)
                                | CI::BlockingAssign(sig, ..)
                                | CI::BlockingAssignRange(sig, ..)
                                | CI::BlockingAssignRangeDyn(sig, ..)
                                | CI::BlockingAssignBitDyn(sig, ..)
                                | CI::BlockingAssignString(sig, ..) => {
                                    edge_written.insert(*sig as usize);
                                }
                                CI::NbaAssignArray(arr, ..)
                                | CI::NbaAssignArrayRange(arr, ..)
                                | CI::BlockingAssignArray(arr, ..)
                                | CI::BlockingAssignArrayRange(arr, ..) => {
                                    if let super::bytecode::ArrayOperand::Dense {
                                        first_id,
                                        lo,
                                        hi,
                                        ..
                                    } = arr.as_ref()
                                    {
                                        let n = (hi - lo).unsigned_abs() as usize + 1;
                                        for k in 0..n {
                                            edge_written.insert(first_id + k);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            let mut comb_writers: HashMap<usize, u32> = HashMap::default();
            for e in entries.iter() {
                for &w in &e.cold.write_signal_ids {
                    *comb_writers.entry(w).or_insert(0) += 1;
                }
            }
            let single_and_free = |w: usize| -> bool {
                comb_writers.get(&w).copied().unwrap_or(0) == 1 && !edge_written.contains(&w)
            };
            let mut constant: std::collections::HashSet<usize> = std::collections::HashSet::new();
            let mut const_entries = 0usize;
            let mut seeds = 0usize;
            for _round in 0..16 {
                let mut changed = false;
                for e in entries.iter() {
                    if e.has_unresolved_reads || e.cold.write_signal_ids.is_empty() {
                        continue;
                    }
                    if !e.cold.write_signal_ids.iter().all(|&w| single_and_free(w)) {
                        continue;
                    }
                    if e.cold.write_signal_ids.iter().all(|w| constant.contains(w)) {
                        continue;
                    }
                    let all_const_reads =
                        e.cold.read_signal_ids.iter().all(|r| constant.contains(r));
                    if !all_const_reads {
                        continue;
                    }
                    if e.cold.read_signal_ids.is_empty() {
                        seeds += 1;
                    }
                    for &w in &e.cold.write_signal_ids {
                        constant.insert(w);
                    }
                    const_entries += 1;
                    changed = true;
                }
                if !changed {
                    break;
                }
            }
            // Reader payoff: load sites that would become immediates.
            let mut load_sites = 0usize;
            let mut blocks_touched = 0usize;
            let mut count_loads = |cb: &super::bytecode::CompiledBlock| {
                let mut hit = 0usize;
                for ins in cb.instructions.iter() {
                    let sig = match ins {
                        CI::LoadSignal(_, sig)
                        | CI::LoadSignalSigned(_, sig)
                        | CI::LoadSignalBit(_, sig, _)
                        | CI::LoadSignalRange(_, sig, _, _) => Some(*sig as usize),
                        _ => None,
                    };
                    if let Some(sig) = sig {
                        if constant.contains(&sig) {
                            hit += 1;
                        }
                    }
                }
                if hit > 0 {
                    blocks_touched += 1;
                    load_sites += hit;
                }
            };
            for e in entries.iter() {
                match &e.item {
                    CombItem::CompiledContAssign { compiled }
                    | CombItem::CompiledAlwaysBlock { compiled, .. } => count_loads(compiled),
                    _ => {}
                }
            }
            for cb in self.compiled_edge_blocks.iter().flatten() {
                count_loads(cb);
            }
            eprintln!(
                "[CONST-CENSUS] const_nets={} from {} entries ({} seed, no-read) | opaque_edge_blocks={}",
                constant.len(),
                const_entries,
                seeds,
                opaque_edge
            );
            eprintln!(
                "[CONST-CENSUS] reader payoff: {} load sites in {} blocks would fold to immediates (of {} comb entries)",
                load_sites,
                blocks_touched,
                entries.len()
            );
        }
        // Aligned bit-slice coalescing is deliberately opt-in until the large
        // design census establishes its general payoff. Unlike dependency
        // region fusion, every member has the same parent trigger set and no
        // member feeds another: one vector operation replaces independent
        // one-bit gate entries without widening sensitivity.
        if std::env::var("XEZIM_VEC_COALESCE").ok().as_deref() == Some("1") {
            #[derive(Clone, Copy, Hash, PartialEq, Eq)]
            enum VShape {
                Buf {
                    src: u32,
                    off: i64,
                    invert: bool,
                },
                Bin {
                    a: u32,
                    a_off: i64,
                    b: u32,
                    b_off: i64,
                    op: GateBin,
                    invert: bool,
                },
                Mux {
                    s: u32,
                    s_off: i64,
                    t: u32,
                    t_off: i64,
                    e: u32,
                    e_off: i64,
                },
            }
            #[derive(Clone, Copy, Hash, PartialEq, Eq)]
            struct VKey {
                dst: u32,
                shape: VShape,
            }

            let mut buckets: HashMap<VKey, Vec<(usize, u32)>> = HashMap::default();
            for (idx, entry) in entries.iter().enumerate() {
                let (dst, shape) = match &entry.item {
                    CombItem::FusedGate {
                        op: FusedGate::Buf1 { dst, src, invert },
                    } => {
                        // A primitive buffer maps z to x, while a continuous
                        // assignment passes z through. Retain the single-bit
                        // primitive path for that case.
                        if !*invert
                            && self
                                .signal_gate_driven
                                .get(dst.sig_id as usize)
                                .copied()
                                .unwrap_or(false)
                        {
                            continue;
                        }
                        (
                            *dst,
                            VShape::Buf {
                                src: src.sig_id,
                                off: src.bit as i64 - dst.bit as i64,
                                invert: *invert,
                            },
                        )
                    }
                    CombItem::FusedGate {
                        op:
                            FusedGate::Bin2 {
                                dst,
                                a,
                                b,
                                op,
                                invert,
                            },
                    } => (
                        *dst,
                        VShape::Bin {
                            a: a.sig_id,
                            a_off: a.bit as i64 - dst.bit as i64,
                            b: b.sig_id,
                            b_off: b.bit as i64 - dst.bit as i64,
                            op: *op,
                            invert: *invert,
                        },
                    ),
                    CombItem::FusedGate {
                        op: FusedGate::Mux2 { dst, s, t, e },
                    } => (
                        *dst,
                        VShape::Mux {
                            s: s.sig_id,
                            s_off: s.bit as i64 - dst.bit as i64,
                            t: t.sig_id,
                            t_off: t.bit as i64 - dst.bit as i64,
                            e: e.sig_id,
                            e_off: e.bit as i64 - dst.bit as i64,
                        },
                    ),
                    _ => continue,
                };
                if entry.has_unresolved_reads || entry.defer_at_time0 {
                    continue;
                }
                buckets
                    .entry(VKey {
                        dst: dst.sig_id,
                        shape,
                    })
                    .or_default()
                    .push((idx, dst.bit));
            }

            let mut groups: Vec<(VKey, Vec<(usize, u32)>)> = buckets.into_iter().collect();
            groups.sort_unstable_by_key(|(_, members)| {
                members
                    .iter()
                    .map(|(idx, _)| *idx)
                    .min()
                    .unwrap_or(usize::MAX)
            });
            let mut vectors = 0usize;
            let mut members_removed = 0usize;
            let mut bits = 0usize;
            for (key, mut members) in groups {
                members.sort_unstable_by_key(|&(idx, bit)| (bit, idx));
                let mut start = 0usize;
                while start < members.len() {
                    let mut end = start + 1;
                    while end < members.len()
                        && members[end].1 == members[end - 1].1 + 1
                        && end - start < 64
                    {
                        end += 1;
                    }
                    let run = &members[start..end];
                    start = end;
                    if run.len() < 2 {
                        continue;
                    }
                    let dst_lo = run[0].1;
                    let width = run.len() as u32;
                    let dst_hi = dst_lo + width - 1;
                    if self
                        .signal_widths
                        .get(key.dst as usize)
                        .is_none_or(|&w| w > 64 || dst_hi >= w)
                    {
                        continue;
                    }
                    let overlaps_dst =
                        |sig: u32, lo: u32, hi: u32| sig == key.dst && lo <= dst_hi && dst_lo <= hi;
                    let (read_ids, vector_op) = match key.shape {
                        VShape::Buf { src, off, invert } => {
                            let src_lo_i = dst_lo as i64 + off;
                            if src_lo_i < 0 {
                                continue;
                            }
                            let src_lo = src_lo_i as u32;
                            let src_hi = src_lo + width - 1;
                            if self
                                .signal_widths
                                .get(src as usize)
                                .is_none_or(|&w| src_hi >= w)
                                || overlaps_dst(src, src_lo, src_hi)
                            {
                                continue;
                            }
                            (
                                vec![src as usize],
                                VectorGate::Buf {
                                    dst: VectorRef {
                                        sig_id: key.dst,
                                        lo: dst_lo,
                                    },
                                    src: VectorRef {
                                        sig_id: src,
                                        lo: src_lo,
                                    },
                                    width: width as u8,
                                    invert,
                                },
                            )
                        }
                        VShape::Bin {
                            a,
                            a_off,
                            b,
                            b_off,
                            op,
                            invert,
                        } => {
                            let a_lo_i = dst_lo as i64 + a_off;
                            let b_lo_i = dst_lo as i64 + b_off;
                            if a_lo_i < 0 || b_lo_i < 0 {
                                continue;
                            }
                            let (a_lo, b_lo) = (a_lo_i as u32, b_lo_i as u32);
                            let (a_hi, b_hi) = (a_lo + width - 1, b_lo + width - 1);
                            if self
                                .signal_widths
                                .get(a as usize)
                                .is_none_or(|&w| a_hi >= w)
                                || self
                                    .signal_widths
                                    .get(b as usize)
                                    .is_none_or(|&w| b_hi >= w)
                                || overlaps_dst(a, a_lo, a_hi)
                                || overlaps_dst(b, b_lo, b_hi)
                            {
                                continue;
                            }
                            let mut reads = vec![a as usize];
                            if b != a {
                                reads.push(b as usize);
                            }
                            (
                                reads,
                                VectorGate::Bin {
                                    dst: VectorRef {
                                        sig_id: key.dst,
                                        lo: dst_lo,
                                    },
                                    a: VectorRef {
                                        sig_id: a,
                                        lo: a_lo,
                                    },
                                    b: VectorRef {
                                        sig_id: b,
                                        lo: b_lo,
                                    },
                                    width: width as u8,
                                    op,
                                    invert,
                                },
                            )
                        }
                        VShape::Mux {
                            s,
                            s_off,
                            t,
                            t_off,
                            e,
                            e_off,
                        } => {
                            let s_lo_i = dst_lo as i64 + s_off;
                            let t_lo_i = dst_lo as i64 + t_off;
                            let e_lo_i = dst_lo as i64 + e_off;
                            if s_lo_i < 0 || t_lo_i < 0 || e_lo_i < 0 {
                                continue;
                            }
                            let (s_lo, t_lo, e_lo) = (s_lo_i as u32, t_lo_i as u32, e_lo_i as u32);
                            let (s_hi, t_hi, e_hi) =
                                (s_lo + width - 1, t_lo + width - 1, e_lo + width - 1);
                            if self
                                .signal_widths
                                .get(s as usize)
                                .is_none_or(|&w| s_hi >= w)
                                || self
                                    .signal_widths
                                    .get(t as usize)
                                    .is_none_or(|&w| t_hi >= w)
                                || self
                                    .signal_widths
                                    .get(e as usize)
                                    .is_none_or(|&w| e_hi >= w)
                                || overlaps_dst(s, s_lo, s_hi)
                                || overlaps_dst(t, t_lo, t_hi)
                                || overlaps_dst(e, e_lo, e_hi)
                            {
                                continue;
                            }
                            let mut reads = vec![s as usize];
                            if !reads.contains(&(t as usize)) {
                                reads.push(t as usize);
                            }
                            if !reads.contains(&(e as usize)) {
                                reads.push(e as usize);
                            }
                            (
                                reads,
                                VectorGate::Mux {
                                    dst: VectorRef {
                                        sig_id: key.dst,
                                        lo: dst_lo,
                                    },
                                    s: VectorRef {
                                        sig_id: s,
                                        lo: s_lo,
                                    },
                                    t: VectorRef {
                                        sig_id: t,
                                        lo: t_lo,
                                    },
                                    e: VectorRef {
                                        sig_id: e,
                                        lo: e_lo,
                                    },
                                    width: width as u8,
                                },
                            )
                        }
                    };
                    let head = run.iter().map(|&(idx, _)| idx).min().unwrap();
                    entries[head].item = CombItem::VectorGate { op: vector_op };
                    entries[head].cold.read_signal_ids = read_ids;
                    entries[head].cold.write_signal_ids.clear();
                    entries[head].cold.write_signal_ids.push(key.dst as usize);
                    entries[head].has_unresolved_reads = false;
                    entries[head].defer_at_time0 = false;
                    for &(idx, _) in run {
                        if idx == head {
                            continue;
                        }
                        entries[idx].item = CombItem::Noop;
                        entries[idx].cold.read_signal_ids.clear();
                        entries[idx].cold.write_signal_ids.clear();
                        entries[idx].has_unresolved_reads = false;
                        entries[idx].defer_at_time0 = false;
                    }
                    vectors += 1;
                    members_removed += run.len() - 1;
                    bits += run.len();
                }
            }

            #[derive(Clone, Copy, Hash, PartialEq, Eq)]
            enum SShape {
                Buf {
                    src: u32,
                    invert: bool,
                },
                Bin {
                    a: u32,
                    b: u32,
                    delta: i64,
                    op: GateBin,
                    invert: bool,
                },
            }
            let scatter_enabled = std::env::var("XEZIM_VEC_SCATTER").ok().as_deref() == Some("1");
            let min_scatter_width = std::env::var("XEZIM_VEC_SCATTER_MIN")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|&width| (2..=64).contains(&width))
                .unwrap_or(64);
            let mut scatter_widths = [0usize; 65];
            let mut scatter_layout_groups = [0usize; 4];
            let mut scatter_layout_lanes = [0usize; 4];
            let mut scatter_kind_groups = [0usize; 4];
            let mut scatter_kind_lanes = [0usize; 4];
            let mut scalar_buckets: HashMap<SShape, Vec<(usize, u32, BitRef)>> = HashMap::default();
            for (idx, entry) in entries.iter().enumerate() {
                if !scatter_enabled {
                    break;
                }
                if entry.has_unresolved_reads || entry.defer_at_time0 {
                    continue;
                }
                let (dst, coord, shape) = match &entry.item {
                    CombItem::FusedGate {
                        op: FusedGate::Buf1 { dst, src, invert },
                    } if self.signal_widths.get(dst.sig_id as usize).copied() == Some(1)
                        && self
                            .signal_widths
                            .get(src.sig_id as usize)
                            .copied()
                            .unwrap_or(1)
                            > 1
                        && (*invert
                            || !self
                                .signal_gate_driven
                                .get(dst.sig_id as usize)
                                .copied()
                                .unwrap_or(false)) =>
                    {
                        (
                            *dst,
                            src.bit,
                            SShape::Buf {
                                src: src.sig_id,
                                invert: *invert,
                            },
                        )
                    }
                    CombItem::FusedGate {
                        op:
                            FusedGate::Bin2 {
                                dst,
                                a,
                                b,
                                op,
                                invert,
                            },
                    } if self.signal_widths.get(dst.sig_id as usize).copied() == Some(1) => {
                        let aw = self
                            .signal_widths
                            .get(a.sig_id as usize)
                            .copied()
                            .unwrap_or(1);
                        let bw = self
                            .signal_widths
                            .get(b.sig_id as usize)
                            .copied()
                            .unwrap_or(1);
                        let (a, b) = if aw <= 1 && bw > 1 {
                            (*b, *a)
                        } else {
                            (*a, *b)
                        };
                        if self
                            .signal_widths
                            .get(a.sig_id as usize)
                            .copied()
                            .unwrap_or(1)
                            <= 1
                        {
                            continue;
                        }
                        (
                            *dst,
                            a.bit,
                            SShape::Bin {
                                a: a.sig_id,
                                b: b.sig_id,
                                delta: b.bit as i64 - a.bit as i64,
                                op: *op,
                                invert: *invert,
                            },
                        )
                    }
                    _ => continue,
                };
                scalar_buckets
                    .entry(shape)
                    .or_default()
                    .push((idx, coord, dst));
            }
            let mut scalar_groups: Vec<_> = scalar_buckets.into_iter().collect();
            scalar_groups.sort_unstable_by_key(|(_, members)| {
                members
                    .iter()
                    .map(|(idx, _, _)| *idx)
                    .min()
                    .unwrap_or(usize::MAX)
            });
            for (shape, mut members) in scalar_groups {
                members.sort_unstable_by_key(|&(idx, bit, _)| (bit, idx));
                let mut start = 0usize;
                while start < members.len() {
                    let mut end = start + 1;
                    while end < members.len()
                        && members[end].1 == members[end - 1].1 + 1
                        && end - start < 64
                    {
                        end += 1;
                    }
                    let run = &members[start..end];
                    start = end;
                    if run.len() < 2 {
                        continue;
                    }
                    scatter_widths[run.len()] += 1;
                    if run.len() < min_scatter_width {
                        continue;
                    }
                    let lo = run[0].1;
                    let width = run.len() as u32;
                    let dsts: Vec<u32> = run.iter().map(|(_, _, dst)| dst.sig_id).collect();
                    let mut dst_ids: HashSet<u32> = HashSet::default();
                    if dsts.iter().any(|&dst| !dst_ids.insert(dst)) {
                        continue;
                    }
                    let stride = i64::from(dsts[1]) - i64::from(dsts[0]);
                    let fixed_stride = dsts
                        .windows(2)
                        .all(|pair| i64::from(pair[1]) - i64::from(pair[0]) == stride);
                    let layout = if stride == 1 && fixed_stride {
                        0
                    } else if stride == -1 && fixed_stride {
                        1
                    } else if fixed_stride {
                        2
                    } else {
                        3
                    };
                    let (reads, op, kind) = match shape {
                        SShape::Buf { src, invert } => {
                            if dst_ids.contains(&src)
                                || self
                                    .signal_widths
                                    .get(src as usize)
                                    .is_none_or(|&w| lo + width > w)
                            {
                                continue;
                            }
                            let op = ScatterGate::Buf {
                                dsts: dsts.into_boxed_slice(),
                                src: VectorRef { sig_id: src, lo },
                                width: width as u8,
                                invert,
                            };
                            (vec![src as usize], op, 0)
                        }
                        SShape::Bin {
                            a,
                            b,
                            delta,
                            op,
                            invert,
                        } => {
                            let b_lo_i = lo as i64 + delta;
                            if b_lo_i < 0 {
                                continue;
                            }
                            let b_lo = b_lo_i as u32;
                            if dst_ids.contains(&a)
                                || dst_ids.contains(&b)
                                || self
                                    .signal_widths
                                    .get(a as usize)
                                    .is_none_or(|&w| lo + width > w)
                                || self
                                    .signal_widths
                                    .get(b as usize)
                                    .is_none_or(|&w| b_lo + width > w)
                            {
                                continue;
                            }
                            let mut reads = vec![a as usize];
                            if b != a {
                                reads.push(b as usize);
                            }
                            (
                                reads,
                                ScatterGate::Bin {
                                    dsts: dsts.into_boxed_slice(),
                                    a: VectorRef { sig_id: a, lo },
                                    b: VectorRef {
                                        sig_id: b,
                                        lo: b_lo,
                                    },
                                    width: width as u8,
                                    op,
                                    invert,
                                },
                                match op {
                                    GateBin::And => 1,
                                    GateBin::Or => 2,
                                    GateBin::Xor => 3,
                                },
                            )
                        }
                    };
                    scatter_layout_groups[layout] += 1;
                    scatter_layout_lanes[layout] += run.len();
                    scatter_kind_groups[kind] += 1;
                    scatter_kind_lanes[kind] += run.len();
                    let head = run.iter().map(|(idx, _, _)| *idx).min().unwrap();
                    entries[head].item = CombItem::ScatterGate { op };
                    entries[head].cold.read_signal_ids = reads;
                    entries[head].cold.write_signal_ids =
                        dst_ids.iter().map(|&id| id as usize).collect();
                    entries[head].has_unresolved_reads = false;
                    entries[head].defer_at_time0 = false;
                    for &(idx, _, _) in run {
                        if idx != head {
                            entries[idx].item = CombItem::Noop;
                            entries[idx].cold.read_signal_ids.clear();
                            entries[idx].cold.write_signal_ids.clear();
                            entries[idx].has_unresolved_reads = false;
                            entries[idx].defer_at_time0 = false;
                        }
                    }
                    vectors += 1;
                    members_removed += run.len() - 1;
                    bits += run.len();
                }
            }
            if std::env::var_os("XEZIM_VEC_STATS").is_some() {
                eprintln!(
                    "[VEC] vectors={} bits={} entries_removed={}",
                    vectors, bits, members_removed
                );
                let widths = scatter_widths
                    .iter()
                    .enumerate()
                    .filter(|&(_, &count)| count != 0)
                    .map(|(width, count)| format!("{}:{}", width, count))
                    .collect::<Vec<_>>()
                    .join(",");
                if !widths.is_empty() {
                    eprintln!("[VEC] scatter_widths={}", widths);
                    eprintln!(
                        "[VEC] scatter_layout groups=forward:{},reverse:{},strided:{},irregular:{} lanes=forward:{},reverse:{},strided:{},irregular:{}",
                        scatter_layout_groups[0],
                        scatter_layout_groups[1],
                        scatter_layout_groups[2],
                        scatter_layout_groups[3],
                        scatter_layout_lanes[0],
                        scatter_layout_lanes[1],
                        scatter_layout_lanes[2],
                        scatter_layout_lanes[3]
                    );
                    eprintln!(
                        "[VEC] scatter_kind groups=buf:{},and:{},or:{},xor:{} lanes=buf:{},and:{},or:{},xor:{}",
                        scatter_kind_groups[0],
                        scatter_kind_groups[1],
                        scatter_kind_groups[2],
                        scatter_kind_groups[3],
                        scatter_kind_lanes[0],
                        scatter_kind_lanes[1],
                        scatter_kind_lanes[2],
                        scatter_kind_lanes[3]
                    );
                }
            }
        }

        // R1 census (XEZIM_VEC_CENSUS=1, analysis-only): size the population
        // of per-bit comb entries that could re-coalesce into vector ops —
        // groups doing the SAME operation on same-offset bit slices of the
        // same parent vectors. Census-first per project discipline.
        if std::env::var("XEZIM_VEC_CENSUS").is_ok() {
            use super::bytecode::Insn as VI;
            #[derive(Hash, PartialEq, Eq)]
            struct VKey {
                dst: u32,
                shape: String,
                srcs: Vec<u32>,
                offs: Vec<i64>,
            }
            let mut buckets: std::collections::HashMap<VKey, Vec<u32>> =
                std::collections::HashMap::new();
            let mut n_entries = 0usize;
            let mut n_gate_vecdst = 0usize;
            let mut n_gate_scalar = 0usize;
            let mut n_bitca = 0usize;
            let mut n_small_unmatched = 0usize;
            let gate_dst = |b: &BitRef| (b.sig_id, b.bit);
            for e in entries.iter() {
                n_entries += 1;
                match &e.item {
                    CombItem::FusedGate { op } => {
                        let (dst, dbit, shape, srcs): (u32, u32, String, Vec<(u32, u32)>) = match op
                        {
                            FusedGate::Buf1 { dst, src, invert } => (
                                gate_dst(dst).0,
                                gate_dst(dst).1,
                                format!("buf{}", if *invert { "~" } else { "" }),
                                vec![(src.sig_id, src.bit)],
                            ),
                            FusedGate::Bin2 {
                                dst,
                                a,
                                b,
                                op,
                                invert,
                            } => (
                                gate_dst(dst).0,
                                gate_dst(dst).1,
                                format!("{:?}{}", op, if *invert { "~" } else { "" }),
                                vec![(a.sig_id, a.bit), (b.sig_id, b.bit)],
                            ),
                            FusedGate::Mux2 { dst, s, t, e } => (
                                gate_dst(dst).0,
                                gate_dst(dst).1,
                                "mux2".to_string(),
                                vec![(s.sig_id, s.bit), (t.sig_id, t.bit), (e.sig_id, e.bit)],
                            ),
                            FusedGate::UdpLut3 { dst, .. } => (
                                gate_dst(dst).0,
                                gate_dst(dst).1,
                                "udp3".to_string(),
                                Vec::new(),
                            ),
                        };
                        let dw = self.signal_widths.get(dst as usize).copied().unwrap_or(1);
                        if dw <= 1 {
                            n_gate_scalar += 1;
                            continue;
                        }
                        n_gate_vecdst += 1;
                        let key = VKey {
                            dst,
                            shape,
                            srcs: srcs.iter().map(|(s, _)| *s).collect(),
                            offs: srcs.iter().map(|(_, b)| *b as i64 - dbit as i64).collect(),
                        };
                        buckets.entry(key).or_default().push(dbit);
                    }
                    CombItem::CompiledContAssign { compiled }
                        if compiled.instructions.len() <= 10 =>
                    {
                        // Linear extract: reg -> (parent, bit) loads, opcode
                        // trail, final single-bit store.
                        let mut loads: Vec<(u32, u32)> = Vec::new();
                        let mut shape = String::new();
                        let mut store: Option<(u32, u32)> = None;
                        let mut ok = true;
                        for ins in compiled.instructions.iter() {
                            match ins {
                                VI::LoadSignalBit(_, id, b) => loads.push((*id, *b)),
                                VI::LoadSignalRange(_, id, l, r) if l == r => loads.push((*id, *l)),
                                VI::LoadSignal(_, id)
                                    if self
                                        .signal_widths
                                        .get(*id as usize)
                                        .is_some_and(|&w| w == 1) =>
                                {
                                    loads.push((*id, 0))
                                }
                                VI::BitAnd(..) => shape.push('&'),
                                VI::BitOr(..) => shape.push('|'),
                                VI::BitXor(..) => shape.push('^'),
                                VI::BitNot(..) => shape.push('~'),
                                VI::Move(..) | VI::MoveResize(..) | VI::Resize(..) => {}
                                VI::BlockingAssignRange(id, hi, lo, _) if hi == lo => {
                                    store = Some((*id, *hi));
                                }
                                VI::BlockingAssign(id, _, w)
                                    if *w == 1
                                        && self
                                            .signal_widths
                                            .get(*id as usize)
                                            .is_some_and(|&sw| sw == 1) =>
                                {
                                    store = Some((*id, 0));
                                }
                                _ => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        let Some((dst, dbit)) = store else {
                            n_small_unmatched += 1;
                            continue;
                        };
                        if !ok || loads.is_empty() {
                            n_small_unmatched += 1;
                            continue;
                        }
                        let dw = self.signal_widths.get(dst as usize).copied().unwrap_or(1);
                        if dw <= 1 {
                            n_gate_scalar += 1;
                            continue;
                        }
                        n_bitca += 1;
                        if shape.is_empty() {
                            shape.push('=');
                        }
                        let key = VKey {
                            dst,
                            shape,
                            srcs: loads.iter().map(|(s, _)| *s).collect(),
                            offs: loads.iter().map(|(_, b)| *b as i64 - dbit as i64).collect(),
                        };
                        buckets.entry(key).or_default().push(dbit);
                    }
                    _ => {}
                }
            }
            let mut sizes: Vec<(usize, &VKey)> =
                buckets.iter().map(|(k, v)| (v.len(), k)).collect();
            sizes.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            let coalescable: usize = sizes.iter().filter(|(n, _)| *n >= 2).map(|(n, _)| n).sum();
            let groups = sizes.iter().filter(|(n, _)| *n >= 2).count();
            eprintln!(
                "[VEC-CENSUS] entries={} bit-entries: gate_vecdst={} bitca={} gate_scalar={} small_unmatched={}",
                n_entries, n_gate_vecdst, n_bitca, n_gate_scalar, n_small_unmatched
            );
            eprintln!(
                "[VEC-CENSUS] coalescable={} in {} groups (>=2 members) -> ratio {:.1}x on that set",
                coalescable,
                groups,
                coalescable as f64 / groups.max(1) as f64
            );
            for (n, k) in sizes.iter().take(12) {
                let name = self.name_opt(k.dst as usize).unwrap_or("?");
                eprintln!(
                    "[VEC-CENSUS]   members={:5} dst={} shape={} srcs={} offs={:?}",
                    n,
                    name,
                    k.shape,
                    k.srcs.len(),
                    k.offs
                );
            }
        }
        // Cluster census (XEZIM_CLUSTER_CENSUS=1, analysis-only) — the go/no-go
        // gate for native clustering (see the gap analysis). Forms clusters the
        // way the region pass does (dependency-connected, size-capped) and then
        // answers the question that decides the payoff: of the nets a cluster
        // writes, how many are read ONLY inside that cluster and could
        // therefore live in a local instead of being published to the signal
        // table? Every interior net that stays local also deletes its write
        // bridge, its dirty marking and its worklist re-trigger.
        //
        // Readers counted here are comb entries and edge blocks. Processes,
        // dumps and VPI can read anything by name, so the interior figure is
        // an UPPER BOUND that a visibility contract would have to license —
        // exactly the gap that made the dead-cone prune unsound.
        if std::env::var("XEZIM_CLUSTER_CENSUS").is_ok() {
            use super::bytecode::Insn as KI;
            let cap: usize = std::env::var("XEZIM_REGION_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(16);
            let n = entries.len();
            // Producer map over comb entries.
            let mut producer: HashMap<usize, usize> = HashMap::default();
            for (i, e) in entries.iter().enumerate() {
                for &w in &e.cold.write_signal_ids {
                    producer.insert(w, i);
                }
            }
            // Optional co-activation gate: when XEZIM_COACT_FILE names an edge
            // file from a census run of the SAME design, grow only along edges
            // whose measured same-slot ratio clears XEZIM_COACT_MIN. Gating is
            // required for the real transform anyway — ungated merging brings
            // back the recompute waste that made bytecode fusion negative — so
            // the interior share has to be measured under it, not without it.
            let coact_min: f64 = std::env::var("XEZIM_COACT_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.9);
            let coact: Option<std::collections::HashSet<(u32, u32)>> =
                std::env::var("XEZIM_COACT_FILE").ok().and_then(|path| {
                    let text = std::fs::read_to_string(&path).ok()?;
                    let mut lines = text.lines();
                    let file_n: usize =
                        lines.next()?.strip_prefix("entries ")?.parse().ok()?;
                    if file_n != n {
                        eprintln!(
                            "[CLUSTER-CENSUS] coact file has {file_n} entries, this run has {n}; ignoring"
                        );
                        return None;
                    }
                    let mut set = std::collections::HashSet::new();
                    for l in lines {
                        let mut it = l.split_whitespace();
                        let (Some(a), Some(b), Some(r)) = (it.next(), it.next(), it.next())
                        else {
                            continue;
                        };
                        if let (Ok(a), Ok(b), Ok(r)) =
                            (a.parse::<u32>(), b.parse::<u32>(), r.parse::<f64>())
                        {
                            if r >= coact_min {
                                set.insert((a, b));
                            }
                        }
                    }
                    eprintln!(
                        "[CLUSTER-CENSUS] coact-gated growth: {} edges at ratio >= {}",
                        set.len(),
                        coact_min
                    );
                    Some(set)
                });
            let edge_ok = |consumer: usize, producer: usize| -> bool {
                match &coact {
                    None => true,
                    Some(set) => set.contains(&(consumer as u32, producer as u32)),
                }
            };
            // Consumers of each entry, so growth can follow dataflow in both
            // directions.
            let mut consumers: Vec<Vec<usize>> = vec![Vec::new(); n];
            for (i, e) in entries.iter().enumerate() {
                for &r in &e.cold.read_signal_ids {
                    if let Some(&p) = producer.get(&r) {
                        if p != i {
                            consumers[p].push(i);
                        }
                    }
                }
            }
            // Signals read by EDGE blocks (a cluster can never keep those local).
            let mut edge_reads: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for (bi, cb) in self.compiled_edge_blocks.iter().enumerate() {
                if let Some(b) = self.edge_blocks.get(bi) {
                    for t in &b.resolved_sensitivities {
                        edge_reads.insert(t.signal_id);
                    }
                }
                let Some(cb) = cb else { continue };
                for ins in cb.instructions.iter() {
                    match ins {
                        KI::LoadSignal(_, sg)
                        | KI::LoadSignalSigned(_, sg)
                        | KI::LoadSignalBit(_, sg, _)
                        | KI::LoadSignalRange(_, sg, _, _)
                        | KI::BranchIfSignalFalse(sg, _, _) => {
                            edge_reads.insert(*sg as usize);
                        }
                        _ => {}
                    }
                }
            }
            // GREEDY LOCALITY-PRESERVING GROWTH: seed at an unassigned entry
            // and pull in its producers and consumers breadth-first up to the
            // cap. Measured against the naive alternative (chunking a
            // dependency component in index order) this barely moves the
            // interior share — 8.3% -> 8.4% at cap 8 — which is itself the
            // finding: on a gate-level netlist the fanout is so wide that a
            // net escapes almost any cluster boundary, however it is cut.
            let mut cluster_of: HashMap<usize, usize> = HashMap::default();
            let mut sizes: Vec<usize> = Vec::new();
            let mut cid = 0usize;
            for seed in 0..n {
                if cluster_of.contains_key(&seed) {
                    continue;
                }
                let mut queue = std::collections::VecDeque::new();
                queue.push_back(seed);
                let mut members = 0usize;
                while let Some(m) = queue.pop_front() {
                    if members >= cap || cluster_of.contains_key(&m) {
                        continue;
                    }
                    cluster_of.insert(m, cid);
                    members += 1;
                    for &r in &entries[m].cold.read_signal_ids {
                        if let Some(&p) = producer.get(&r) {
                            if !cluster_of.contains_key(&p) && edge_ok(m, p) {
                                queue.push_back(p);
                            }
                        }
                    }
                    for &c in &consumers[m] {
                        if !cluster_of.contains_key(&c) && edge_ok(c, m) {
                            queue.push_back(c);
                        }
                    }
                }
                sizes.push(members);
                cid += 1;
            }
            // Reader sets per signal, restricted to comb entries.
            let mut interior = 0usize;
            let mut boundary = 0usize;
            let mut boundary_edge = 0usize;
            let mut unread = 0usize;
            let mut readers: HashMap<usize, Vec<usize>> = HashMap::default();
            for (i, e) in entries.iter().enumerate() {
                for &r in &e.cold.read_signal_ids {
                    readers.entry(r).or_default().push(i);
                }
            }
            for (i, e) in entries.iter().enumerate() {
                let Some(&my) = cluster_of.get(&i) else {
                    continue;
                };
                for &w in &e.cold.write_signal_ids {
                    if edge_reads.contains(&w) {
                        boundary += 1;
                        boundary_edge += 1;
                        continue;
                    }
                    match readers.get(&w) {
                        None => unread += 1,
                        Some(rs) => {
                            if rs.iter().all(|r| cluster_of.get(r) == Some(&my)) {
                                interior += 1;
                            } else {
                                boundary += 1;
                            }
                        }
                    }
                }
            }
            sizes.sort_unstable_by(|a, b| b.cmp(a));
            let clustered: usize = sizes.iter().filter(|&&s| s >= 2).sum();
            let nclusters = sizes.iter().filter(|&&s| s >= 2).count();
            eprintln!(
                "[CLUSTER-CENSUS] cap={} entries={} clustered={} ({:.1}%) in {} clusters (mean {:.1}, largest {:?})",
                cap,
                n,
                clustered,
                100.0 * clustered as f64 / n.max(1) as f64,
                nclusters,
                clustered as f64 / nclusters.max(1) as f64,
                &sizes[..sizes.len().min(6)]
            );
            eprintln!(
                "[CLUSTER-CENSUS] written nets: interior(could stay LOCAL)={} boundary={} (of which read by EDGE blocks/flops {}) unread={} -> interior share {:.1}%",
                interior,
                boundary,
                boundary_edge,
                unread,
                100.0 * interior as f64 / (interior + boundary).max(1) as f64
            );
        }

        // Step 9 (comb-region fusion, opt-in XEZIM_REGIONS=1): fuse
        // dependency-connected COMPILED comb entries into one larger
        // compiled block, so a value hop between members skips the whole
        // dispatch round trip (write -> trigger -> worklist -> re-dispatch)
        // and the two-state / JIT / AOT backends see one bigger body.
        // Rules:
        //  - members are COMB entries only — edge (flop) blocks are never
        //    candidates; a flop-driven signal enters a region strictly as an
        //    external input;
        //  - classes never mix: cont-assign entries defer during a process's
        //    post-write settle (#35) while always bodies stay eager, so a
        //    region is all-cont-assign or all-always_comb;
        //  - the fused read set is the FULL union of member reads (internal
        //    link signals included), so the settle fixpoint semantics are
        //    bit-identical — an internal value change re-triggers the region
        //    for one extra converging pass instead of being assumed settled;
        //  - member programs are self-contained register-wise (every reg is
        //    written before read; vm_regs are never cleared between entries
        //    today), so concatenation needs no register renumbering — but
        //    BRANCH TARGETS are block-relative and DO get offset;
        //  - members whose body contains fallback/NBA/process-local/jump-
        //    table insns stay out (targets or side effects we can't carry).
        if matches!(std::env::var("XEZIM_REGIONS").as_deref(), Ok("1")) {
            use super::bytecode::Insn as RIns;
            let max_members: usize = std::env::var("XEZIM_REGION_MAX")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|&v| v >= 2)
                .unwrap_or(8);
            let n = entries.len();
            let insn_ok = |c: &super::bytecode::CompiledBlock| -> bool {
                c.instructions.iter().all(|ins| {
                    !matches!(
                        ins,
                        RIns::StmtFallback(..)
                            | RIns::CovHit(..)
                            | RIns::EvalExprFallback(..)
                            | RIns::LoadProcessLocal(..)
                            | RIns::Format(..)
                            | RIns::CaseJump(..)
                            | RIns::CaseMaskJump(..)
                            | RIns::NbaAssign(..)
                            | RIns::NbaAssignConst(..)
                            | RIns::NbaAssignRange(..)
                            | RIns::NbaAssignRangeDyn(..)
                            | RIns::NbaAssignBitDyn(..)
                            | RIns::NbaAssignArray(..)
                            | RIns::NbaAssignArrayRange(..)
                            | RIns::NbaAssignArrayRead(..)
                    )
                })
            };
            let mut writer_count: HashMap<usize, u32> = HashMap::default();
            for e in entries.iter() {
                for &w in &e.cold.write_signal_ids {
                    *writer_count.entry(w).or_insert(0) += 1;
                }
            }
            // Class per entry: 0 = cont-assign, 1 = always_comb,
            // 2 = plain `always @(...)` (t0-deferred). Members fuse only
            // within a class so the #35 defer split AND the §9.2.2.1 t0
            // deferral stay uniform across the fused entry — a region of
            // plain-always members keeps defer_at_time0 = true.
            let mut class: Vec<Option<u8>> = Vec::with_capacity(n);
            for e in entries.iter() {
                // A member that is not individually width-safe would poison
                // its WHOLE fused region out of native compilation (the
                // fused block inherits every member's signals, and the
                // width gate is all-or-nothing). Measured on c906: without
                // this check, fusing 6,285 members DEMOTED 4,563 entries
                // from native to interpreted and cost 24% wall. Keep such
                // members unfused — they stay native individually.
                // Members must ALSO two-state-lower individually: fusing a
                // ts-lowered member into a non-lowerable block moves it off
                // the cheapest execution tier. Measured on c906: ts evals
                // collapsed 158.9M -> 71.6M under fusion without this check,
                // cancelling the eval-count win.
                let ts_ok = |cb: &super::bytecode::CompiledBlock| -> bool {
                    super::bytecode::lower_two_state(
                        cb,
                        &self.signal_widths,
                        &self.signal_signed,
                        &self.signal_real,
                        &self.array_first_id,
                    )
                    .is_some()
                };
                let c = match &e.item {
                    CombItem::CompiledContAssign { compiled }
                        if !compiled.has_fallback
                            && insn_ok(compiled)
                            && self.block_signals_fit_u64(compiled)
                            && ts_ok(compiled) =>
                    {
                        Some(0u8)
                    }
                    CombItem::CompiledAlwaysBlock {
                        compiled,
                        is_always_comb,
                    } if !compiled.has_fallback
                        && insn_ok(compiled)
                        && self.block_signals_fit_u64(compiled)
                        && ts_ok(compiled) =>
                    {
                        Some(if *is_always_comb { 1 } else { 2 })
                    }
                    // 3 = fused 1-bit gate primitive. Needs none of the
                    // compiled-block gates (insn/width/ts) — a GateRegion
                    // executes via fused_gate_eval, never via bytecode.
                    // Opt-in: measured net-NEGATIVE on c906 (+4-9s settle).
                    // Gate clusters there are clock/ICG trees whose mid-chain
                    // outputs feed external consumers at every slot position,
                    // so any single region slot delays some consumers by one
                    // settle pass (iters +13-50%), outweighing the dispatch
                    // saving (gate dispatch is already the cheapest arm).
                    CombItem::FusedGate { .. }
                        if std::env::var("XEZIM_REGION_GATES").is_ok_and(|v| v == "1") =>
                    {
                        Some(3u8)
                    }
                    _ => None,
                };
                let ok = c.is_some()
                    && !e.has_unresolved_reads
                    && !e.cold.read_signal_ids.is_empty()
                    && e.cold
                        .write_signal_ids
                        .iter()
                        .all(|w| writer_count.get(w).copied().unwrap_or(0) == 1);
                class.push(if ok { c } else { None });
            }
            // PGO: when XEZIM_COACT_FILE names an edge file from a census
            // run of the SAME design/binary (entry count must match), only
            // merge along edges whose measured same-slot co-activation ratio
            // clears XEZIM_COACT_MIN (default 0.9). This bounds the fused
            // region's recompute waste — the defect that made blind fusion
            // net-negative twice.
            let coact_min: f64 = std::env::var("XEZIM_COACT_MIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.9);
            let coact_edges: Option<std::collections::HashSet<(u32, u32)>> =
                std::env::var("XEZIM_COACT_FILE").ok().and_then(|path| {
                    let text = std::fs::read_to_string(&path).ok()?;
                    let mut lines = text.lines();
                    let head = lines.next()?;
                    let file_n: usize = head.strip_prefix("entries ")?.parse().ok()?;
                    if file_n != n {
                        eprintln!(
                            "[REGIONS] coact file entry count {file_n} != {n}; ignoring file"
                        );
                        return None;
                    }
                    let mut set = std::collections::HashSet::new();
                    for l in lines {
                        let mut it = l.split_whitespace();
                        let (Some(e), Some(p), Some(r)) = (it.next(), it.next(), it.next()) else {
                            continue;
                        };
                        let (Ok(e), Ok(p), Ok(r)) =
                            (e.parse::<u32>(), p.parse::<u32>(), r.parse::<f64>())
                        else {
                            continue;
                        };
                        if r >= coact_min {
                            set.insert((e, p));
                        }
                    }
                    eprintln!(
                        "[REGIONS] coact-gated fusion: {} qualifying edges (min ratio {})",
                        set.len(),
                        coact_min
                    );
                    Some(set)
                });
            let mut producer: HashMap<usize, usize> = HashMap::default();
            for (i, e) in entries.iter().enumerate() {
                if class[i].is_some() {
                    for &w in &e.cold.write_signal_ids {
                        producer.insert(w, i);
                    }
                }
            }
            let mut uf: Vec<usize> = (0..n).collect();
            fn rfind(uf: &mut [usize], mut x: usize) -> usize {
                while uf[x] != x {
                    uf[x] = uf[uf[x]];
                    x = uf[x];
                }
                x
            }
            for i in 0..n {
                if class[i].is_none() {
                    continue;
                }
                // Borrow dance: collect the producer links first.
                let links: Vec<usize> = entries[i]
                    .cold
                    .read_signal_ids
                    .iter()
                    .filter_map(|r| producer.get(r).copied())
                    .filter(|&p| p != i && class[p] == class[i])
                    .filter(|&p| {
                        coact_edges
                            .as_ref()
                            .map(|set| set.contains(&(i as u32, p as u32)))
                            .unwrap_or(true)
                    })
                    .collect();
                for p in links {
                    let (a, b) = (rfind(&mut uf, i), rfind(&mut uf, p));
                    if a != b {
                        uf[a] = b;
                    }
                }
            }
            let mut comps: HashMap<usize, Vec<usize>> = HashMap::default();
            for i in 0..n {
                if class[i].is_some() {
                    let root = rfind(&mut uf, i);
                    comps.entry(root).or_default().push(i);
                }
            }
            let mut fused_regions = 0usize;
            let mut fused_members = 0usize;
            for (_, mut members) in comps {
                if members.len() < 2 {
                    continue;
                }
                members.sort_unstable();
                // Entry-index order is NOT topo order (entries are built in
                // declaration order): a consumer that precedes its producer
                // inside a region reads a stale value on every evaluation and
                // costs one extra converging settle pass per firing — chained
                // across regions this measured ~33 passes/settle on ibex.
                // Kahn-sort the component's members over the internal
                // producer→consumer edges (idx as the deterministic
                // tie-break); a cycle falls back to idx order for its
                // remainder, which the kept feedback reads still converge.
                let order: Vec<usize> = {
                    let pos: HashMap<usize, usize> =
                        members.iter().enumerate().map(|(k, &m)| (m, k)).collect();
                    let mut indeg = vec![0usize; members.len()];
                    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); members.len()];
                    for (k, &m) in members.iter().enumerate() {
                        for &r in &entries[m].cold.read_signal_ids {
                            if let Some(&p) = producer.get(&r) {
                                if let Some(&pk) = pos.get(&p) {
                                    if pk != k {
                                        succ[pk].push(k);
                                        indeg[k] += 1;
                                    }
                                }
                            }
                        }
                    }
                    let mut ready: Vec<usize> =
                        (0..members.len()).filter(|&k| indeg[k] == 0).collect();
                    ready.sort_unstable_by_key(|&k| members[k]);
                    let mut out: Vec<usize> = Vec::with_capacity(members.len());
                    let mut done = vec![false; members.len()];
                    while let Some(k) = ready.first().copied() {
                        ready.remove(0);
                        if done[k] {
                            continue;
                        }
                        done[k] = true;
                        out.push(members[k]);
                        for &n in &succ[k] {
                            indeg[n] = indeg[n].saturating_sub(1);
                            if indeg[n] == 0 && !done[n] {
                                let key = members[n];
                                let at = ready
                                    .binary_search_by_key(&key, |&x| members[x])
                                    .unwrap_or_else(|e| e);
                                ready.insert(at, n);
                            }
                        }
                    }
                    // Cycle remainder: append in idx order.
                    for (k, &m) in members.iter().enumerate() {
                        if !done[k] {
                            out.push(m);
                        }
                    }
                    out
                };
                for chunk in order.chunks(max_members) {
                    if chunk.len() < 2 {
                        continue;
                    }
                    // The fused entry lives at the LOWEST member slot so its
                    // scheduling position (same-pass cascade is idx-ordered)
                    // is never later than any member's was; the BODY executes
                    // members in topo order regardless of slot choice.
                    // (Max-slot was tried: it delays every external consumer
                    // sitting between min and max member by one settle pass —
                    // measured settle 168.1s vs 165.0s on c906 it1.)
                    let head = chunk.iter().copied().min().unwrap();
                    // RegId is u16: renumbering offsets member registers by a
                    // running base, so the fused total must stay in range.
                    let total_regs: u64 = chunk
                        .iter()
                        .map(|&m| match &entries[m].item {
                            CombItem::CompiledContAssign { compiled }
                            | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                                compiled.num_regs as u64
                            }
                            _ => 0,
                        })
                        .sum();
                    if total_regs > u16::MAX as u64 {
                        continue;
                    }
                    // Sensitivity: a link signal whose producer member runs
                    // BEFORE every member that reads it is satisfied by the
                    // region's own execution order and is DROPPED from the
                    // fused read set — keeping it made each firing re-trigger
                    // the region across settle passes (the historical 11x
                    // over-trigger trap: a chain of L members cost ~L extra
                    // full-region passes). Feedback reads (producer at or
                    // after the consumer) stay, preserving the fixpoint.
                    // Dropping is sound: candidates are the SINGLE comb
                    // writer of their outputs, and a comb-driven variable has
                    // no legal procedural co-driver (§10.3.2) — a flop can
                    // only feed a region as an external input, never rewrite
                    // an internal link.
                    let mpos: HashMap<usize, usize> =
                        chunk.iter().enumerate().map(|(pos, &m)| (m, pos)).collect();
                    let mut region_reads: Vec<usize> = Vec::new();
                    let mut region_writes: Vec<usize> = Vec::new();
                    for (pos, &m) in chunk.iter().enumerate() {
                        for &r in &entries[m].cold.read_signal_ids {
                            let satisfied = producer
                                .get(&r)
                                .and_then(|pm| mpos.get(pm))
                                .is_some_and(|&ppos| ppos < pos);
                            if !satisfied && !region_reads.contains(&r) {
                                region_reads.push(r);
                            }
                        }
                        for &w in &entries[m].cold.write_signal_ids {
                            if !region_writes.contains(&w) {
                                region_writes.push(w);
                            }
                        }
                    }
                    if class[chunk[0]] == Some(3) {
                        // Gate cluster: one worklist entry executing the
                        // member gates in topo (chunk) order via the same
                        // per-gate evaluator. Same head/Noop slot layout and
                        // link-dropped sensitivity as compiled regions.
                        let gates: Vec<FusedGate> = chunk
                            .iter()
                            .map(|&m| match &entries[m].item {
                                CombItem::FusedGate { op } => *op,
                                _ => unreachable!("class 3 admits only FusedGate"),
                            })
                            .collect();
                        entries[head].item = CombItem::GateRegion {
                            gates: gates.into_boxed_slice(),
                        };
                        entries[head].cold.read_signal_ids = region_reads;
                        entries[head].cold.write_signal_ids = region_writes;
                        for &m in chunk {
                            if m == head {
                                continue;
                            }
                            entries[m].item = CombItem::Noop;
                            entries[m].cold.read_signal_ids.clear();
                            entries[m].cold.write_signal_ids.clear();
                        }
                        fused_regions += 1;
                        fused_members += chunk.len();
                        continue;
                    }
                    let mut fused = super::bytecode::CompiledBlock {
                        instructions: Vec::new(),
                        num_regs: 0,
                        has_fallback: false,
                        nba_dup_targets: false,
                        foreach_slots: 0,
                        static_locals: Box::new([]),
                    };
                    for &m in chunk {
                        let mb = match &entries[m].item {
                            CombItem::CompiledContAssign { compiled }
                            | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled,
                            _ => unreachable!("class gate admits only compiled items"),
                        };
                        let off = fused.instructions.len() as u32;
                        let reg_base = fused.num_regs as u16;
                        for ins in mb.instructions.iter() {
                            let mut ins = ins.clone();
                            let ok = ins.offset_regs_and_targets(reg_base, off);
                            debug_assert!(ok, "insn_ok admitted an un-offsettable insn");
                            fused.instructions.push(ins);
                        }
                        fused.num_regs += mb.num_regs;
                        fused.nba_dup_targets |= mb.nba_dup_targets;
                    }
                    let is_ab = matches!(entries[head].item, CombItem::CompiledAlwaysBlock { .. });
                    let always_comb = chunk.iter().any(|&m| {
                        matches!(
                            entries[m].item,
                            CombItem::CompiledAlwaysBlock {
                                is_always_comb: true,
                                ..
                            }
                        )
                    });
                    entries[head].item = if is_ab {
                        CombItem::CompiledAlwaysBlock {
                            compiled: fused,
                            is_always_comb: always_comb,
                        }
                    } else {
                        CombItem::CompiledContAssign { compiled: fused }
                    };
                    entries[head].cold.read_signal_ids = region_reads;
                    entries[head].cold.write_signal_ids = region_writes;
                    for &m in chunk {
                        if m == head {
                            continue;
                        }
                        entries[m].item = CombItem::Noop;
                        entries[m].cold.read_signal_ids.clear();
                        entries[m].cold.write_signal_ids.clear();
                    }
                    fused_regions += 1;
                    fused_members += chunk.len();
                }
            }
            if std::env::var("XEZIM_REGION_STATS").is_ok() {
                eprintln!(
                    "[REGION] fused {} members into {} regions (cap {}) of {} entries",
                    fused_members, fused_regions, max_members, n
                );
            }
        }

        // Build reverse dependency index by signal ID using final entry
        // order. CSR layout: counts → prefix-sum → fill, all in flat
        // u32 Vecs. Avoids 585K × 24 B of empty Vec headers and 585K
        // separate small allocations.
        let mut counts: Vec<u32> = vec![0u32; num_signals + 1];
        for entry in entries.iter() {
            for &sig_id in &entry.cold.read_signal_ids {
                if sig_id < num_signals {
                    counts[sig_id + 1] += 1;
                }
            }
        }
        // In-place prefix sum: counts[i] becomes start-offset for signal i.
        let mut dep_offsets = counts;
        for i in 1..dep_offsets.len() {
            dep_offsets[i] += dep_offsets[i - 1];
        }
        let total = dep_offsets[num_signals] as usize;
        let mut dep_entries: Vec<u32> = vec![0u32; total];
        // `cursor[id]` tracks the next write position for signal `id`;
        // initialized to the start-offset and bumped per insert.
        let mut cursor: Vec<u32> = dep_offsets[..num_signals].to_vec();
        for (idx, entry) in entries.iter().enumerate() {
            for &sig_id in &entry.cold.read_signal_ids {
                if sig_id < num_signals {
                    let pos = cursor[sig_id] as usize;
                    dep_entries[pos] = idx as u32;
                    cursor[sig_id] += 1;
                }
            }
        }
        self.comb_dep_offsets = dep_offsets;
        self.comb_dep_entries = dep_entries;
        self.comb_dep_edges.clear();
        self.gate_lane_valid = false;
        // Co-activation census setup: predecessor CSR (entry -> writer
        // entries of its read signals, capped at 16 preds/entry). Built here
        // so it reflects the FINAL entry order, learning from the topo-
        // reorder dep-index bug.
        self.coact_enabled = std::env::var("XEZIM_COACT_CENSUS")
            .map(|v| v == "1")
            .unwrap_or(false);
        if self.coact_enabled {
            let n = entries.len();
            let mut writer_of: HashMap<usize, Vec<u32>> = HashMap::default();
            for (e, ent) in entries.iter().enumerate() {
                for &w in &ent.cold.write_signal_ids {
                    let v = writer_of.entry(w).or_default();
                    if v.len() < 4 {
                        v.push(e as u32);
                    }
                }
            }
            let mut off: Vec<u32> = Vec::with_capacity(n + 1);
            let mut ents: Vec<u32> = Vec::new();
            off.push(0);
            for (e, ent) in entries.iter().enumerate() {
                let start = ents.len();
                'reads: for &r in &ent.cold.read_signal_ids {
                    if let Some(ws) = writer_of.get(&r) {
                        for &w in ws {
                            if w as usize != e && !ents[start..].contains(&w) {
                                ents.push(w);
                                if ents.len() - start >= 16 {
                                    break 'reads;
                                }
                            }
                        }
                    }
                }
                off.push(ents.len() as u32);
            }
            self.coact_edge_count = vec![0; ents.len()];
            self.coact_pred_off = off;
            self.coact_pred_ent = ents;
            self.coact_epoch = vec![u64::MAX; n];
        }
        if std::env::var("XEZIM_DEP_STATS").is_ok() {
            let mut nonzero = 0usize;
            let mut max_fanout = 0usize;
            let mut max_id = 0usize;
            for id in 0..num_signals {
                let fanout =
                    self.comb_dep_offsets[id + 1] as usize - self.comb_dep_offsets[id] as usize;
                if fanout > 0 {
                    nonzero += 1;
                    if fanout > max_fanout {
                        max_fanout = fanout;
                        max_id = id;
                    }
                }
            }
            eprintln!(
                "[DEP_STATS] dep_edges={} dep_signals={} avg_dep_fanout={:.2} max_dep_fanout={} max_signal={}",
                total,
                nonzero,
                if nonzero > 0 {
                    total as f64 / nonzero as f64
                } else {
                    0.0
                },
                max_fanout,
                self.name_for_id(max_id)
            );
        }
        let dc_count = entries
            .iter()
            .filter(|e| {
                matches!(
                    &e.item,
                    CombItem::DirectCopy { .. } | CombItem::FastDirectCopy { .. }
                )
            })
            .count();
        let cca_count = entries
            .iter()
            .filter(|e| matches!(&e.item, CombItem::CompiledContAssign { .. }))
            .count();
        let ca_count = entries
            .iter()
            .filter(|e| matches!(&e.item, CombItem::ContAssign { .. }))
            .count();
        let noop_count = entries
            .iter()
            .filter(|e| matches!(&e.item, CombItem::Noop))
            .count();
        let ab_count = entries
            .iter()
            .filter(|e| matches!(&e.item, CombItem::AlwaysBlock { .. }))
            .count();
        let fg_count = entries
            .iter()
            .filter(|e| matches!(&e.item, CombItem::FusedGate { .. }))
            .count();
        if std::env::var("XEZIM_DEP_STATS").is_ok() {
            eprintln!(
                "[COMB_STATS] direct_copy={} compiled_ca={} ast_ca={} noop={} always_block={} fused_gate={}",
                dc_count, cca_count, ca_count, noop_count, ab_count, fg_count
            );
        }
        if std::env::var("XEZIM_COMPILE_FAIL_STATS").is_ok() && !ca_compile_fail.is_empty() {
            let mut items: Vec<_> = ca_compile_fail.into_iter().collect();
            items.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            eprintln!("[COMPILE_FAIL_STATS] continuous assign bytecode failures:");
            for (reason, count) in items.into_iter().take(20) {
                eprintln!("[COMPILE_FAIL_STATS] {:>8} {}", count, reason);
            }
        }
        if dc_count > 0 || fg_count > 0 {
            sim_dbg_eprintln!(
                "[OPT] comb entries: {} direct-copy, {} compiled-ca, {} ast-ca, {} noop, {} always-block, {} fused-gate",
                dc_count,
                cca_count,
                ca_count,
                noop_count,
                ab_count,
                fg_count
            );
            sim_dbg_eprintln!(
                "[OPT] edge blocks: {}, event_waiters: {}",
                self.edge_blocks.len(),
                self.event_waiters.len()
            );
        }
        self.comb_unresolved_idx = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                if e.has_unresolved_reads {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        // Unresolved entries re-evaluate on EVERY settle call, so a single
        // AST-interpreted one can dominate a run (measured: 6.2s of 37.4s
        // settle time on Ibex CoreMark, one eval per settle call). Name them
        // so the cost is attributable instead of anonymous.
        if std::env::var("XEZIM_UNRESOLVED_DUMP").is_ok() {
            eprintln!(
                "[UNRESOLVED] {} entries re-evaluate every settle call:",
                self.comb_unresolved_idx.len()
            );
            for &i in self.comb_unresolved_idx.iter() {
                let e = &entries[i];
                let kind = match &e.item {
                    CombItem::CompiledContAssign { .. } => "compiled-cont-assign",
                    CombItem::ContAssign { .. } => "AST-cont-assign",
                    CombItem::DirectCopy { .. } => "direct-copy",
                    CombItem::FastDirectCopy { .. } => "fast-direct-copy",
                    CombItem::CompiledAlwaysBlock { .. } => "compiled-always",
                    CombItem::AlwaysBlock { .. } => "AST-always",
                    _ => "other",
                };
                let wn: Vec<&str> = e
                    .cold
                    .write_signal_ids
                    .iter()
                    .filter_map(|&w| self.name_opt(w))
                    .take(4)
                    .collect();
                eprintln!("[UNRESOLVED]   #{i} {kind} writes={wn:?}");
            }
        }
        self.comb_time0_idx = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let always_comb = matches!(
                    &e.item,
                    CombItem::AlwaysBlock {
                        is_always_comb: true,
                        ..
                    } | CombItem::CompiledAlwaysBlock {
                        is_always_comb: true,
                        ..
                    }
                );
                // §9.2.2.2: an always_comb with an EMPTY inferred sensitivity
                // list (no inputs) is held out of the pre-initial settle and
                // fired at the end of the time-0 active region instead (see
                // `comb_time0_deferred`). always_comb blocks WITH inputs still
                // settle at time 0 via normal dirty propagation, so excluding
                // the empty ones here changes nothing for them.
                if always_comb && e.cold.read_signal_ids.is_empty() {
                    return None;
                }
                if e.cold.read_signal_ids.is_empty() || always_comb {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();
        // Clone the empty-sensitivity always_comb entries into the deferred
        // list; they fire once at the end of time-0's active region.
        self.comb_time0_deferred = entries
            .iter()
            .filter(|e| {
                let always_comb = matches!(
                    &e.item,
                    CombItem::AlwaysBlock {
                        is_always_comb: true,
                        ..
                    } | CombItem::CompiledAlwaysBlock {
                        is_always_comb: true,
                        ..
                    }
                );
                always_comb && e.cold.read_signal_ids.is_empty()
            })
            .cloned()
            .collect();
        // Cone-merge OPPORTUNITY analysis (diagnostic only; XEZIM_CONE=1).
        //
        // A compiled-code simulator evaluates whole fanin cones as one
        // function; xezim dispatches each tiny entry separately, paying the
        // per-eval constant every hop. The candidate for merging is a CHAIN:
        // entry A writes signal s, and B is the ONLY reader of s, and s has no
        // other writer — then A+B can become one entry with A's inputs and
        // B's outputs. This pass counts how much of the graph is chain-shaped
        // so the merge transform is built (or rejected) on numbers.
        if std::env::var("XEZIM_CONE").is_ok() {
            use xezim_core::hasher::HashMap;
            let n = entries.len();
            // signal -> reader entries / writer entries
            let mut readers: HashMap<usize, Vec<usize>> = HashMap::default();
            let mut writers: HashMap<usize, Vec<usize>> = HashMap::default();
            for (eidx, e) in entries.iter().enumerate() {
                for &r in e.cold.read_signal_ids.iter() {
                    readers.entry(r).or_default().push(eidx);
                }
                for &w in e.cold.write_signal_ids.iter() {
                    writers.entry(w).or_default().push(eidx);
                }
            }
            // A -> B edge when A writes s, B is s's sole reader, A its sole
            // writer, and B reads nothing else written by a third entry that
            // would break ordering (kept simple: just the sole-reader /
            // sole-writer test — an upper bound on mergeable pairs).
            let mut chain_next: Vec<Option<usize>> = vec![None; n];
            let mut chain_prev_cnt: Vec<u32> = vec![0; n];
            for (eidx, e) in entries.iter().enumerate() {
                let mut nexts: Vec<usize> = Vec::new();
                for &w in e.cold.write_signal_ids.iter() {
                    if writers.get(&w).map(|v| v.len()) != Some(1) {
                        nexts.clear();
                        break;
                    }
                    if let Some(rd) = readers.get(&w) {
                        for &b in rd.iter() {
                            if b != eidx {
                                nexts.push(b);
                            }
                        }
                    }
                }
                nexts.sort_unstable();
                nexts.dedup();
                if nexts.len() == 1 && nexts[0] != eidx {
                    chain_next[eidx] = Some(nexts[0]);
                    chain_prev_cnt[nexts[0]] += 1;
                }
            }
            // Walk maximal chains: start where prev_cnt == 0.
            let mut in_chain = 0usize;
            let mut chains = 0usize;
            let mut longest = 0usize;
            let mut len_hist: HashMap<usize, usize> = HashMap::default();
            for start in 0..n {
                if chain_prev_cnt[start] != 0 || chain_next[start].is_none() {
                    continue;
                }
                let mut len = 1usize;
                let mut cur = start;
                let mut guard = 0usize;
                let mut members: Vec<u32> = vec![start as u32];
                while let Some(nx) = chain_next[cur] {
                    // A node with multiple predecessors ends the chain.
                    if chain_prev_cnt[nx] != 1 {
                        break;
                    }
                    len += 1;
                    members.push(nx as u32);
                    cur = nx;
                    guard += 1;
                    if guard > n {
                        break;
                    }
                }
                if len >= 2 {
                    chains += 1;
                    in_chain += len;
                    longest = longest.max(len);
                    *len_hist.entry(len.min(10)).or_insert(0) += 1;
                    self.cone_chains.push(members);
                }
            }
            let mut hist: Vec<(usize, usize)> = len_hist.into_iter().collect();
            hist.sort();
            eprintln!(
                "[CONE] entries={} chain-members={} ({:.1}%) chains={} longest={} len-hist(capped@10)={:?}",
                n,
                in_chain,
                in_chain as f64 * 100.0 / n.max(1) as f64,
                chains,
                longest,
                hist
            );
            eprintln!(
                "[CONE] merging chains would remove {} dispatches ({:.1}% of entries) — upper bound, ordering unchecked",
                in_chain.saturating_sub(chains),
                (in_chain.saturating_sub(chains)) as f64 * 100.0 / n.max(1) as f64
            );
            // Eval-weighted share (needs XEZIM_PROFILE_TIMING=1 and is printed
            // at end-of-run instead when counts exist): static entry share
            // overstates the win if the HOT entries are not chain-shaped.
            self.cone_chain_member = {
                let mut v = vec![false; n];
                for (i, nx) in chain_next.iter().enumerate() {
                    if nx.is_some() || chain_prev_cnt[i] > 0 {
                        v[i] = true;
                    }
                }
                v
            };
        }
        if std::env::var("XEZIM_LAYOUT").is_ok() {
            eprintln!(
                "[LAYOUT] CombEntry={}B CombItem={}B Value={}B entries={} SensitivityId={}B Expression={}B EdgeSensitiveBlock={}B edge_blocks={} sens_terms={}",
                std::mem::size_of::<CombEntry>(),
                std::mem::size_of::<CombItem>(),
                std::mem::size_of::<Value>(),
                entries.len(),
                std::mem::size_of::<SensitivityId>(),
                std::mem::size_of::<Expression>(),
                std::mem::size_of::<EdgeSensitiveBlock>(),
                self.edge_blocks.len(),
                self.edge_blocks
                    .iter()
                    .map(|b| b.resolved_sensitivities.capacity())
                    .sum::<usize>()
            );
        }
        self.comb_entries = entries;
        self.write_prepared_comb_cache();
        // Drop AST storage for items we've consumed into comb_entries.
        // continuous_assigns live on in CombItem::ContAssign (fallback) or
        // as DirectCopy / FusedGate / CompiledContAssign; the source Vec in
        // the module is no longer read. Same for combinational always blocks
        // — edge-sensitive ones were moved into self.edge_blocks earlier.
        self.drop_comb_source_ast();
    }
}
