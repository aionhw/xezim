//! Class-object reclamation (IEEE 1800-2017 §8.7: an object no handle reaches
//! may be reclaimed). Mark from a conservative trace of every scheduler-owned
//! root, follow typed object edges, then sweep each unmarked object together
//! with its owned storage. Collection runs only at a time advance of the outer
//! event loop, where no Rust frame holds values outside the traced state.
use super::heap_trace::{HandleTrace, TraceError};
use super::*;

/// Fewest allocations between two collections.
const CLASS_GC_MIN_ALLOCS: usize = 20_000;

/// `XEZIM_CLASS_GC=0` disables reclamation; `XEZIM_CLASS_GC=stress` collects
/// at every time advance that allocated an object (for validating roots).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClassGcMode {
    Off,
    On,
    Stress,
}

pub(super) fn class_gc_mode() -> ClassGcMode {
    static MODE: std::sync::OnceLock<ClassGcMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("XEZIM_CLASS_GC").as_deref() {
        Ok("0") | Ok("off") => ClassGcMode::Off,
        Ok("stress") => ClassGcMode::Stress,
        _ => ClassGcMode::On,
    })
}

impl Simulator {
    /// Collect unreachable class objects once enough have been allocated
    /// since the last collection. Called between time slots.
    pub(super) fn maybe_collect_class_heap(&mut self) {
        let mode = class_gc_mode();
        if mode == ClassGcMode::Off {
            return;
        }
        let grown = self.heap.len().saturating_sub(self.class_gc_heap_len);
        // A collection costs time and memory in proportion to every slot
        // ever allocated, not to the live objects, so the next one waits for
        // half again as many allocations: the slots grow geometrically
        // between collections and the total cost stays linear. A fixed
        // threshold made a long run with few live objects quadratic.
        let threshold = match mode {
            ClassGcMode::Stress => 1,
            _ => CLASS_GC_MIN_ALLOCS
                .max(self.class_gc_live)
                .max(self.heap.len() / 2),
        };
        // A suspended DPI task keeps values on a foreign stack the trace
        // cannot see; wait until none is parked.
        if grown < threshold || !self.dpi_tasks.is_empty() {
            return;
        }
        self.class_gc_heap_len = self.heap.len();
        let Ok(marked) = self.class_heap_reachability_probe() else {
            return;
        };
        let dead: Vec<bool> = self
            .heap
            .iter()
            .zip(&marked)
            .enumerate()
            .map(|(h, (object, &live))| h != 0 && object.is_some() && !live)
            .collect();
        let released = self.release_unreachable_class_storage(&dead);
        self.class_gc_live = self.heap.iter().filter(|o| o.is_some()).count();
        self.class_gc_released += released;
        self.class_gc_runs += 1;
    }
}

