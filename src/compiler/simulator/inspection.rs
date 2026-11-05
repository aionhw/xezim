//! Simulator inspection.

use super::*;

impl Simulator {
    /// Work counters for performance REGRESSION testing.
    ///
    /// `(comb entry evaluations, bytecode instructions executed)`. Both are
    /// deterministic for a given design and run length, which is what makes
    /// them testable — wall-clock is not. A change that makes the simulator
    /// do more work to reach the same answer (a broken dirty-set, a
    /// re-introduced dead instruction, a lost fusion) moves these even when
    /// every functional test still passes.
    pub fn work_counters(&self) -> (u64, u64) {
        (self.entry_evals, self.prof_insns_executed)
    }

    /// Work counters for expression-backed procedural assignments:
    /// `(RHS evaluations, clean dependency checks skipped)`.
    pub fn active_force_work_counters(&self) -> (u64, u64) {
        (self.active_force_evals, self.active_force_skips)
    }

    /// Deterministic aggregate work: (specialized declarations resolved,
    /// root packed layouts built). Repeated member accesses reuse both.
    pub fn aggregate_type_work_counters(&self) -> (u64, u64) {
        (self.aggregate_type_work[0].get(), self.aggregate_type_work[1].get())
    }

    pub fn get_signal(&self, name: &str) -> Option<&Value> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(&self.signal_table[id]);
        }
        self.signals.get(name)
    }

    // ---- assertion + coverage accessors (for regression tests + tools) ----

    /// Number of distinct source positions that fired an immediate
    /// `assert` / `assume` / `cover` at least once.
    pub fn assertion_site_count(&self) -> usize {
        self.assertion_stats.len()
    }
    /// Total pass count across all assertion sites.
    pub fn assertion_pass_total(&self) -> u64 {
        self.assertion_stats.values().map(|s| s.pass_count).sum()
    }
    /// Total fail count across all assertion sites.
    pub fn assertion_fail_total(&self) -> u64 {
        self.assertion_stats.values().map(|s| s.fail_count).sum()
    }
    /// Number of allocated covergroup instances (index 0 is the null
    /// sentinel; skipped here).
    pub fn covergroup_instance_count(&self) -> usize {
        self.cg_heap.iter().skip(1).filter(|x| x.is_some()).count()
    }
    /// Total `sample()` invocations across every covergroup instance.
    pub fn covergroup_sample_total(&self) -> u64 {
        self.cg_heap.iter().flatten().map(|i| i.sample_count).sum()
    }
    /// Hit cardinality for a named coverpoint on the first instance whose
    /// covergroup matches `cg_name`. Returns 0 if none found.
    pub fn coverpoint_hits(&self, cg_name: &str, cp_name: &str) -> usize {
        for inst in self.cg_heap.iter().flatten() {
            if inst.cg_name == cg_name {
                if let Some(set) = inst.point_hits.get(cp_name) {
                    return set.len();
                }
            }
        }
        0
    }
    /// Cross-tuple cardinality for a named cross on the first matching
    /// covergroup instance.
    pub fn cross_hits(&self, cg_name: &str, cr_name: &str) -> usize {
        for inst in self.cg_heap.iter().flatten() {
            if inst.cg_name == cg_name {
                if let Some(set) = inst.cross_hits.get(cr_name) {
                    return set.len();
                }
            }
        }
        0
    }
    /// LRM §19.6: hit count for a specific cross body bin filter.
    /// `key` is `<cross_name>.<bin_name>`.
    pub fn cross_bin_hits(&self, cg_name: &str, key: &str) -> u64 {
        for inst in self.cg_heap.iter().flatten() {
            if inst.cg_name == cg_name {
                if let Some(&n) = inst.cross_bin_hits.get(key) {
                    return n;
                }
            }
        }
        0
    }
    /// Hit count of the coverpoint bin `key` (`cp.bin`, `cp.bin[v]`) on the
    /// first instance of covergroup `cg_name` that recorded it.
    pub fn coverpoint_bin_hits(&self, cg_name: &str, key: &str) -> u64 {
        self.cg_heap
            .iter()
            .flatten()
            .filter(|i| i.cg_name == cg_name)
            .find_map(|i| i.bin_hits.get(key).copied())
            .unwrap_or(0)
    }
    pub fn set_signal(&mut self, name: &str, val: Value) {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let w = self.signal_widths[id];
            write_sig!(self, id, val.resize(w));
            self.table_modified = true;
            self.mark_dirty_id(id);
            return;
        }
        if let Some(w) = self.widths.get(name) {
            self.signals.insert(name.to_string(), val.resize(*w));
        } else {
            self.widths.insert(name.to_string(), val.width);
            self.signals.insert(name.to_string(), val);
        }
    }
}