impl Simulator {
    /// Trace scheduler-owned runtime roots without interpreting any expressions.
    /// The sweep runs only at a time advance with no suspended DPI task
    /// (`maybe_collect_class_heap`), whose foreign stack the trace cannot see.
    pub(super) fn trace_class_runtime_roots(&self) -> Result<HandleTrace, TraceError> {
        let mut trace = HandleTrace::new(self.heap.len());
        macro_rules! roots {
            ($($field:ident),* $(,)?) => { $(trace.trace(&self.$field)?;)* };
        }
        roots!(
            signal_table,
            forced_signals,
            static_fn_ret,
            active_force_exprs,
            partial_forces,
            prev_val,
            prev_wide,
            pending_strobes,
            pending_observed,
            clocking_snapshots,
            clocking_output_pending,
            clocking_cycle_pending,
            clocking_preponed,
            clocking_hist,
            deferred_clocking_conts,
            deferred_clocking_guarded,
            pending_reactive,
            reactive_conts,
            static_struct_holders,
            monitor,
            monitor_prev,
            monitor_arg_prev,
            delayed_updates,
            delayed_nba,
            delayed_nba_queue,
            packed_nba,
            this_stack,
            local_stack,
            block_scope_saves,
            return_value,
            queue_frame_saves,
            task_cleanup,
            ref_binding_stack,
            ref_alias_stack,
            local_dyn,
            formal_dyn,
            static_local_syncs,
            decl_shadow_log,
            static_local_vars,
            process_contexts,
            fork_baselines,
            fork_signal_captures,
            join_waiters,
            await_waiters,
            suspended_proc_info,
            vm_regs,
            vm_ret_vals,
            foreach_arena,
            ts_regs,
            ts_wregs,
            ts_wregs8,
            ts_save_list,
            nba_queue,
            pending_nba_instance_triggers,
            intra_saved,
            vc_saved,
            event_aliases,
            event_waiters,
            instance_event_waiters,
            triggered_conts_buf,
            condition_waiters,
            ready_condition_waiters,
            cond_waiter_conditions,
            inactive_queue,
            nba_region_waiters,
            fork_spawn_cont,
            exec_park_cont,
            slot_batches,
            deferred_comb,
            settle_prev_values,
            recv_call_memo,
            method_receiver_cache,
            suspended_loop_frames,
            process_loop_bytecode,
            process_cond_bytecode,
            forever_cont_cache,
            forever_sens_cache,
            blocking_cont_cache,
            class_task_frame_cache,
            compiled_fast_calls,
            compiled_fast_static_calls,
            pending_struct_coll_wb,
            pending_ret_struct,
            pending_ret_collection,
            vm_ret_coll,
            pending_formal_dyn,
            static_vif_bindings,
            local_iface_aliases,
            viface_var_aliases,
            frame_pool,
            queue_save_pool,
            ctor_class_stack,
            obj_rng_stack,
            factory_reg_in_progress,
            var_type_args,
            sampled_watches,
            sampled_defaults,
            proc_sampled_blocks,
            sva_lv,
            sva_proc_pending,
            sva_default_disable,
            sva_preponed,
            cg_heap,
            cg_named_event_waiters,
            cg_instance_event_waiters,
            cg_type_options
        );
        for (name, value) in self.signals.iter() {
            // An object's own storage is an edge FROM that object, not a root.
            if Self::class_storage_owner(name).is_none() {
                trace.trace(&(name, value))?;
            }
        }
        for entry in self.class_statics.iter() {
            trace.trace(&entry)?;
        }
        for list in self
            .event_queue
            .wheel
            .iter()
            .chain(self.event_queue.overflow.values())
        {
            trace.trace(list)?;
        }
        // A blocked operation keeps its mailbox/semaphore alive, including its
        // captured value, output lvalue and continuation. Queued values themselves
        // are edges from the owning mailbox, added during object traversal.
        roots!(
            mailbox_put_waiters,
            mailbox_get_waiters,
            semaphore_get_waiters
        );
        for waiter in self.mailbox_put_waiters.values().flatten() {
            if let Some(items) = self.mbx_pattern_items(&waiter.value) {
                trace.trace(items)?;
            }
        }
        for fsm in self.proc_fsm.values() {
            trace.trace(&fsm.regs)?;
            trace.trace(&fsm.compiled)?;
            trace.trace(&fsm.waits)?;
            if let Some(ts) = &fsm.ts {
                trace.trace(&ts.regs)?;
                trace.trace(&ts.snap)?;
            }
            #[cfg(feature = "jit")]
            trace.trace(&fsm.native_frame)?;
        }
        for entry in self.compiled_method_block_cache.values() {
            if let super::super::bytecode::CompiledMethodOutcome::Block(entry) = entry {
                trace.trace(&entry.block)?;
            }
        }
        for handle in self.pending_class_initializers.keys() {
            trace.candidate(*handle as u64);
        }
        for site in &self.sva_sites {
            trace.trace(&(
                &site.events,
                &site.node,
                &site.disable,
                &site.attempts,
                &site.past_snapshots,
                &site.pass_action,
                &site.fail_action,
                &site.sels,
                &site.lv_init,
                &site.captured,
                &site.matured,
            ))?;
        }
        for report in &self.deferred_asserts {
            trace.trace(&report.this_handle)?;
            trace.trace(&report.action)?;
        }
        Ok(trace)
    }

    pub(super) fn class_storage_owner(name: &str) -> Option<usize> {
        let (prefix, _) = name.split_once('#')?;
        if prefix.is_empty() || !prefix.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        prefix.parse().ok()
    }
}

impl Simulator {
    /// The objects reachable from the runtime roots, transitively: the
    /// marking `maybe_collect_class_heap` sweeps by.
    pub(super) fn class_heap_reachability_probe(&self) -> Result<Vec<bool>, TraceError> {
        let mut roots = self.trace_class_runtime_roots()?;
        let mut edges = vec![Vec::new(); self.heap.len()];
        let mut edge_trace = HandleTrace::new(self.heap.len());
        for (handle, object) in self.heap.iter().enumerate() {
            let Some(object) = object else {
                continue;
            };
            for (property, value) in object.properties.iter() {
                if !self.class_property_has_no_handles(handle, property) {
                    edge_trace.trace(value)?;
                }
            }
            if let Some(values) = self.mailboxes.get(&handle) {
                edge_trace.trace(values)?;
                for value in values {
                    if let Some(items) = self.mbx_pattern_items(value) {
                        edge_trace.trace(items)?;
                    }
                }
            }
            edges[handle].extend(edge_trace.drain_candidates());
        }
        for (name, value) in self.signals.iter().chain(self.class_assoc_defaults.iter()) {
            if let Some(owner) = Self::class_storage_owner(name).filter(|&h| h < edges.len()) {
                edge_trace.trace(name)?;
                let (_, property) = name.split_once('#').unwrap();
                if !self.class_property_has_no_handles(owner, property) {
                    edge_trace.trace(value)?;
                }
                edges[owner].extend(edge_trace.drain_candidates());
            } else {
                roots.trace(&(name, value))?;
            }
        }
        let mut marked = vec![false; self.heap.len()];
        let mut todo = roots.drain_candidates();
        while let Some(handle) = todo.pop() {
            if marked[handle] || self.heap[handle].is_none() {
                continue;
            }
            marked[handle] = true;
            todo.extend_from_slice(&edges[handle]);
        }
        Ok(marked)
    }

    fn class_property_has_no_handles(&self, handle: usize, property: &str) -> bool {
        let property = property.split(['[', '.']).next().unwrap_or(property);
        let Some(object) = self.heap.get(handle).and_then(Option::as_ref) else {
            return false;
        };
        let mut class = Some(object.class_name.as_str());
        for _ in 0..64 {
            let Some(cd) = class.and_then(|c| self.module.classes.get(c)) else {
                return false;
            };
            if let Some(data_type) = cd.property_types.get(property) {
                // Deliberately do not use the ambient/global typedef resolver:
                // T can be an object's class type parameter while a same-named
                // global typedef names int. Unresolved types retain candidates.
                return matches!(
                    data_type,
                    DataType::IntegerAtom { .. }
                        | DataType::IntegerVector { .. }
                        | DataType::Real { .. }
                        | DataType::Enum(_)
                        | DataType::Implicit { .. }
                        | DataType::Simple {
                            kind: crate::ast::types::SimpleType::String,
                            ..
                        }
                );
            }
            class = cd.extends.as_deref();
        }
        false
    }
}

impl Simulator {
    /// Sweep only slots whose unreachable status has already been established
    /// (`maybe_collect_class_heap`). Keep handle indices monotonic: stale
    /// numeric comparisons must never alias a subsequently allocated object.
    pub(super) fn release_unreachable_class_storage(&mut self, dead: &[bool]) -> usize {
        assert_eq!(dead.len(), self.heap.len());
        assert!(!dead.first().copied().unwrap_or(false));
        let dead_handle = |h: usize| dead.get(h).copied().unwrap_or(false);
        let dead_name = |name: &str| Self::class_storage_owner(name).is_some_and(&dead_handle);
        // Inherent remove maintains SignalMap's element index. HashMap::retain
        // through DerefMut would leave dangling names in that index.
        let names: Vec<_> = self
            .signals
            .keys()
            .filter(|n| dead_name(n))
            .cloned()
            .collect();
        for name in names {
            self.signals.remove(&name);
        }
        macro_rules! module_maps {
            ($($field:ident),* $(,)?) => { $(self.module.$field.retain(|name, _| !dead_name(name));)* };
        }
        macro_rules! module_sets {
            ($($field:ident),* $(,)?) => { $(self.module.$field.retain(|name| !dead_name(name));)* };
        }
        module_maps!(
            arrays,
            arrays_2d,
            arrays_nd,
            associative_arrays,
            assoc_elem_widths,
            assoc_index_widths,
            assoc_key_type_names,
            assoc_inner_string_keys,
            assoc_row_dims,
            assoc_defaults,
            queue_max_sizes,
            var_decl_types,
            packed_full_dims,
            packed_struct_fields,
            packed_signal_elem_widths,
            array_elem_class,
            unpacked_decl_dims,
            ascending_packed,
            struct_members,
            class_type_args
        );
        module_sets!(
            dynamic_arrays,
            descending_arrays,
            queue_vars,
            assoc_coll_elems,
            string_signals,
            two_state_signals
        );
        macro_rules! maps {
            ($($field:ident),* $(,)?) => { $(self.$field.retain(|name, _| !dead_name(name));)* };
        }
        macro_rules! sets {
            ($($field:ident),* $(,)?) => { $(self.$field.retain(|name| !dead_name(name));)* };
        }
        maps!(
            widths,
            var_class_types,
            var_typedef_types,
            var_container_types,
            var_type_args,
            class_assoc_defaults,
            dynamic_assoc_dims,
            local_coll_elems,
            static_struct_holders,
            rand_ranges,
            rand_elem_ranges
        );
        sets!(
            signed_signals,
            string_signals,
            real_signals,
            multi_dim_array_names,
            array_local_packed_dims,
            assoc_elem_colls,
            signed_class_colls,
            mixed_packed_vars,
            static_struct_names
        );
        macro_rules! handles {
            ($($field:ident),* $(,)?) => { $(self.$field.retain(|h, _| !dead_handle(*h));)* };
        }
        handles!(
            obj_rng,
            rand_mode_disabled,
            constraint_mode_disabled,
            mailboxes,
            mailbox_bound,
            semaphores,
            pending_class_initializers
        );
        // A discarded mailbox (or a killed producer) releases its captured
        // patterns. Queued messages and parked puts retain their own tokens.
        if !self.mbx_patterns.is_empty() {
            let tokens: HashSet<_> = self
                .mailboxes
                .values()
                .flatten()
                .chain(
                    self.mailbox_put_waiters
                        .values()
                        .flatten()
                        .map(|w| &w.value),
                )
                .filter(|value| value.width == 64 && value.is_fill)
                .filter_map(Value::to_u64)
                .collect();
            self.mbx_patterns.retain(|token, _| tokens.contains(token));
        }
        self.virtual_iface_bindings
            .retain(|(h, _), _| !dead_handle(*h));
        self.randc_used.retain(|(h, _), _| !dead_handle(*h));
        self.randc_pending.retain(|(h, _), _| !dead_handle(*h));
        self.dist_decks.retain(|(h, _), _| !dead_handle(*h));
        let mut released = 0;
        for (handle, object) in self.heap.iter_mut().enumerate() {
            if dead_handle(handle) && object.take().is_some() {
                released += 1;
            }
        }
        released
    }
}
