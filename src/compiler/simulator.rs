//! Event-driven simulator for SystemVerilog combinatorial and sequential logic.
//!
//! Implements a simplified IEEE 1800 scheduling model:
//!   Active region:  blocking assigns, continuous assigns, always_comb
//!   NBA region:     non-blocking assign updates
//!   Reactive:       edge-triggered always_ff/always_latch blocks
//!
//! The simulator state and shared types live here. Execution methods are
//! grouped by responsibility in the `simulator/` implementation modules.

// Compiler modules shared by the implementation submodules.
#[cfg(feature = "jit")]
use super::{aot, ts_jit};
use super::{
    bytecode, dispatch, elaborate, fst_sink, interrupt, jit, sdf, stdout_sink, value, vcd_sink,
};

use super::elaborate::{
    AlwaysBlock, CollShape, Dim, DpiImportSpec, ElaboratedModule, KeyKind, Signal,
};
use super::packed_assignment_pattern_plan::{has_pattern, is_packed, type_name_expr};
use super::value::{LogicBit, Value};
use crate::ast::decl::{
    AlwaysKind, ClassConstraint, ConstraintItem, ConstraintRange, CovergroupDeclaration,
    CovergroupItem, FunctionDeclaration, LetDeclaration, TaskDeclaration,
};
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::types::{DataType, IntegerAtomType, PortDirection};
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

use super::interrupt::{
    INTERRUPT_SIGNAL, acknowledge_interrupt, interrupt_requested, loop_stop_requested,
    publish_sim_time,
};
use fst_writer::{
    FstHeaderWriter, FstScopeType, FstSignalId, FstSignalType, FstVarDirection, FstVarType,
};
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Library;
use rand::SeedableRng;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CStr, CString};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::raw::c_void;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use xezim_core::elaborate::resolve_type_width;
use xezim_core::hasher::{HashMap, HashSet};
use xezim_core::value::{decode_name_key, encode_name_key};

/// First activations of a cached process-`if` condition also run the AST
/// evaluator and compare (see `try_eval_process_cond_bytecode`). Small: each
/// audit costs one redundant AST evaluation; divergence pins the span to the
/// AST forever, so the window only needs to catch shape-level miscompiles,
/// which show on the first data that exercises them.
const COND_AUDIT_EVALS: u8 = 4;

/// Opaque handle base for `process::self()` (IEEE 1800-2023 §9.7). The token is
/// `PROCESS_HANDLE_BASE + pid`, chosen far above any real heap index so a
/// process handle can never be mistaken for a class-object handle.
const PROCESS_HANDLE_BASE: u64 = 0x7000_0000;

/// Interpreted loops poll the Ctrl-C flag once every this-many-plus-one
/// iterations: a long loop inside one time slot never reaches the event
/// loop's slot-boundary poll, and the backstop would otherwise kill it with
/// the dumps unfinalized.
const LOOP_INTERRUPT_POLL_MASK: u64 = 0x3ff;

/// Serde for fixed arrays longer than serde's built-in 32 elements, as a
/// sequence, for checkpoints.
mod serde_big_array {
    use serde::de::{Deserialize, Deserializer, Error};
    use serde::ser::{Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize, const N: usize>(
        a: &[T; N],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        a.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
        d: D,
    ) -> Result<[T; N], D::Error> {
        let v = Vec::<T>::deserialize(d)?;
        let n = v.len();
        v.try_into()
            .map_err(|_| D::Error::invalid_length(n, &"a fixed-length array"))
    }
}

thread_local! {}

/// Compiler salt for the persistent compiled-method cache (XEZIM_METHOD_CACHE).
/// Bump on ANY change that can alter the compiled instructions or the
/// admit/decline decision for the same source text — admission inputs,
/// lowering, register allocation, block layout. Stale entries with an old
/// salt hash to different keys and are simply recompiled; the safety net is
/// the suite-level ON == OFF parity run against a warm cache.
// §18.8 (#846): mode builtins retain their receiver expressions in compiled code.
// §11.4.10, §11.4.3: shift, `/`, `%` and `**` operands of unknown width keep their loaded width.
// §8.25: compiled methods take type-parameter formals and locals.
// §7.5/§7.10: design-scope dynamic-array and queue elements read the live collection.
// §11.6.1: member element selects size at the declared element width
// (`coll_elem_widths` plan fact in `expr_max_width`).
// §11.5.1, §7.4.1: an x/z index into a struct member keeps the element width,
// and a chain of indices into a call result's member declines.
// §7.4.1/§7.4.5: a packed select inside an unpacked array's element reads that element.
// §11.5.1: an x/z literal index into a packed element reads x.
// §7.4.1: a run-time index into an unpacked array of three or more dimensions declines.
// §7.4.1: `a[i].m[j][k]` selects elements of a multi-dimensional packed member.
// §7.4.1: local packed typedefs and selected struct-array members keep their dimensions.
// §13.4: an inlined function body does not see the caller's unrolled loop constants.
// §6.21: a subroutine whose static variable carries between calls is not inlined.
// §13.5.3: function copy-out resolves caller locals and indexed targets after return.
// §13.4.1: returns inside nested inline calls exit only the innermost callee.
// §13.5.1 / §8.25: packed method arguments and results retain specialized widths.
// §8.8: typed constructors compile the named class scope.
// §6.21: addressed static locals retain shared storage.
// §25.5: a modport-qualified name resolves to the interface instance's own variable.
// §8.14: a base static that a derived class redeclares counts as a redeclared member.
// §7.4.1: a member of an element of an ascending packed array of structs slices from slot 0.
// §10.9.2: structure replication in scalarized function calls.
// §6.16: a string-collection element read through a class handle compares as a string.
// §7.4.2: an index into an array with a negative bound is sign-extended.
// §10.9.1, §11.8.1, §7.2.1, §7.4.1, §11.4.14: typed pattern keys, operand signs, packed elements, stream widths.
// §23.3.2, §7.4: an element of a multidimensional array member of an instance-array element.
// §6.19.5: enumeration methods take the receiver's declared enumeration.
// §6.20: procedural parameters retain their own type and declaration kind.
// Scope-specific combinational dependency resolution (§23.6).
// Absolute packed references retain declaration geometry (§23.6, §7.4.1).
// Class aggregate ownership and struct-field constructor typing (§8.8/§8.11).
// Scoped memory destinations, dynamic rows, combinational time scope, and string-member local shadowing.
// Explicit event expressions retain function-call operands and result guards.
// Net typedef metadata and virtual-interface specialization identities.
// Class-local shadowing keeps numeric comparisons out of string-member admission.
// §7.4.1, §6.20: elements of packed-array members of struct formals and nested
// callees reading module parameters inline.
// §13.5, §11.6.1: compiled import calls and calls handed to the interpreter size
// integral input actuals by their formals; a method with a vif local declines (§25.9).
// §6.18, §6.16, §13.5.1, §8.9: typedef write-back receivers, string-collection members, unpacked-formal actuals, static collections.
// §5.7.1, §11.8.2, §11.4.13, §12.5.4: fill operands of real operators fold at one bit; real,
// fill and negative-member `inside` and non-constant `case inside` items run interpreted.
// §8.9, §23.9, §6.18, §8.25: bare stores to class-scope shadow names decline; class typedef chains size formals.
// §5.7.1, §7.4.1: unsized signed decimal widths with a sign bit; multidimensional packed range NBAs decline.
// §26.3: package code reads its own package's parameters, variables and imports, and an
// inlined package callee its own, ahead of the design's same-named declarations.
// §13.5.2: a call that declines its output or inout write-back leaves no compiled call
// behind its interpreter fallback.
// §6.21, §13.4.2, §7.4.1: a static local that constant selects write whole before a read
// carries nothing between calls, so its subroutine inlines; a subroutine selecting a variable
// declared with an ascending or offset range does not inline.
// §9.3, §13.3: block bodies in shared statement lists, a continuation tree per task call
// site, and boxed optional delays, initializers and case item patterns (#721).
// §12.5: an unsigned item makes the entire case comparison unsigned.
// §10.9.1, §26.3: a named assignment pattern key carries its lexically bound type name.
// §6.18, §7.4.1: a cast to a typedef of real and a function returning one are real operands,
// and a subroutine or block selecting a packed array of a named signed type does not compile.
// §7.10.2, §12.7.1: a for loop storing a queue pop into a variable or array element compiles.
// §11.8.2: preserve integral arithmetic in constant operands of real comparisons.
// §6.24.1: a signing conversion consumes an unsized fill literal.
// §20.8.1, §7.4.1: `$clog2` of a constant folds; unfolded select bounds map labels.
// §20.8.1, §11.6.1: a folded `$clog2` is signed, and folds only a literal or parameter.
// §7.4.1: unfolded select bounds on a packed multidimensional base run interpreted.
// §6.21, §13.5.3: output-before-read locals and literal default actuals compile.
// §6.12.1, §6.24.1, §12.5: `RealToInt` rounds a value that may be real fitted to 64 integral
// bits; a value stored into a real target converts first; a real `case` runs interpreted.
// §20.3, §3.14.2.3: a scoped `$realtime` read is a real that is not a constant, and a
// scoped `$time` read is 64 bits wide. §6.24.1, §7.4.3, §11.5.1: a struct-parameter field
// sizes its cast, and a part-select after a packed index selects whole elements, clipped to
// its dimension.
// Packed word selects inherit the declared dimensions of their bank.
// §7.4.5: member collection reads that yield unpacked rows use their row storage.
// §6.24.1: an integer atom cast of an integral operand copies the operand before it is fitted.
// §7.2, §7.4.2: a member select through constant element selects of a packed struct compiles,
// and a nested packed-struct member read compiles to one range select.
// §7.5, §7.10: compiled local collection reads retain element width and sign.
// §7.4.5: packed element constant indices keep their declared sign.
// §7.10.2: module queue pops call the collection runtime directly.
const METHOD_CACHE_COMPILER_SALT: u64 = 0x6370_6666_317d;

/// Depth at which `run_process_stmts` loop handlers switch from recursing to
/// trampolining via the event queue. ~2.9KB/frame overflows an 8MB stack near
/// ~2800; 300 leaves a wide margin while keeping normal (shallow) loops on the
/// fast recursive path.
const RPS_TRAMPOLINE_DEPTH: usize = 300;

/// Internal marker retrieving a §9.4.5 intra-assignment RHS value captured in
/// `Simulator::intra_saved` (see `make_intra_saved_expr`). Never user-visible.
const INTRA_SAVED_MARKER: &str = "$__xz_intra_saved";
/// §9.4.5, §10.9: an item of an intra-assignment pattern, captured when the
/// statement ran. Read without being taken (a `default:` item is read once
/// per element); `INTRA_RELEASE_MARKER` frees the items after the store.
const INTRA_KEPT_MARKER: &str = "$__xz_intra_kept";
const INTRA_RELEASE_MARKER: &str = "$__xz_intra_release";
/// Internal `$__xz_vc_changed(idx, expr)`: true once `expr` differs from the
/// value stashed in `Simulator::vc_saved[idx]` when a value-change event
/// control (`@prop` on a non-event class property) was armed. Never
/// user-visible.
const VC_CHANGED_MARKER: &str = "$__xz_vc_changed";
/// Internal `$__xz_vc_edge(idx, expr, edge[, iff])`: true once `expr` has made the
/// requested TRANSITION relative to `Simulator::vc_saved[idx]`. Used by
/// `@(posedge prop)` / `@(negedge prop)` / `@(edge prop)` when `prop` is a
/// class property or a procedural local — an edge-qualified event term has no
/// backing signal, so it is armed as an edge-sensitive value-change wait
/// (§9.4.2). `edge` is 1 = posedge, 2 = negedge, 3 = any edge, 0 = any
/// change (an untagged term of an `or` list); an `iff` guard must hold when
/// it fires. Never user-visible.
const VC_EDGE_MARKER: &str = "$__xz_vc_edge";
/// Internal `$__xz_vif_value(expr)`: the identity of the interface instance
/// the virtual-interface operand `expr` holds (the `iface_handle_token` of the
/// bound instance, 0 for null). `@(vif)` / `@(vifs[i])` waits for this value
/// to change (§9.4.2, §25.9). Never user-visible.
const VIF_VALUE_MARKER: &str = "$__xz_vif_value";
/// Ends a deferred declaration initializer: `(<target>, ...)` records the
/// values it gave as their default sampled values (see `sampled_defaults`).
const SAMPLED_DEFAULT_MARKER: &str = "$__xz_sampled_default";
/// A signal of a concurrent assertion resolved at registration: `(<id>)`
/// reads `signal_table[id]` (see `sva_resolve_signals`).
const SVA_SIG_MARKER: &str = "$__xz_sva_sig";
/// §16.3: the message of the `$error` a failed assertion with no `else`
/// clause reports (the reference simulator's wording).
const ASSERT_DEFAULT_ERROR: &str = "Assertion error.";
/// Internal task a waiter resumes to report that a wait made from the
/// nested (C-called) path is over; see `run_nested_until`.
const SYNC_WAKE_MARKER: &str = "$__xz_sync_wake";

/// §21.2.1.7 `%m` bookkeeping: an `m_scope_stack` entry starting with
/// `M_ROOT_MARK` carries the absolute declaring path of the running
/// subroutine (`pk`, `tb.u`); one starting with `M_SUBR_MARK` is a subroutine
/// named from the caller's instance. Plain entries are named blocks.
pub(crate) const M_ROOT_MARK: char = '\u{1}';
const M_SUBR_MARK: char = '\u{2}';

macro_rules! env_set_cached {
    ($name:literal) => {{
        static FLAG: OnceLock<bool> = OnceLock::new();
        cached_env_set($name, &FLAG)
    }};
}

macro_rules! env_is_one_cached {
    ($name:literal) => {{
        static FLAG: OnceLock<bool> = OnceLock::new();
        cached_env_flag($name, &FLAG)
    }};
}

macro_rules! sim_dbg_eprintln {
    ($($arg:tt)*) => {
        if sim_debug_enabled() {
            eprintln!($($arg)*);
        }
    };
}

/// Write `signal_table[id]` and update the parallel `signal_has_xz`
/// byte that the JIT inline X/Z prelude reads. Must be used in place
/// of bare `write_sig!(self, id, val);` so the JIT bitmap stays
/// consistent. Both fields are simple `Vec`s on `Self`, so the macro
/// expands to a single pair of indexed writes — the borrow checker
/// sees disjoint field borrows just like the bare assignment did.
/// Mark a signal id dirty for the next incremental VCD flush. A macro (not a
/// method) so it expands to disjoint FIELD accesses — a `&mut self` method call
/// would conflict with call sites that already hold a mutable borrow of another
/// field (e.g. `for cg in &mut self.clock_generators`). Cheap: one bounds check
/// + one epoch compare when the dump is active, a single branch otherwise. The
/// rolling epoch dedups repeat writes within a flush.
macro_rules! vcd_mark {
    ($self:ident, $id:expr_2021) => {{
        if $self.dump_dirty_active {
            let __vm_id = $id;
            if __vm_id < $self.dump_dirty_mark.len()
                && $self.dump_dirty_mark[__vm_id] != $self.dump_dirty_epoch
            {
                $self.dump_dirty_mark[__vm_id] = $self.dump_dirty_epoch;
                $self.dump_dirty.push(__vm_id as u32);
            }
        }
    }};
}

/// Arm the clocked readers of a stored signal. The change mask is evaluated
/// only for a signal with a bit-selective fanout. Keep the field borrows
/// disjoint for writes made inside loops over other simulator fields.
macro_rules! arm_edge_readers {
    ($self:ident, $id:expr_2021, $change:expr_2021) => {{
        let arm_id = $id;
        if $self.armed_edge
            && (arm_id >> 6) < $self.armed_input_bitmap.len()
            && ($self.armed_input_bitmap[arm_id >> 6] >> (arm_id & 63)) & 1 != 0
        {
            let (lo, hi, mask_base) = $self.armed_input_ranges[arm_id];
            if mask_base == u32::MAX {
                for k in lo as usize..hi as usize {
                    let bi = $self.armed_input_blocks[k] as usize;
                    $self.edge_block_armed[bi] |= EDGE_ARMED;
                }
            } else {
                let changed = $change;
                let blocks = &$self.armed_input_blocks[lo as usize..hi as usize];
                let base = mask_base as usize;
                $crate::compiler::simulator::Simulator::arm_masked_edge_readers(
                    &mut $self.edge_block_armed,
                    blocks,
                    &$self.armed_input_masks[base..base + blocks.len()],
                    changed,
                );
            }
        }
    }};
}

/// The bookkeeping of a completed store into `signal_table[id]` (`changed`:
/// whether the stored value differs from the one it replaced). Clocked
/// readers are armed separately so a whole-value write can use the old value.
macro_rules! write_sig_done {
    ($self:ident, $id:expr_2021, $changed:expr_2021) => {{
        let __wsig_id = $id;
        let __wsig_changed = $changed;
        // Expression-backed procedural assignments use per-signal
        // generations as their sensitivity set. Keep the usual no-active-
        // assignment path to one branch; only real value changes advance
        // the generation.
        if __wsig_changed
            && !$self.active_force_exprs.is_empty()
            && __wsig_id < $self.active_force_signal_epochs.len()
        {
            $self.active_force_signal_epochs[__wsig_id] =
                $self.active_force_signal_epochs[__wsig_id].wrapping_add(1);
            $self.force_epoch_gen = $self.force_epoch_gen.wrapping_add(1);
        }
        // Incremental VCD: mark this write dirty (no-op when the dump is off
        // or the full scan is forced). Superset-safe — the flush re-checks
        // prev vs cur, so marking an unchanged write costs only a skip.
        vcd_mark!($self, __wsig_id);
        // O1 measurement: stamp the signal's last-change phase.
        if $self.event_measure && !$self.armed_edge && __wsig_id < $self.sig_last_change.len() {
            $self.sig_last_change[__wsig_id] = $self.event_phase;
        }
        // Dirty-driven edge detect: record edge-sensitive writes (no-op unless
        // XEZIM_DIRTY_EDGE/_SHADOW). Inlined here rather than a method call so
        // the borrow stays a direct field access in macro-expanded contexts.
        if ($self.dirty_edge || $self.dirty_edge_shadow)
            && (__wsig_id as usize) < $self.sig_to_edge_pos.len()
        {
            let __ep = $self.sig_to_edge_pos[__wsig_id as usize];
            if __ep >= 0 && !$self.edge_pos_seen[__ep as usize] {
                $self.edge_pos_seen[__ep as usize] = true;
                $self.changed_edge_pos.push(__ep as usize);
            }
        }
        // Parked-waiter index: queue a watched signal that changed, for
        // the next waiter drain and the next snapshot. One length
        // compare while no waiter is indexed.
        if __wsig_changed
            && __wsig_id < $self.event_waiters.watch.len()
            && $self.event_waiters.watch[__wsig_id] & event_waiters::WATCH_ANY != 0
        {
            $self.event_waiters.note_write(__wsig_id);
        }
        // §9.2 re-trigger: a blocking write to an edge-sensitive signal
        // made WHILE an edge block executes must re-run edge detection
        // for that signal (drain_edge_exec_rescan) — record its position.
        // One bool load when not inside an edge block.
        if ($self.in_edge_block || $self.in_edge_cont)
            && (__wsig_id as usize) < $self.sig_to_edge_pos.len()
        {
            let __ep = $self.sig_to_edge_pos[__wsig_id as usize];
            if __ep >= 0 && !$self.edge_exec_seen[__ep as usize] {
                $self.edge_exec_seen[__ep as usize] = true;
                $self.edge_exec_wrote.push(__ep as usize);
            }
        }
        // §38.36.1 cbValueChange: report the change right away, in the
        // region of the write. One emptiness check while nothing is
        // watched. Writes that bypass this macro are caught by the poll
        // at the end of each settle and delta (`vpi_cb::vpi_checkpoint`).
        // The dispatcher takes the raw self-pointer: this macro expands
        // under live borrows of other fields.
        if !$self.dpi_value_change_cbs.is_empty()
            && $self.dpi_value_change_cbs.contains_key(&__wsig_id)
        {
            vpi_cb::vpi_vc_written($self.vpi_self_ptr, __wsig_id);
        }
    }};
}

macro_rules! write_sig {
    ($self:ident, $id:expr_2021, $val:expr_2021) => {{
        let __wsig_id = $id;
        let mut __wsig_val = $val;
        // Invariant: a table slot's stored signedness is the signal's DECLARED
        // signedness (`signal_signed[id]`), never the rvalue's. An unsized
        // decimal literal is signed (§5.7.1), so without this `barr[i] = 9`
        // stamped is_signed onto a `bit [3:0]` element and every later read
        // sign-extended it (-7). Real values are exempt (is_signed unused) and
        // strings never reach a sized table slot with it consulted.
        if !__wsig_val.is_real && __wsig_id < $self.signal_signed.len() {
            __wsig_val.is_signed = $self.signal_signed[__wsig_id];
        }
        // §6.12/§20.5: a SHORTREAL holds an IEEE binary32 value, so a write
        // rounds to binary32 (16777216.25 -> 16777216.0). A `real` (binary64)
        // keeps every bit. Only a real write checks destination precision.
        if __wsig_val.is_real && $self.shortreal_signals.contains(&__wsig_id) {
            let __wsig_f = __wsig_val.to_f64() as f32 as f64;
            __wsig_val = Value::from_f64(__wsig_f);
            __wsig_val.is_signed = $self.signal_signed[__wsig_id];
        }
        // LRM §9.3.1: skip writes to forced signals. The `is_empty` guard
        // keeps the common no-force case to one branch — `contains_key`
        // hashes the id even when the map is empty, and this macro runs on
        // EVERY signal write (measured ~2% of c906 memcpy cycles).
        if $self.forced_signals.is_empty() || !$self.forced_signals.contains_key(&__wsig_id) {
            let __wsig_changed = $self.signal_table[__wsig_id] != __wsig_val;
            if __wsig_id < $self.signal_has_xz.len() {
                $self.signal_has_xz[__wsig_id] = if __wsig_val.raw_bits().1 != 0 {
                    1u8
                } else {
                    0u8
                };
            }
            // JIT-redesign Stage 1 maintenance: keep signal_inline_bits in
            // sync with signal_table for the future JIT codegen path that
            // reads values by direct pointer offset.  Zero overhead when
            // signal_inline_bits is empty (default; allocated only when
            // XEZIM_INLINE_BITS=1).
            if __wsig_id < $self.signal_inline_bits.len() {
                let (__wsig_v, __wsig_x) = __wsig_val.raw_bits();
                $self.signal_inline_bits[__wsig_id] = [__wsig_v, __wsig_x];
            }
            // `--warn-x`: record a signal taking a NEW x bit after time 0.
            // One bool load when disabled. Placed here because the old value is
            // still in the table, so "became x" is distinguishable from "was
            // already x". Written inline rather than as a call: this macro
            // expands inside bodies that already hold a `&mut` borrow of another
            // field, so only disjoint field access is possible here.
            if $self.warn_x && $self.time > 0 {
                let (__xov, __xox) = $self.signal_table[__wsig_id].raw_bits();
                let (__xnv, __xnx) = __wsig_val.raw_bits();
                let _ = __xov;
                // x is (xz & !val). Report only a VALID 0/1 bit (old xz clear)
                // going x: a bit that has been x or z all along — an undriven
                // input, an uninitialized reg first materializing — is not a
                // valid-value corruption and stays silent.
                let __fresh = (__xnx & !__xnv) & !__xox;
                let __wide_new_x = __wsig_val.width > 64
                    && __wsig_val.has_xz()
                    && !$self.signal_table[__wsig_id].has_xz();
                if (__fresh != 0 || __wide_new_x)
                    && $self.warn_x_seen.borrow_mut().insert(__wsig_id)
                {
                    let __xt = $self.time;
                    let __xp = $self.current_pid;
                    $self.warn_x_pending.borrow_mut().push((
                        __wsig_id,
                        __xt,
                        __wsig_val.clone(),
                        __xp,
                        None,
                    ));
                }
            }
            // XEZIM_VALUE_TRACE: queue a changed watched write (old value is
            // still in the table here). Disjoint field accesses only — this
            // macro expands under live &mut borrows of other fields.
            if let Some(__vt_ids) = &$self.value_trace_ids {
                if __vt_ids.contains(&__wsig_id) && $self.signal_table[__wsig_id] != __wsig_val {
                    $self.value_trace_pending.borrow_mut().push((
                        __wsig_id,
                        $self.time,
                        $self.signal_table[__wsig_id].clone(),
                        __wsig_val.clone(),
                        $self.current_pid,
                        $self.edge_dispatch_phase,
                    ));
                }
            }
            arm_edge_readers!(
                $self,
                __wsig_id,
                $crate::compiler::simulator::Simulator::edge_input_delta(
                    &$self.signal_table[__wsig_id],
                    &__wsig_val,
                )
            );
            $self.signal_table[__wsig_id] = __wsig_val;
            write_sig_done!($self, __wsig_id, __wsig_changed);
        }
    }};
}

// §9.2.2.2.1 (#844): rebuild dependency tables made before packed-region
// sensitivity exclusion; their layout is readable but their edges are stale.
// §9.7 (#940): `Insn::LoadProcessStatus` renumbers the instructions after it.
// §9.2.2.2: whole-struct reads of a collapsed continuous copy depended on
// member ids that the collapse left unused; those edges are stale. Reads of
// an element of an unpacked array of structs, or of the whole array, now
// depend on its members too.
// §10.3.3: keep falling and turn-off delays when prepared tables are restored.
// §10.3.3 / §28.16: retain each delayed entry's original transition target.
// §10.3.3 / §28.16: and the fall and turn-off delays of a select target.
// §7.4.1: compiled members of ascending packed arrays of structs slice from slot 0.
// §10.9.2: scalarized resolver calls expand structure replication.
// §7.4.2: compiled blocks sign-extend an index into an array with a negative bound.
// §10.3.2, §13.4, §23.6: a called function's name and frame names are not
// unresolved reads; whole arrays through an instance-array element, named and
// top-spelled struct-array actuals depend on their elements and members.
// §10.9.1, §11.8.1, §7.2.1, §7.4.1, §11.4.14: typed pattern keys, operand signs, packed elements, stream widths.
// §23.3.2, §7.4: elements of a multidimensional array member of an instance-array
// element are read and written by the interpreter and depend on their own signals.
// §7.4.1, §6.20: elements of packed-array members of struct formals and nested
// callees reading module parameters inline.
// §12.5.4, §11.4.13: `case inside` with a wildcard, range or real item and `inside` with a
// real operand, a fill or a negative member run on the interpreter's matcher.
// §5.7.1, §7.4.1: unsized signed decimal widths with a sign bit; multidimensional
// packed range nonblocking writes are left to the interpreter.
// §10.3.2, §9.2.2.1: shared-read batches leave out members with fused branches or jump
// tables, chain fusion relocates branches within one process class, and an instance's
// read of an unnamed memory cell stays unresolved.
// §13.4.1, §26.3: a scope-qualified function's return variable and the name of a callee
// called through a package or `$unit` are not reads; its body's reads are.
// §6.21, §13.4.2, §7.4.1: continuous assigns calling a static function whose local constant
// selects write whole before a read compile; a call selecting an ascending or offset-range
// variable of its callee stays interpreted.
// §10.9.1, §26.3: a named assignment pattern key carries its lexically bound type name.
// §6.18, §6.24.1: a cast to a typedef of real is a real operand of a continuous assignment.
// §9.2.2.2, §10.3: CombEntryCold::time0_effects; a variable a called function writes is
// not a read of its caller.
// §7.10.2, §12.7.1: a for loop storing a queue pop into a variable or array element compiles.
// §9.2.2.2.1: parameterized instance vectors retain disjoint sensitivity regions.
// §6.24.1: compiled sign conversions use the self-determined operand width.
// §10.9.1: packed array default drivers retain their element type.
// §20.8.1, §7.4.1: `$clog2` of a constant folds, and a select bound that does
// not fold still maps its label to a physical bit.
// §20.8.1, §11.6.1: a folded `$clog2` is signed, and folds only a literal or parameter.
// §7.4.1: unfolded select bounds on a packed multidimensional base run interpreted.
// §9.4.2.2: an `@*` block that reads a variable it writes with `<=` runs as a
// process, not as a combinational entry.
// §6.12.1, §6.24.1, §12.5: `RealToInt` (a new opcode) rounds a value that may be real fitted
// to 64 integral bits, continuous assignments see which signals are real, a value stored into
// a real target converts first, and a real `case` runs interpreted.
// §10.3, §10.10: a continuous assignment of an unpacked concatenation to a fixed array
// drives each element. §3.14.2.3: scoped time reads of continuous assignments are typed
// by their result. §6.11.2: a gate-shaped assignment to a 2-state scalar is no fused gate.
// §6.24.1, §7.4.3, §9.4.2: struct-parameter field casts, part-selects of whole elements after a
// packed index, and alias nets for event controls on packed-struct members.
// Packed word selects inherit the declared dimensions of their bank.
// §9.4.2: a selected member can require a value waiter instead of a comb block.
// §6.24.1, §9.4.2: integer atom casts keep their operand, and a packed-struct member event
// tries every scope spelling for its alias net.
// §7.2, §7.4.2: a member select through constant element selects of a packed struct, and a
// nested packed-struct member read, compile to range stores and selects.
// §7.4.5: signed constant labels of packed array elements.
const PREPARED_COMB_MAGIC: &[u8; 8] = b"XZCMB081";

mod aggregate_assignments;
mod pattern_array_items;
mod intra_pattern_shape;
mod annex_n;
mod aggregate_expressions;
mod aggregate_members;
mod arguments;
mod assertions;
mod sva_until;
mod assignments;
mod bytecode_runtime;
mod calls;
mod checkpoint;
mod checkpoint_calls;
mod class_access;
mod class_members;
mod class_methods;
mod class_specialization;
mod class_array_constants;
mod property_initialization;
mod clocking;
mod collection_methods;
mod collections;
mod conditional_aggregates;
mod comb_compilation;
mod compilation;
mod compiled_methods;
mod constraint_domains;
mod constraint_solver;
mod constraint_targets;
mod construction;
mod coverage;
mod coverage_implicit;
mod dpi_imports;
mod edges;
mod enum_receivers;
mod event_waiters;
mod event_loop;
mod stall_progress;
mod expression_types;
mod expressions;
mod file_io;
mod formatting;
mod functions;
mod initialization;
mod inline_constraints;
mod inspection;
mod isolated_gates;
mod jit_runtime;
mod mailboxes;
mod method_types;
mod named_events;
mod packed_assignment_patterns;
mod process_control;
mod process_scopes;
mod processes;
mod profiling;
mod sensitivity;
mod settling;
mod setup;
mod signal_commits;
mod signal_width_table;
pub use signal_width_table::SignalWidths;
mod statement_assignments;
mod statements;
mod system_tasks;
mod time0_effects;
mod two_state;
mod type_metadata;
mod udp;
mod unpacked_slices;
mod value_display;
mod virtual_interfaces;
mod waveforms;
mod assert_ctl;
mod code_cov;
mod dpi_export;
mod dpi_task;
mod event_bits;
mod event_stamps;
mod module_paths;
mod names;
mod rand_csp;
mod ref_aliases;
mod rand_diag;

/// Marks a `dist` target recorded by MEMBER path in `rand_order_sensitive`.
const MEMBER_DIST_MARK: &str = "\u{1}member:";
mod rand_scope;
mod sv_file;
mod task_suspend;
mod timing_checks;
mod ts_x;
mod typename;
mod uvm_dpi;
mod vpi_api;
mod vpi_value;
use vpi_value::fill_vpi_value;
pub use vpi_value::{vpi_get_value, vpi_put_value};
mod vpi_assert;
mod vpi_cb;
mod vpi_model;
use assert_ctl::{AssertCtl, CtlIdent, CtlSite};
pub use checkpoint::{
    set_restore as set_checkpoint_restore, set_save_at as set_checkpoint_save_at,
};
pub(crate) use code_cov::{
    COND_FN as COV_COND_FN, HIT_TASK as COV_HIT_TASK, marker_id as cov_marker_id,
};
pub use code_cov::{
    CodeCoverage, KIND_BRANCH, KIND_STATEMENT, KIND_TOGGLE,
    parse_kinds as parse_code_coverage_kinds, set_code_coverage,
};
use event_stamps::{EventId, EventStampStore, NamedEventPlan, NamedEventTarget};
pub(crate) use names::{IdNames, NameMap};
pub use timing_checks::{set_no_notifier, set_no_tchk_msg, set_no_timing_checks};

/// `XEZIM_MEM_CENSUS` helper: serialized size of any serde value behind a
/// `dyn` reference, so the census can list heterogeneous fields in one table.
mod erased_size {
    pub(super) trait Ser {
        fn size(&self) -> usize;
    }
    impl<T: serde::Serialize> Ser for T {
        fn size(&self) -> usize {
            crate::serialized_len(self)
        }
    }
}

#[derive(serde::Serialize)]
pub struct Simulator {
    pub signals: SignalMap,
    vpi_port_directions: HashMap<Arc<str>, PortDirection>,
    /// Signals currently under force/release control (LRM §9.3.1).
    forced_signals: HashMap<usize, Value>,
    /// Annex N seed of a no-seed `$random` (§20.15.1); starts at 0.
    annex_n_seed: i32,
    /// Whether any collection of collections is declared (a recorded
    /// [`CollShape`], or a local registering one at runtime): the one-load
    /// gate in front of row creation on an element write.
    has_coll_rows: bool,
    /// §10.6.2 bit/part-select forces on nets, by signal id. Each id is
    /// also in `forced_signals` (so ordinary writes drop) and has a
    /// `partial` `active_force_exprs` entry that re-merges the drivers.
    partial_forces: HashMap<usize, PartialForce>,
    /// Nets released while a settle pass was running (the entry table is
    /// taken out then); re-driven when the pass ends.
    pending_release_ids: Vec<usize>,
    /// §10.6.2: the comb entries that write each signal, by entry index in
    /// table order, for re-driving a released net. Built at the first
    /// release from `comb_entries` (whose read and write lists are fixed once
    /// compiled) and dropped whenever that table is replaced; `None` until
    /// then. The length is the table's when it was built.
    #[serde(skip)]
    comb_writer_index: Option<(usize, HashMap<usize, Vec<u32>>)>,
    /// Memoization for `prop_bound_collection`: (class, member) → resolved
    /// raw type name (hierarchy walk), and type name → is-collection verdict
    /// (which otherwise LINEAR-SCANS every class's local typedefs — 5% of an
    /// AVIP run). Class/typedef tables are elaboration-static, and per-handle
    /// type bindings are applied AFTER the cached raw lookup, so both maps
    /// are sound to memoize for the life of the run.
    prop_raw_ty_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<String>>>>,
    typedef_target_cache:
        std::cell::RefCell<HashMap<String, HashMap<String, Option<crate::ast::types::DataType>>>>,
    collection_dim_cache: std::cell::RefCell<HashMap<String, bool>>,
    /// Memoized, structurally-fixed CSP for a rand class. `randomize()` on
    /// the same class / constraint set / config values re-translated the
    /// whole constraint graph on every call (csp_vars + csp_split_slices +
    /// csp_translate are a large constant per call); when the structural and
    /// config fingerprint is unchanged the post-translate `Csp` is reused
    /// read-only and only the search (`csp_run`) re-runs against current
    /// values. The fingerprint includes a hash of every non-rand property
    /// value the constraints read, so any relevant change rebuilds. Bound the
    /// table so a long-lived run with many distinct classes can't grow it
    /// without limit.
    #[serde(skip)]
    csp_cache: std::cell::RefCell<HashMap<String, rand_csp::RandCspMemo>>,
    /// Sticky: set the first time a `<base>.` frame marker is created (an
    /// unpacked-struct formal binding, or a struct-return variable via
    /// `unpacked_struct_leaf_keys`); gates `frame_leaf_key_of`'s per-eval
    /// probe.
    any_struct_formal_markers: bool,
    /// Sticky bits (`REF_PARK_*`): a task has bound a formal with select
    /// metadata, an aliased leaf of a `ref` struct formal, or a handle
    /// formal aliased to an element. Each gates the work a process switch
    /// does for it.
    ref_park_flags: u8,
    /// See `Simulator::has_indexed_scopes`.
    #[serde(skip)]
    indexed_scopes: std::cell::OnceCell<bool>,
    /// Scratch structs a pattern actual is spread into before its leaves bind
    /// to an unpacked-struct formal (`struct_pattern_leaf_values`). Recycled
    /// like the mailbox shadows.
    pattern_scratch_seq: u64,
    pattern_scratch_free: Vec<u64>,
    /// Recycled subroutine-frame maps: every call built a fresh
    /// `HashMap<String, Value>` and dropped it on return — at UVM call rates
    /// the table alloc/free was pure allocator churn. `clear()` keeps the
    /// table's capacity, so a pooled frame re-binds formals with zero
    /// rehashing growth.
    /// Bumped whenever any `active_force_signal_epochs` entry advances;
    /// `refresh_active_forces` early-exits when nothing bumped since its
    /// last completed pass (the post-settle refresh otherwise re-scanned
    /// every armed force per settle — 1.6% of an axi4Lite run).
    force_epoch_gen: u64,
    force_refresh_done_gen: u64,
    /// XEZIM_NAME_STATS counters for the string-keyed read path (the UVM
    /// "runtime IDs" investigation). 0=get_signal_value_by_name,
    /// 1=resolve_hier_name, 2=subroutine frames pushed, 3=total locals in
    /// them — the last pair sizes the frame maps, which measured a mean of
    /// 1.8 entries and so would be better served by a linear-scan Vec than
    /// by hashing a String key.
    /// Flattened per-class collection-property lookup for
    /// `instance_assoc_member`: class -> (member -> kind), built once by
    /// walking the ancestry, so the 22.3M-call hot path costs two hash
    /// lookups instead of four map probes plus a linear scan PER ANCESTOR
    /// LEVEL. `true` = per-instance collection (`<handle>#<member>`);
    /// `Some(class)` = static fixed array owned by that class.
    #[allow(clippy::type_complexity)]
    class_coll_index: std::cell::RefCell<HashMap<String, HashMap<String, Option<String>>>>,
    /// Per class: bare names that NO class in its inheritance chain declares
    /// as a property (locals, module signals, method formals). Every bare
    /// identifier evaluated inside a class method asked
    /// `instance_assoc_member`, which walked the chain and probed the
    /// collection tables per level, on every evaluation; such a name can
    /// never become a collection member, so the verdict is cached.
    class_non_member_cache: std::cell::RefCell<HashMap<String, HashSet<String>>>,
    /// Runtime class -> bare name -> `instance_assoc_member` verdict.
    member_coll_cache: std::cell::RefCell<HashMap<String, HashMap<String, MemberCollKind>>>,
    /// Per class: properties that are NOT structs (see `class_prop_struct`).
    class_prop_struct_neg: std::cell::RefCell<HashMap<String, HashSet<String>>>,
    /// Shared aggregate declarations per class and exact concrete type bindings.
    /// Widths still use each object's value-parameter scope (§8.25).
    #[serde(skip)]
    #[allow(clippy::type_complexity)]
    class_prop_struct_cache: std::cell::RefCell<
        HashMap<
            String,
            Vec<(
                HashMap<String, String>,
                HashMap<String, Rc<crate::ast::types::StructUnionType>>,
            )>,
        >,
    >,
    /// Root packed layouts retain their declaration, so pointer keys cannot be
    /// reused. Each value-parameter specialization gets its own dimensions.
    #[serde(skip)]
    class_packed_layout_cache: std::cell::RefCell<HashMap<usize, ClassPackedLayouts>>,
    #[serde(skip)]
    aggregate_type_work: [std::cell::Cell<u64>; 2],
    #[serde(skip)]
    aggregate_type_generation: std::cell::Cell<u64>,
    /// Immutable class parameter names; values are always read from the object.
    #[serde(skip)]
    class_value_param_names: std::cell::RefCell<HashMap<String, Rc<Vec<String>>>>,
    name_stats: [std::cell::Cell<u64>; 5],
    name_stats_on: bool,
    frame_pool: Vec<HashMap<String, Value>>,
    /// Name-keyed type registrations (`widths`, `signed_signals`,
    /// `real_signals`, `string_signals`) that a fallback carrying locals
    /// replaced for its duration: (name, width, signed, real, string) as they
    /// were before. See `fb_frame_push`.
    fb_meta_saves: Vec<(String, Option<u32>, bool, bool, bool)>,
    /// Bare-name geometry (`packed_full_dims`, `packed_signal_elem_widths`,
    /// `packed_struct_fields`, a non-plain `var_decl_types` entry) another
    /// declaration of a carried local's name registered, set aside for the
    /// fallback's duration and put back after it. See `fb_frame_push`.
    fb_geom_saves: Vec<FormalMetadataSnapshot>,
    /// Per carried fallback frame, the `ref` redirections of the compiled
    /// method's formals it set aside (`fb_frame_push`), restored on pop.
    fb_unbound_refs: Vec<Vec<(String, Option<Expression>, Option<String>)>>,
    /// `local_stack` depth below each active fallback frame (see
    /// `fb_frame_push`). The carried locals' types are not in
    /// `var_decl_types`, so a write inside one asks `fb_two_state_names`.
    fb_frame_depths: Vec<usize>,
    /// The carried locals of 2-state type of every active fallback frame,
    /// innermost last; `fb_two_state_marks` holds where each frame's start.
    fb_two_state_names: Vec<Arc<str>>,
    fb_two_state_marks: Vec<usize>,
    /// Lazily-built leaf index over `module.parameters` for the bytecode
    /// compiler's suffix-match fallback (see `lookup_param_value`). Built
    /// once — parameters are final before any bytecode compilation runs.
    #[serde(skip)]
    param_leaf_index_cell: std::cell::OnceCell<HashMap<String, Vec<String>>>,
    /// Lazily-built leaf (`::`-suffix) index over `module.var_decl_types`
    /// for `declared_collection_elem_class`'s package-qualified fallback.
    /// The id-collision-in-heap gates consult that fallback on EVERY
    /// member/indexed access, and the old whole-map `rsplit(':')`-per-key
    /// scan was ~5.5ms per access on a 370K-signal SoC — 96% of a
    /// production run's CPU (a 152s TB ballooned past 26 minutes). Index
    /// entries are CANDIDATE KEYS only; lookups stay authoritative against
    /// the live map, so runtime insertions (formal/foreach metadata,
    /// always keyed by the exact bare name the direct probe already
    /// covers) can neither be missed nor stale-hit.
    #[serde(skip)]
    var_decl_leaf_index_cell: std::cell::OnceCell<HashMap<String, Vec<String>>>,
    /// Flattened names currently under an active `force` (§10.6) or
    /// procedural continuous `assign` (§10.6.1) whose storage lives only
    /// in the runtime `signals` map (no compact signal-table id). Mirrors
    /// `forced_signals` for those targets so the name-keyed write
    /// fallbacks can drop ordinary writes while the override is active.
    forced_names: HashSet<String>,
    /// §13.3.1: last value of each STATIC function's implicit return
    /// variable (key: pkg-scope + fn name). Seeds the next call's return
    /// slot so an unassigned exit path returns the previous value.
    static_fn_ret: HashMap<String, Value>,
    /// §10.6.2: an active force/procedural-assign whose RHS is an
    /// EXPRESSION acts as a continuous assignment — it re-evaluates when
    /// its operands change until release. (target key, lvalue, rvalue,
    /// scope hint at arm time). One-shot snapshots diverged: `force n =
    /// src + 1` held the arm-time value forever.
    active_force_exprs: Vec<ActiveForceExpr>,
    /// Number of armed forces that need conservative refreshes.
    #[serde(skip)]
    unresolved_force_count: usize,
    /// Per-signal write generation used by `active_force_exprs`. Allocated
    /// alongside the signal table and touched only while an expression-backed
    /// override is active.
    active_force_signal_epochs: Vec<u64>,
    /// Deterministic work counters for regression tests: RHS evaluations and
    /// clean dependency checks skipped.
    active_force_evals: u64,
    active_force_skips: u64,
    /// Fast signal table: indexed by signal_id for O(1) access.
    signal_table: Vec<Value>,
    /// Packed byte storage for admitted large narrow arrays; cells addressed
    /// by ids at or above `PACKED_BASE` (see `PackedMem`). Empty unless
    /// `XEZIM_PACKED_MEM=1`.
    packed: PackedMem,
    /// Parallel array: 1 byte per signal_id, non-zero iff that signal
    /// currently has any X/Z bits. Maintained by `write_sig!` and a few
    /// in-place mutators. Used by the JIT's inline X/Z prelude (avoids
    /// a C-bridge call per block dispatch). Sized once in `compile()`
    /// to match `signal_table.len()` and never resized — the address of
    /// `signal_has_xz.as_ptr()` is JIT-baked at compile time and must
    /// stay valid for the simulator's lifetime.
    pub(crate) signal_has_xz: Vec<u8>,
    /// Per-destination sidecar roles used by direct commit paths. Bit 0 marks
    /// edge-history participation; bit 1 marks armed-input fanout.
    signal_commit_plan: Vec<u8>,
    /// Parallel storage to `signal_table`: `[val_bits, xz_bits]` per
    /// signal in a JIT-friendly flat layout.  Future JIT codegen can
    /// read signal values by direct pointer offset (base + id * 16)
    /// instead of crossing the FFI boundary to call
    /// `xezim_jit_load_signal`.  Allocated only when
    /// `XEZIM_INLINE_BITS=1`; default is empty (zero overhead).
    /// Maintained synchronously by `write_sig!` macro and the
    /// canonical `after_signal_write` helper — see
    /// JIT-REDESIGN-NOTES.md for the audit context.
    pub(crate) signal_inline_bits: Vec<[u64; 2]>,
    /// JIT Stage 4 Path C1: side queue for inline JIT NBA writes.
    /// JIT-generated code writes `(signal_id, val_bits)` u32+u64 pairs
    /// directly via a baked-in pointer + the length counter below.
    /// Before `apply_nba` runs, `drain_jit_nba_side_queue` copies the
    /// entries into the canonical `nba_fast` Vec via the existing
    /// elision-and-resize logic.  Pre-allocated to 1M slots × 16
    /// bytes = 16 MB; never resized so the JIT's baked pointer
    /// stays valid for the simulator's lifetime.  Allocated only
    /// when `XEZIM_INLINE_BITS=1`.
    pub(crate) jit_nba_side_queue: Vec<JitNbaSideEntry>,
    /// Length counter for `jit_nba_side_queue`.  JIT writes increment
    /// this directly.  `drain_jit_nba_side_queue` resets to 0 after
    /// transferring entries to nba_fast.
    pub(crate) jit_nba_side_len: u32,
    /// Map signal name → signal_id for fast lookup. `Arc<str>` keys are
    /// shared with `id_to_name` so each signal name lives on the heap
    /// once instead of twice (saves ~25 MB on c910-scale). The elements of
    /// named 1-D arrays are VIRTUAL entries (see `NameMap`).
    signal_name_to_id: NameMap,
    /// Compile-time scope resolver: bare leaf name -> enclosing scopes that
    /// declare that leaf. Avoids rescanning the complete flattened namespace
    /// for every continuous assignment and combinational block.
    scope_parents_by_leaf: HashMap<Arc<str>, Vec<Arc<str>>>,
    /// Lazy per-array element-ID cache: array_name → Vec where index is the
    /// element index and the value is the flat signal_id (None if missing).
    /// Populated on first miss in the hot write/read path; avoids per-access
    /// `format!("{}[{}]", name, i)` + HashMap lookup on tight loops like
    /// memory wipe/load in testbenches.
    array_elem_ids: HashMap<String, Vec<i64>>,
    /// Compact resolver for 1D unpacked-array elements: `base_name` →
    /// (`first_id`, `lo`, `hi`). The N elements of a 1D array occupy
    /// contiguous signal_ids `first_id..=first_id + (hi-lo)`, so name-style
    /// lookups like `"tb.foo.mem[5]"` can be resolved by parsing the suffix
    /// and computing `first_id + (idx - lo)` instead of materialising
    /// `signal_name_to_id` and `id_to_name` entries for every element.
    /// On E902 this saves ~107 MB (~852K array cells; HashMap entries +
    /// per-element Arc<str> strings) and on c910 / large-memory designs
    /// scales linearly with memory depth.
    array_first_id: HashMap<Arc<str>, (usize, i64, i64)>,
    /// Reverse index: leaf-name (last `.`-segment of a signal's full path) →
    /// signal_ids that share that leaf. Built once for the "real" signal set
    /// (excludes per-element array IDs which all end with `]`). Keeps the
    /// `path.len()==1 && bare leaf` resolution path O(1) instead of an
    /// O(N) scan over signal_name_to_id, which on c910 scales to 35M entries
    /// and was the dominant cost (575s) of time-0 settle for testbench-probe
    /// continuous assigns like `assign x = bare_leaf;`.
    leaf_name_to_ids: HashMap<Arc<str>, Vec<usize>>,
    /// Signal id → registered name; array cells without a stored name are
    /// gaps (see `IdNames`), resolved through `name_for_id`.
    id_to_name: IdNames,
    /// Element names of virtual-name arrays, built on first request by
    /// `name_for_id` (append-only: entries are never replaced or removed,
    /// which is what lets it hand out `&str` borrowed from `&self`).
    elem_name_cache: RefCell<HashMap<usize, Box<str>>>,
    /// Indices into `module.instances`, sorted by instance path. Hierarchical
    /// upward-name resolution probes instance paths repeatedly; binary search
    /// avoids a full instance-list scan without duplicating the long paths.
    instance_path_order: Vec<usize>,
    /// Map signal_id → width (for fast width lookup).
    signal_widths: Vec<u32>,
    /// Set of signal IDs that are signed.
    signal_signed: Vec<bool>,
    signal_real: Vec<bool>,
    /// §7.2 unpacked-struct variables for the bytecode compiler's member-wise
    /// struct copy (`BytecodeCompiler::set_unpacked_structs`).
    unpacked_struct_leaves: super::bytecode::UnpackedStructLeaves,
    /// Every signal under each unpacked-struct variable, by the variable's
    /// name (`build_unpacked_struct_sens`): what a whole-struct read depends on.
    unpacked_struct_sens: HashMap<String, Vec<usize>>,
    /// Signal ids whose declared type is 2-state (`bit`/`byte`/`int`/…). Per
    /// §6.11.1/§10.7 an implicit conversion of a 4-state RHS into such a target
    /// maps X/Z to 0 — enforced in `fit_value_to_signal`.
    signal_two_state: Vec<bool>,
    /// Signal ids driven by a lowered GATE PRIMITIVE. §28.4's `buf` truth table
    /// maps a `z` input to an `x` output, but §10.3 continuous assignment passes
    /// the value through unchanged — and a gate lowers to an ordinary continuous
    /// assign, so by the time one-bit copies are fused into a buffer op the two
    /// are indistinguishable. Applying the gate rule to both made every
    /// `assign y = x;` eat `z`. This says which ids the gate rule applies to.
    signal_gate_driven: Vec<bool>,
    /// Signal ids whose declared type is `string`. §6.16 makes a string a
    /// DYNAMIC type with no length limit, but it is stored in the fixed-width
    /// signal table like everything else — `resolve_type_width` hands it 1024
    /// bits, i.e. 128 characters. Fitting a value to that width silently
    /// truncated every longer string (a 130-char concat came back 128, and the
    /// dropped end took a `{base, "TAIL"}` suffix with it). These ids skip the
    /// width fit and keep whatever length they are given.
    signal_is_string: Vec<bool>,
    /// `--warn-x` state. `warn_x` is a plain field (not the atomic) so the
    /// write hot path costs one bool load; everything else is touched only
    /// when a report actually fires.
    warn_x: bool,
    /// Signals already recorded, so a register that re-loads an X input every
    /// cycle is named once rather than ten thousand times. `RefCell` because
    /// `write_sig!` expands in contexts that already hold a `&mut` borrow of
    /// another field (the clock-generator loop), so the hook can only touch
    /// disjoint fields — never `&mut self` as a whole.
    warn_x_seen: std::cell::RefCell<HashSet<usize>>,
    /// Captured at the moment of the write — id, time, and the value — and
    /// formatted later, once a `&mut self` is available again.
    /// id, time, value, and the process that performed the write. The pid is
    /// the only attribution available for a procedural write: `initial`/`always`
    /// bodies are drained into processes at compile time, so they are not in
    /// `module.initial_blocks` any more, but `process_origin` still knows what
    /// each pid came from and where.
    warn_x_pending: std::cell::RefCell<Vec<(usize, u64, Value, usize, Option<String>)>>,
    /// Dedup for entries identified by NAME rather than id — associative and
    /// other elements that live in the untyped `signals` map and so have no
    /// entry in the signal table.
    warn_x_seen_named: std::cell::RefCell<HashSet<String>>,
    warn_x_count: usize,
    /// Lazily built on the first report: signal id -> one line per driver.
    /// Derived from the comb entries and edge blocks on demand, so a run
    /// without `--warn-x` pays nothing for it.
    warn_x_drivers: Option<HashMap<usize, Vec<String>>>,
    /// Signal ids that carry a CONTINUOUS driver — the whole-name LHS of an
    /// `assign` (including the continuous-assigns that inlining synthesizes for
    /// instance port connections). §6.5 forbids mixing continuous and procedural
    /// drivers on a variable, so this is exactly "has no procedural driver", and
    /// §21.7.2.1's `var_type` for such an object is `wire` — what a reference simulator emits
    /// and what makes a viewer colour it as a net. `module.continuous_assigns` is
    /// drained into bytecode at compile time, so the fact is recorded here while
    /// it is still known.
    cont_driven: HashSet<usize>,
    /// Names whose sole continuous driver was an identity port connect that
    /// collapsed into a shared id. The dropped connect made them wire-typed
    /// for a dump, and the shared id can't carry per-name typing.
    collapsed_port_children: HashSet<String>,
    /// §23.3.3, §6.8: whether each renamed input port's formal is 2-state,
    /// by the formal's path without the top module's prefix (from
    /// `ElaboratedModule::input_port_actuals`); see `alias_t0_kind`.
    port_formal_two_state: HashMap<String, bool>,
    /// Set by a `collapse_identity_port_nets` run that reached its fixed
    /// point: the name-map generation it left, the continuous-assign list
    /// (length, buffer) and port-alias count it read, and whether its buffer
    /// pass ran. A later run over the same inputs has nothing to re-point.
    collapse_settled: Option<(u64, usize, usize, usize, bool)>,
    /// Instantiated always blocks awaiting classification, in order (see
    /// `DeferredAlways`); empty once `classify_always_blocks` has run.
    #[serde(skip)]
    deferred_always: Vec<DeferredAlways>,
    /// Sparse: signal_id → declared user type name (e.g. class/struct
    /// type for `MyClass h;`). Only populated for signals where the
    /// elaborator recorded a non-None `type_name` on the source
    /// `Signal` struct — most regular bit/logic signals have None
    /// here. Populated once at construction so `module.signals` can
    /// be freed afterwards.
    signal_type_names: HashMap<usize, String>,
    /// Real signal slots that round writes to binary32.
    shortreal_signals: HashSet<usize>,
    pub widths: SignalWidths,
    pub signed_signals: HashSet<String>,
    /// Names of ARRAY locals whose packed element geometry
    /// `exec_stmt_var_decl` registered in `packed_full_dims` (so a later
    /// same-named array local can drop it).
    array_local_packed_dims: HashSet<String>,
    /// Field layout of the packed-struct element of the array local being
    /// declared, for `declare_local_dyn` to register under the per-call key.
    /// The bare name's previous entry is restored once the layout moves to
    /// that key: (layout, bare name, previous entry).
    #[allow(clippy::type_complexity)]
    local_elem_layout: Option<(
        Vec<(String, u32, u32)>,
        String,
        Option<Vec<(String, u32, u32)>>,
    )>,
    pub real_signals: HashSet<String>,
    /// Base names of 2D/ND UNPACKED arrays (union of `module.arrays_2d` and
    /// `module.arrays_nd` keys). The bytecode compiler consults this so a
    /// continuous-assign LHS `m[0][j]` on a genuine 2D array is NOT mistaken
    /// for a flattened-signal bit-select (`flattened_outer_zero_signal_id`) —
    /// it must bail to the interpreter's element-aware path like `m[1][j]`
    /// already does. Computed once at construction.
    pub multi_dim_array_names: HashSet<String>,
    /// Fast prev signal table for edge detection (indexed by signal_id).
    /// SoA storage for "previous-iter" edge-detection state (A3 from the
    /// compression analysis). Replaces `prev_table: Vec<Value>` for signals
    /// with width ≤ 64 — which is ~95% of typical designs — at half the
    /// per-signal footprint (16 B vs 32 B) and with direct u64 compare
    /// in the check_edges hot path. Wide signals (> 64 bits) fall back to
    /// `prev_wide` below.
    prev_val: Vec<u64>,
    prev_xz: Vec<u64>,
    /// Fallback for signals wider than 64 bits where the inline u64 pair
    /// above can't represent the full state. Until its first snapshot a wide
    /// signal has its full-width all-X baseline implicitly.
    prev_wide: HashMap<usize, Value>,
    /// Snapshot generation counter: bumped every `snapshot_edge_signals`.
    /// (Previously read by `EventWaiter::registered_snap_gen` to guard
    /// pre-registration edges; that mechanism was replaced by per-signal
    /// `captured_prev` baselines captured at arm time — see `EventWaiter` —
    /// which handles the NBA-region / same-tick cases directly. The counter
    /// is retained for diagnostics/future use but is currently write-only.)
    snap_gen: u64,
    edge_signal_names: HashSet<String>,
    /// Edge sensitivity resolved to signal IDs.
    edge_signal_ids: Vec<usize>,
    /// Signals registered by sampled-value watches at run time (`$rose`,
    /// `$past`, …): their preponed value lives in `prev_*` and is read by
    /// `preponed_value_of`, so the incremental snapshot always walks them.
    preponed_ids: Vec<usize>,
    /// Reverse index signal_id → edge_block_idx lists by edge kind, built once after
    /// edge_blocks are classified. Lets `check_edges` iterate edge-sensitive
    /// signals (typically ~100s on c910: clk, rst_b, a few enables) instead
    /// of all edge_blocks (10K+), yielding 50-100× faster edge detection.
    edge_blocks_by_sig: Vec<EdgeFanout>,
    /// Indices (into `edge_blocks`) of edge-sensitive always blocks that
    /// carry at least one `iff` guard on a sensitivity term (LRM §9.4.2.3).
    /// Built alongside `edge_blocks_by_sig`; empty for the common
    /// guard-free design, so `check_edges` skips the guard pass entirely.
    edge_blocks_with_iff: Vec<usize>,
    /// Per-block scratch bitmap (indexed by edge-block index) marking blocks
    /// whose `iff` guard denies firing this delta-cycle. Rebuilt each
    /// `check_edges` before the detection loop so the hot loop can skip a
    /// denied block with a single array read.
    edge_iff_denied: Vec<bool>,
    pub time: u64,
    pub output: Vec<SimOutput>,
    capture_output: bool,
    pub finished: bool,
    /// §20.10: a `$fatal` executed, or the run ended on a run-time error
    /// (`abort_run`) — the run must not exit 0.
    pub saw_fatal: bool,
    /// `$fatal`'s finish_number (0/1/2), for the exit status.
    pub fatal_finish_number: Option<i32>,
    /// Count of `$error` occurrences (see `--error-exit`).
    pub error_count: u64,
    /// Compile-time diagnostics that must prevent simulation (e.g. an illegal
    /// §6.18 assignment of a non-class value to a class handle). The CLI and
    /// library `simulate()` report these and stop; simulation never runs.
    pub compile_errors: Vec<String>,
    /// Set when the dead-clock watchdog aborted the run (XEZIM_STUCK_CLOCK=abort).
    /// The CLI reports a non-zero exit so CI/regressions fail fast.
    pub stuck_clock_aborted: bool,
    compiled: bool,
    pub monitor: Option<(String, Vec<Expression>)>,
    /// `$monitoroff` pauses (not destroys) the monitor; `$monitoron` resumes.
    pub monitor_paused: bool,
    /// System-task names already warned about this run — both the generic
    /// "unknown system task" diagnostic and the recognized-but-unsupported
    /// stubs ($save, $asserton, …) emit ONCE per distinct name so a task in
    /// a clocked loop cannot flood stderr.
    warned_system_tasks: HashSet<String>,
    /// `$strobe` queue — (task name, args, arming instance scope, arming
    /// `%m` lexical suffix): formatted+printed at the end of the current
    /// event-loop iteration, after `apply_nba` has committed scheduled
    /// non-blocking writes. Each entry is `(task_name, args)` and is
    /// re-evaluated at drain time so the printed values reflect the
    /// post-NBA state per IEEE 1800 §15.3.4 (postponed region).
    pending_strobes: Vec<(String, Vec<Expression>, String, String)>,
    /// IEEE 1800-2017 §4.4 "observed region" — drained AFTER the NBA region
    /// has settled and BEFORE the postponed region runs. Concurrent
    /// property/sequence checkers (LRM §16) register a probe expression
    /// here; the runtime evaluates each probe and records the boolean
    /// outcome. Probe = (property-name, predicate-expr, current-instance
    /// `this` handle).
    ///
    /// Today this queue is plumbing only — no producer is wired up.
    /// Concurrent assertions land in M9, which depends on this region
    /// existing. Drain is a no-op when empty so behavior is unchanged.
    pending_observed: Vec<ObservedProbe>,
    /// §16.4 deferred immediate assertion reports awaiting maturity.
    deferred_asserts: Vec<DeferredReport>,
    /// §20.12: an assertion control is in force (the control log is not
    /// empty); every assertion path tests only this until then.
    assert_ctl_on: bool,
    assert_ctl: Option<Box<AssertCtl>>,
    /// `typedef_layout_member`'s unpacked-struct typedef layouts by total
    /// width; dropped when a block-local typedef is registered.
    typedef_layouts_by_width: Option<HashMap<u32, Vec<Vec<(String, u32, u32, bool)>>>>,
    /// Class packed ranges needing label translation, keyed by (class,
    /// property); None for descending ranges ending at zero.
    class_prop_dim_cache: std::cell::RefCell<HashMap<(String, String), Option<(i64, i64)>>>,
    /// LRM §16.5 SVA clocked-assertion sites. Populated lazily on
    /// the first time the AssertionStatement{is_property,
    /// expr:SvaClocked} executes. Each site's saved event values let us
    /// edge-detect the clock signal each loop iteration; `pending`
    /// queues consequents deferred by `|=>` or `##N`.
    sva_sites: Vec<SvaClockedSite>,
    /// §16.10: the local variables of the attempt thread being evaluated,
    /// read through slot markers (see `sva_bind_locals`).
    sva_lv: Vec<Value>,
    /// `(width, signed, two-state)` of each local variable slot of the site
    /// being registered (see `sva_bind_locals`).
    sva_lv_types: Vec<(u32, bool, bool)>,
    /// §16.14.6.2: procedural concurrent assertion instances queued this time
    /// step, not yet matured: `(site, process, captured slot values)`. A
    /// flush point of the process drops its own.
    sva_proc_pending: Vec<(usize, usize, Vec<Value>)>,
    /// §16.15 `default disable iff`, by the scope that declares it.
    sva_default_disable: HashMap<String, Expression>,
    /// LRM §16.9.3: index of the SVA site whose body is currently
    /// being evaluated. Set by `tick_sva_sites` around each body
    /// eval; consulted by `eval_expr`'s `$past` handler to find the
    /// site's snapshot map. `None` when no SVA body is active —
    /// `$past` then returns 0.
    active_sva_site: Option<usize>,
    /// LRM §16.5.1 Preponed-region sample cache for clocked concurrent
    /// assertions. Keyed by signal id, holds each SVA-referenced signal's
    /// value as of the START of the current time slot (captured at slot
    /// entry, before the active/NBA regions run). When a clocked property
    /// fires on its clock edge, its predicate is evaluated against these
    /// sampled values rather than the live post-NBA values, so a signal
    /// that changes on the SAME edge is seen with its old (preponed) value
    /// — matching the §16.5.1/§16.9 sampled-value semantics instead of
    /// being one clock cycle early. Only populated when `sva_sites` is
    /// non-empty; the referenced ids are those collected per site.
    sva_preponed: HashMap<usize, Value>,
    /// The time slot `sva_preponed` was captured for. The Preponed region
    /// runs once per slot (§4.4.2.1), so the later deltas of a slot (a VPI
    /// write from cbReadWriteSynch reopening the Active region) keep it.
    sva_preponed_slot: Option<u64>,
    /// A site's clock event is recorded (`fired`) and not evaluated yet.
    sva_owed: bool,
    /// §16.12.14: `sva_advance` runs in a time step without a tick of the
    /// site, so only asynchronous abort conditions are checked and nothing
    /// else advances (`sva_abort_step_site`).
    sva_abort_step: bool,
    /// `sva_post_nba` is running the slot's due cbReadWriteSynch batches.
    sva_post_nba_busy: bool,
    /// `vpi_slot_post_nba` is running the slot's leftover cbNBASynch and
    /// cbReadWriteSynch routines: the delta one opens matures the `#0`
    /// deferred reports before its Reactive work.
    #[serde(skip)]
    vpi_slot_maturing: bool,
    /// LRM §14.3 clocking-block per-cycle input snapshots. Each entry
    /// is `cb_name → (signal_name → pre-edge value)`. Updated in
    /// `tick_clocking_blocks` just before the clock signal's posedge
    /// is consumed by edge-triggered always blocks. Reads of
    /// `cb.<signal>` (where the signal is direction `input`) return
    /// from this snapshot — the `#1step` input skew of LRM §14.3.
    /// Without the snapshot, `cb.sig` would return the post-edge
    /// value, defeating monitor patterns that rely on stable
    /// pre-edge sampling.
    clocking_snapshots: HashMap<String, HashMap<String, Value>>,
    /// §14.3: leaf names of unpacked-array clockvars, whose samples are kept
    /// per element (`ready[0]`); the prefilter of `cb.sig[i]` reads.
    clocking_array_leaves: HashSet<String>,
    /// LRM §14.3: per-cb metadata. `(clock_signal_name, [(signal_name, is_input)])`.
    /// Populated at compile() from `module.clocking_blocks`.
    clocking_meta: HashMap<String, (String, Vec<(String, bool)>)>,
    /// §27.6: some clocking block is declared in a generate copy or an
    /// instance-array element (its key holds a select, `gen[1].cb`), so a
    /// name with a select (`gen[1].cb.a`) may reach a block. Without one, the
    /// indexed lookups of member reads are skipped.
    clocking_in_indexed_scope: bool,
    /// LRM §14.4: pending clocking-output drives. Each drive retains its
    /// clockvar name, physical net and value. `cb.<out_sig> <= val` queues here instead of driving
    /// immediately; `tick_clocking_blocks` applies them at the next
    /// posedge of the cb's clock (output skew).
    /// A drive of a SELECT (`cb.v[2] <= 1`) carries its target: the net with
    /// the drive's indices frozen.
    clocking_output_pending: HashMap<String, Vec<ClockingOutputDrive>>,
    /// Per-clocking-block previous clock bit (for edge detection).
    clocking_prev_clock: HashMap<String, u8>,
    /// Per-tick clock poll table for `tick_clocking_blocks` (see there).
    clocking_tick_cache: Vec<ClockingTick>,
    /// Buffer-collapse exclusions (override-target leaves, procedural write
    /// names), collected once while the process bodies are still present.
    buf_collapse_write_sets: Option<(HashSet<String>, HashSet<String>)>,
    /// Last source value of constant packed-memory element connections.
    /// Unresolved dependency polling must not overwrite a VPI deposit when
    /// the connection's source has not changed (§23.3.3.3, §38.34).
    packed_array_copy_values: HashMap<(usize, usize), Value>,
    /// LRM §14.3: each clocking block's clock EDGE (`@(posedge/negedge clk)`).
    /// `tick_clocking_blocks` and `@(cb)` resolution consult this so a
    /// `@(negedge clk)` block samples/advances on the negedge, not the posedge.
    /// Absent ⇒ posedge (the default and by far the common case).
    clocking_edge: HashMap<String, EdgeKind>,
    /// Set when `tick_clocking_blocks` published a changed sample mirror, so
    /// the caller runs one more edge pass — the publish happens after this
    /// slot's edge detection, and the next iteration's snapshot would
    /// otherwise absorb the change and swallow the `@(cb.sig)` event.
    clocking_mirror_dirty: bool,
    /// Set when `tick_clocking_blocks` drove an edge-skew output, so the
    /// caller settles and runs one more edge pass: the drive lands after this
    /// slot's edge detection, and its fan-out must still wake in this slot.
    clocking_edge_drive_dirty: bool,
    /// §14.16 `cb.out <= ##N v` drives awaiting N further clocking events:
    /// cb → [(remaining_cycles, clockvar, net, value, target)]. Decremented at each edge of that
    /// block; applied (with output skew) when it reaches zero.
    clocking_cycle_pending: HashMap<String, Vec<ClockingCycleDrive>>,
    /// §14.16.2: clocking select drives waiting for their maturity as
    /// nonblocking updates: (maturity time, signal). A whole drive of the
    /// same signal maturing with one queues behind it, so the drives take
    /// effect in the order they were issued.
    clocking_select_due: Vec<(u64, usize)>,
    /// §14.3 signal RENAMING (`input alias = net;`): cb → (clocking name →
    /// resolved net). The snapshot/meta store the NET; reads and drives use
    /// the clocking name, so they translate through this first.
    /// §14.16: a clocking OUTPUT clockvar drives a stage of its own when
    /// the net it is bound to has another driver: (block name, net) →
    /// stage signal. The clockvar drives the stage, and the stage joins the
    /// net's other drivers in the resolution fold.
    clocking_output_stages: HashMap<(String, String), String>,
    /// §14.3 signal RENAMING (`input alias = net;`): cb → (clocking name →
    /// resolved net). The snapshot/meta store the NET; reads and drives use
    /// the clocking name, so they translate through this first.
    clocking_sig_alias: HashMap<String, HashMap<String, String>>,
    /// §14.5 a clockvar bound to a select of a net (`input a = bus[3:0];`):
    /// cb → (clocking name → the bound expression). The meta/alias store the
    /// BASE net; a sample applies the select to the base's sample, a drive
    /// writes through it (#823).
    clocking_sig_bound_sel: HashMap<String, HashMap<String, Expression>>,
    /// §14.5 a clockvar bound to an expression (`input sum = a + 1;`,
    /// `input i = {op, ra, rb[3:1]};`): cb → (clocking name → (the bound
    /// expression, each operand net with the hidden clocking name of its
    /// sample)). Each operand is sampled as a clocking input of its own, with
    /// the clockvar's skew; the clockvar's sample is the expression evaluated
    /// over those samples.
    clocking_sig_bound_expr: HashMap<String, HashMap<String, ClockingBoundExpr>>,
    /// §14.4 per-cb default OUTPUT skew in ticks (absent = #0).
    clocking_out_skew: HashMap<String, u64>,
    /// §14.4 per-signal output-skew overrides: cb → (resolved net → ticks).
    clocking_sig_out_skew: HashMap<String, HashMap<String, u64>>,
    /// §14.3 edge of a block's `default output negedge` skew.
    clocking_out_edge: HashMap<String, EdgeKind>,
    /// §14.3 per-signal skews: cb → (net → edge, or None for a delay-only
    /// skew that overrides the block's default edge).
    clocking_sig_out_edge: HashMap<String, HashMap<String, Option<EdgeKind>>>,
    /// Drives through an edge-skew output, waiting for that edge of the
    /// block's clock: cb → [(net, value, edge, post-edge delay, select target)]
    /// in issue order.
    clocking_edge_out_pending: HashMap<String, Vec<ClockingEdgeDrive>>,
    /// §14.4 per-cb default INPUT skew in ticks (absent = #0).
    clocking_in_skew: HashMap<String, u64>,
    /// §14.4 per-signal input-skew overrides: cb → (resolved net → ticks).
    clocking_sig_in_skew: HashMap<String, HashMap<String, u64>>,
    /// Sim time of each cb's most recent clock edge — a `cb.out <= v` executed
    /// AT the edge time drives at edge+skew (§14.16.1); between edges it waits
    /// for the next edge.
    clocking_last_edge: HashMap<String, u64>,
    /// The `default clocking` block's real name — `##0` consults its
    /// `clocking_last_edge` entry to decide "already at the clocking event".
    default_clocking_cb: Option<String>,
    /// LRM §14.4 `#1step` input skew — Preponed (slot-entry) value of every
    /// clocking-block INPUT signal, keyed by signal name. Captured at the top
    /// of each time slot BEFORE callback writes and active/NBA regions run,
    /// so when a clock posedge fires later in the slot the
    /// input snapshot is the value from *before* the edge, not the same-edge
    /// NBA update. Without this, `cb.<in>` reads the post-edge value (off by
    /// one clock relative to a reference simulator).
    clocking_preponed: HashMap<String, Value>,
    /// Slot whose Preponed clocking-input values have already been captured.
    clocking_preponed_slot: Option<u64>,
    /// §14.4 `input #N` history per sampled net (see `ClockingHist`). Empty
    /// unless some clocking input has an explicit positive skew.
    clocking_hist: HashMap<String, ClockingHist>,
    /// Time of the slot `refresh_clocking_preponed` last ran in.
    clocking_hist_slot: Option<u64>,
    /// §14.3 clocking-visible name of each `clocking_meta` signal, in the
    /// same order. Snapshots are keyed by it, so `input d; input #3 dl = d;`
    /// keeps two samples of one net.
    clocking_vis: HashMap<String, Vec<String>>,
    /// §14.16.2 inout clockvars: cb → nets that are BOTH sampled and driven.
    clocking_inouts: HashMap<String, HashSet<String>>,
    /// Continuations of clocking-event waiters (`@(cb)` / `##N`) that fired this
    /// slot, held for resumption in the Reactive region — after `apply_nba` and
    /// `tick_clocking_blocks` — instead of running in the Active region with the
    /// raw edge. See `EventWaiter::is_clocking`.
    deferred_clocking_conts: Vec<(usize, ProcCont)>,
    /// §9.4.2.3/§14.13: clocking-event waiters (`@(cb iff g)`) whose edge
    /// fired this slot but whose `iff` guard is judged only once the block
    /// has sampled — the clocking event, and so its guard, occurs after the
    /// clockvars update. Each entry holds the waiter and the guard of the
    /// term that fired; see `resolve_deferred_clocking_guards`.
    deferred_clocking_guarded: Vec<(EventWaiter, Rc<Expression>)>,
    /// LRM §4.4 "reactive region" — drained AFTER the observed region
    /// has fired and BEFORE the postponed region runs. In a strict LRM
    /// implementation this hosts `program`-block procedural code so a
    /// testbench observes design state only after design NBAs have
    /// settled. Today the queue is framework-only (no producer is
    /// wired up); drain is a no-op when empty, so behavior is unchanged.
    /// Each entry is one program `initial` block: its statements and its
    /// instance scope (`""` for a top-level program).
    pending_reactive: Vec<(Vec<crate::ast::stmt::Statement>, String)>,
    /// §24.3 / §4.4: the processes of `program` blocks (their initial
    /// blocks and every fork child they spawn). They resume in the Reactive
    /// region, after the slot's NBA updates and clocking events.
    program_pids: HashSet<usize>,
    /// §24.3: the program instance (its scope) of each live program process,
    /// the program initial procedures still running, and how many of each
    /// program's are. When a program's initial procedures have all ended,
    /// its remaining processes end too; when no program process is left, the
    /// run ends by an implicit `$finish`.
    program_pid_scope: HashMap<usize, String>,
    program_initial_pids: HashSet<usize>,
    program_initials_live: HashMap<String, usize>,
    /// §24.3: program-process continuations woken in the Active region (an
    /// edge, a delay, a join), held for this slot's Reactive region.
    reactive_conts: Vec<(usize, ProcCont)>,
    /// Set while `drain_reactive_region` runs program processes.
    in_reactive_drain: bool,
    /// §7.9.11: the `default:` value of a class associative-array member
    /// (`<handle>#<member>`) that an assignment pattern gave it, read for
    /// every key the array does not hold.
    class_assoc_defaults: HashMap<String, Value>,
    /// §7.2.1: variables of a 4-state packed struct type with 2-state
    /// members (`struct packed {bit [3:0] b; logic [3:0] l;}`). Such a
    /// member converts x/z to 0 on every read and write, which the compiled
    /// member slices do not do, so their field layouts are withheld from the
    /// bytecode compiler (`bc_packed_fields`) and the interpreter serves them.
    mixed_packed_vars: HashSet<String>,
    /// §8.9: the storage object of each STATIC unpacked-struct property,
    /// keyed by its static key (`Cls::prop`): a heap object of the declaring
    /// class that holds the member cells (`<prop>.<member>`) every access —
    /// `Cls::prop.m`, `obj.prop.m`, `prop.m` in a method — shares.
    static_struct_holders: HashMap<String, usize>,
    /// Names of the static unpacked-struct properties any class declares
    /// (gates the `static_struct_holders` lookups).
    static_struct_names: HashSet<String>,
    /// `module.packed_struct_fields` without the `mixed_packed_vars` entries;
    /// `None` when there are none.
    bc_packed_fields: Option<HashMap<String, Vec<(String, u32, u32)>>>,
    pub monitor_prev: HashMap<String, Value>,
    /// Previously-printed values of the active `$monitor`'s non-`$time`
    /// arguments. `$monitor` re-prints only when one of these changes
    /// (LRM §21.2.3); `None` forces a print (fresh registration).
    monitor_arg_prev: Option<Vec<Value>>,
    /// Instance scope in force when `$monitor`/`$fmonitor` was ARMED.
    ///
    /// `check_monitor` re-formats the saved format string at the end of every
    /// time slot, long after the arming statement returned — so a `%m` in it
    /// used whatever scope happened to be executing then (usually none, which
    /// degrades to the top module). §21.2.1.7 ties `%m` to the scope of the
    /// statement that CONTAINS it, so a `$fmonitor(fd, "%m")` inside a bound
    /// instance must keep reporting that instance. Captured here at arm time
    /// and reinstalled around the re-format.
    monitor_scope: String,
    /// The LEXICAL half of that `%m` (process block label + task / function /
    /// named-block chain), captured at the same moment. `monitor_scope` alone
    /// only pins the INSTANCE: a slot-end check that fires while some
    /// unrelated task is mid-flight would otherwise append that task's name.
    monitor_m_suffix: String,
    /// Set only while `check_monitor` re-formats, so the `%m` arm uses the
    /// captured suffix instead of the live scope stack.
    monitor_m_active: bool,
    /// Active tag for a tagged union variable: signal name → tag name.
    pub active_union_tag: HashMap<String, String>,
    pub max_time: u64,
    /// Seconds per simulation tick (finest timescale precision; 1e-9 default).
    pub tick_s: f64,
    /// $timeformat state (§21.3.5): (units_exp, fraction_digits, suffix,
    /// min_field_width). `None` until `$timeformat` is called; %t then uses
    /// the design's finest precision, 0 fraction digits, no suffix.
    time_format: Option<(i32, usize, String, usize)>,
    /// Maximum iterations for combinatorial settling per cycle.
    pub settle_limit: u32,
    /// §5 diagnosability: how many settles exhausted the cap, and when last.
    settle_limit_hits: u64,
    settle_limit_last_time: u64,
    /// §5.5/§10.3 (#35): when set, the running settle DEFERS cont-assign
    /// family entries (the writing PROCESS must not observe its own write's
    /// propagation until it yields); UDP/gate/always entries stay eager.
    proc_settle_defer: bool,
    /// READ signal ids of the entries skipped while deferring. The next FULL
    /// settle re-dirties them so the skipped entries re-trigger through the
    /// normal dependency machinery — entry INDICES would go stale across the
    /// topo reorder / lazy materialization of `comb_entries`.
    deferred_proc_entries: Vec<u32>,
    /// (#1149) One bit per signal id listed in `deferred_proc_entries`, so
    /// each id is listed once; the drain clears the bits it consumes. An
    /// entry that re-triggers on every settle (an unresolved read) is
    /// deferred again on every blocking write of a process, and a long
    /// time-0 block holds the drain back until its start batch ends.
    deferred_proc_seen: Vec<u64>,
    /// (#1149) Whether the `u32::MAX` marker (an entry with no resolvable
    /// read) is in `deferred_proc_entries`.
    deferred_proc_unres: bool,
    /// The buffer of a deferring settle's changed signals: the ones its
    /// process wrote (the dirty seeds), then the outputs its eager entries
    /// changed. A deferred `always` entry lists these, not every signal it
    /// reads: re-dirtying a read that did not change (`x` of
    /// `always_comb y = f(x)` woken by `g`) would wake blocks the change
    /// never woke (`always @* z = f(x)`, whose sensitivity is `x` alone).
    defer_changed: Vec<usize>,
    /// (#t0-comb-defer) Compile-time process starts still unrun in the
    /// time-0 slot. A commercial simulator queues the cont-assign /
    /// port-connect update events that a time-0 process write schedules
    /// BEHIND every start activation of the slot (active-region FIFO), so a
    /// source-later blocking-body `always` parked on the propagated signal
    /// is armed before the propagation fires. While positive, the #35
    /// deferred-entry drain is held; it releases once, when the last start
    /// of the batch has run.
    t0_starts_remaining: usize,
    /// §18.12 (#519): the multi-dimensional arrays of the `std::randomize`
    /// call being set up — name as the constraints write it, bounds
    /// outermost first — handed to the scope solve as `Csp::nd_dims`.
    rand_scope_nd: HashMap<String, Vec<(i64, i64)>>,
    /// §7.4.5: the `std::randomize` targets with a dynamic outer dimension
    /// over unpacked rows, by storage key: true for dynamic rows (`T d[][]`),
    /// false for fixed ones (`T a[][N]`). Set by `std_randomize_vars`.
    rand_scope_row_owners: HashMap<String, bool>,
    /// Of `rand_scope_nd`'s arrays, the number of leading dimensions that
    /// are dynamic (a dynamic outer dimension, then dynamic rows), by the
    /// name the constraints write: `Csp::nd_dyn`.
    rand_scope_nd_dyn: HashMap<String, usize>,
    /// §21.2.1.7: set while the `%p` operand itself is being rendered; a
    /// standalone scalar prints plain decimal, a leaf INSIDE an aggregate
    /// prints by its declared type (`'h` hex for vectors). Cleared by the
    /// aggregate renderers before they descend into members or elements.
    p_top_level: bool,
    /// §8.4: method names already reported as called through a null handle
    /// from inside class code (one report per method, not per call).
    null_call_reported: HashSet<String>,
    /// Maximum snapshot→apply_nba→settle→check_edges rounds per event-loop
    /// iter (see drain_edge_cascade). Cached at sim init; override via
    /// XEZIM_CASCADE_LIMIT env var.
    cascade_limit: u32,
    /// True when `XEZIM_CASCADE_LIMIT` was set explicitly: the cascade is then
    /// cut off at that many generations (opt-in strict mode). When unset, the
    /// cascade may run past `cascade_limit` generations and is only declared a
    /// livelock when the signal state recurs (it is cycling, not progressing).
    cascade_limit_explicit: bool,
    /// SDF delay annotation (None if no SDF loaded).
    #[serde(skip)]
    pub sdf_annotation: Option<super::sdf::SdfAnnotation>,
    /// min/typ/max selection from the CLI (--sdf-min/typ/max or the
    /// +mindelays/+typdelays/+maxdelays plusargs). A runtime `$sdf_annotate`
    /// honors this; `None` means Typ (the CLI default).
    #[serde(skip)]
    pub sdf_select: Option<super::sdf::DelaySelect>,
    /// Per-signal delay in sim ticks (0 = no delay). Indexed by signal_id.
    sdf_delays: Vec<u64>,
    /// Pending delayed signal updates: (time, signal_id, value)
    // A mask limits a delayed update to the bits its driver owns (§10.3.3).
    delayed_updates: Vec<(u64, usize, Value, Option<Value>)>,
    /// §10.3.3 / §28.16: some net may hold more than one pending update in
    /// `delayed_updates` (select drivers due at different times, see
    /// `schedule_delayed_slice_chain`). Cleared when no update is pending.
    delayed_slice_chains: bool,
    /// §4.9.4: future-time NBA updates (`a <= #d v`). Kept SEPARATE from
    /// `delayed_updates` (gate/CA transport, an ACTIVE-region event at
    /// maturity): these commit in the target slot's NBA region, after that
    /// slot's Active reads.
    delayed_nba: Vec<(u64, usize, Value)>,
    /// Future NBA writes whose targets require captured declaration context.
    delayed_nba_queue: Vec<(u64, NbaEntry)>,
    /// NBA writes into PACKED-ARENA cells (issue #147). A packed cell has no
    /// `signal_table` slot, so it cannot ride `nba_fast` (apply indexes the
    /// per-signal tables) — and committing immediately gave NBAs into big
    /// RAMs blocking semantics: a same-timestep reader in another edge block
    /// saw the new value a delta early. (eid, value plane, xz plane); tiny —
    /// entries exist only between an edge exec and the same tick's NBA
    /// region. Last-write-wins by linear scan.
    packed_nba: Vec<(usize, u64, u64)>,
    /// §29 per-instance UDP runtime state, indexed by `CombItem::Udp{idx}`.
    udp_runtime: Vec<UdpRuntime>,
    /// True when the design contains any UDP instance — forces the serial
    /// settle path (sequential UDPs need `&mut self` to update their state,
    /// which the parallel/BSP isolated eval cannot provide).
    has_udp: bool,
    /// §31 timing checks, and the signals their terminals watch (indexed
    /// by `CombItem::TimingCheck{idx}`).
    timing_checks: Vec<timing_checks::TimingCheckRt>,
    timing_watches: Vec<timing_checks::TcWatch>,
    /// The conditions of the timing checks, compiled on first use
    /// (`timing_checks::TcCond`): check `i`'s start at `timing_cond_base[i]`,
    /// one per condition it has. They depend on the design only, so a
    /// checkpoint neither holds nor resets them.
    #[serde(skip)]
    timing_cond_base: Vec<u32>,
    #[serde(skip)]
    timing_conds: Vec<timing_checks::TcCond>,
    /// Condition evaluations of the timing checks: compiled, interpreted
    /// (`[PROF] timing_conds`).
    #[serde(skip)]
    timing_cond_evals: [u64; 2],
    /// Like `has_udp`: timing checks keep per-check state, so their comb
    /// entries force the serial settle.
    has_timing_checks: bool,
    /// §30.4 module paths (see `module_paths`); `None` without any, so a
    /// design with no specify paths carries one null pointer.
    module_paths: Option<Box<module_paths::ModulePaths>>,
    /// Timing-check events seen by a settle inside a running process,
    /// evaluated once it suspends: (check, event role).
    timing_pending: Vec<(u32, u8)>,
    /// Armed time-based `$timeskew`/`$fullskew` deadlines: (time, check).
    timing_timers: Vec<(u64, usize)>,
    module: ElaboratedModule,
    #[serde(skip)]
    dpi_libraries: Vec<Library>,
    #[serde(skip)]
    dpi_bindings: HashMap<String, DpiBinding>,
    dpi_unsupported: HashSet<String>,
    dpi_unresolved: HashSet<String>,
    /// §8.10 non-static-method-through-`::` diagnostics already emitted
    /// (`(class, method)` pairs) — a given pair is reported once.
    nonstatic_scope_reported: HashSet<(String, String)>,
    /// Imports whose library binding has been attempted.
    dpi_bind_tried: HashSet<String>,
    /// State of the built-in UVM DPI-C helpers (`uvm_dpi.rs`).
    #[serde(skip)]
    uvm_dpi: uvm_dpi::UvmDpiState,
    /// Class instance heap (index 0 is null).
    heap: Vec<Option<Box<ClassInstance>>>,
    /// Class reclamation bookkeeping (`heap_gc`): heap length at the last
    /// collection, live objects it left, and running totals.
    class_gc_heap_len: usize,
    /// Objects live after the last collection, ascending. Handles are only
    /// ever appended, so these plus the slots from `class_gc_scan_from` on
    /// are every live object: a collection never walks dead slots.
    class_gc_survivors: Vec<usize>,
    class_gc_scan_from: usize,
    class_gc_live: usize,
    class_gc_live_signals: usize,
    class_gc_released: usize,
    class_gc_runs: usize,
    /// LRM §25.8 virtual-interface bindings.
    /// Key: `(class_instance_handle, property_name)`.
    /// Value: `(bound_iface_inst_name, modport_opt)`. The modport is
    /// carried from the class property's declared type (e.g. `virtual
    /// bus_if.master vif`) so the rewrite path can also emit a direction
    /// warning when writing to a modport-input member.
    virtual_iface_bindings: HashMap<(usize, String), (String, Option<String>)>,
    /// §25.9/§8.9: the interface instance a STATIC class property holding a
    /// virtual interface is bound to, keyed by the property's static storage
    /// key (`S::spv`, `G#virtual pif::one`) — one binding per class (per
    /// specialization), shared by every object and static method.
    static_vif_bindings: HashMap<String, String>,
    /// The property names `static_vif_bindings` holds a binding of (the
    /// cheap gate `vif_rhs_possible` checks).
    static_vif_names: HashSet<String>,
    /// Every static property name some class declares as a virtual
    /// interface or through a type parameter (built on first use): the gate
    /// before `bind_static_vif` looks at an assignment.
    #[serde(skip)]
    vif_capable_statics: std::cell::OnceCell<HashSet<String>>,
    /// Virtual-interface member names registered as flattened storage.
    #[serde(skip)]
    vif_member_leaves: std::cell::OnceCell<HashSet<String>>,
    /// The leaf names of the unpacked variables declared below a scope
    /// (`arr` of `bus.arr`): the prefilter of `vif_aggregate_canon`.
    #[serde(skip)]
    scoped_unpacked_leaves: std::cell::OnceCell<HashSet<String>>,
    /// False once start-up found no such static (`vif_capable_statics`
    /// empty): the one-byte test every blocking assignment makes.
    may_bind_static_vif: bool,
    /// The names of static fixed arrays of virtual interfaces some class
    /// declares (§25.10, §8.9): the gate before `static_vif_array_elem`.
    static_vif_array_names: HashSet<String>,
    /// §13.4/§8.5: the CALL in receiver position of a member/select chain
    /// being evaluated (`me()` in `me().o.x`, `q.pop_front()` in
    /// `q.pop_front().addr`), with its one value: `(node address, call-frame
    /// depth, value)`. The chain's paths re-evaluate their receiver freely;
    /// while an entry is live, the same node at the same depth returns the
    /// value instead of running the call again. See `recv_call_once_eval`.
    recv_call_memo: Vec<(usize, usize, Value)>,
    /// A virtual-interface handle token has been stored in a collection
    /// element (see `queue_eval_arg`): receivers may then resolve BY VALUE.
    vif_tokens_stored: bool,
    /// Memo for transitive blocking-task detection (pure-LRM mode only): maps a
    /// subroutine name to whether its body — following calls — eventually hits a
    /// blocking construct (`#`/`@`/`wait`/`fork…join[_any]`). Lets a task that
    /// blocks only via nested calls route through the suspend-aware runner
    /// instead of the name whitelist. Keyed by bare name (over-approx across a
    /// free task / same-named method). See `subroutine_name_blocks`.
    tb_cache: task_suspend::CycleMemo,
    /// The wait-free analysis that gates compiling a class task (see
    /// `task_suspend`).
    ts: task_suspend::TsState,
    /// LRM §25.9: stack of per-call virtual-interface formal-arg
    /// aliases. When a task or function takes `virtual <iface> <name>`,
    /// the call hooks add a frame mapping `<name>` to the caller's
    /// interface-instance identifier (e.g. `bus`). `resolve_hier_name`
    /// consults the top frame so `vif.member` resolves to `bus.member`.
    /// Popped at task return.
    local_iface_aliases: Vec<HashMap<String, String>>,
    /// LRM §25.9: name aliases for PLAIN variables declared `virtual
    /// <iface>` (block-local or module-scope, not class properties and
    /// not subroutine formals). `vif = bus;` records `vif -> bus` here;
    /// the same resolve sites that consult the top
    /// `local_iface_aliases` frame fall back to this map, so
    /// `vif.member` reads/writes and `vif.f()` calls dispatch to the
    /// bound instance. `vif = null` removes the entry. Keyed by bare
    /// variable name (consistent with the simulator's global-by-bare-
    /// name treatment of subroutine locals).
    viface_var_aliases: HashMap<String, String>,
    /// §25.9: the interface instance behind the vif a function RETURN just
    /// produced (`return req_vif;`) — consumed by a `vd = call();` assignment
    /// so accessor-style getters keep the binding. Cleared before each such
    /// call.
    last_vif_return: Option<String>,
    /// While a `with (item...)` filter is being evaluated over a queue whose
    /// elements are unpacked STRUCTS, `item` names no value — the element's
    /// members live in separate signals. Bind it to the element's flat name
    /// (`q[3]`) so `item.field` reads `q[3].field`. Checked at the eval sites,
    /// before `resolve_hier_name`, whose per-node cache would freeze the first
    /// element's binding for the whole sort.
    item_alias: Option<String>,
    /// Custom iterator name bound inside a locator/`with` filter (e.g. `i` in
    /// `q.find(i) with (i.field)`). `item_alias` resolves `item.field`; with a
    /// custom name the struct-member lambda reads `i.field`, which must also
    /// dereference through the aliased element path. Defaults to `item` when
    /// the user wrote no explicit iterator.
    locator_iter: String,
    /// LRM §18.5.4 dist-pick-once tracking (per-randomize scope). Records
    /// `(handle, prop_name)` pairs for which a weighted `dist` constraint
    /// has already drawn one sample in the current randomize call. The
    /// outer 16-pass fixpoint loop would otherwise re-pick on every
    /// iteration, biasing the empirical distribution toward higher-
    /// weighted values (9:1 weights produced ~98:2 instead of 90:10).
    /// Cleared by `exec_randomize` at entry.
    dist_picked_once: HashSet<(usize, String)>,
    /// The dist items the current trial dealt, as `target inside {item}`:
    /// the joint solve after a failed trial first keeps each target in its
    /// item (§18.5.4).
    dist_dealt: Vec<ConstraintItem>,
    /// IEEE 1800-2017 §18.3/§18.5.12 — the `inside`-range domain of each rand
    /// property of the object currently being solved (exact, signed). The
    /// fixpoint consults it so an affine solve or a `!=` re-pick lands the
    /// target inside its DECLARED range rather than merely satisfying the one
    /// item in hand (an `unique {a, arr[0]}` re-pick used to blow `a inside
    /// {[1:10]}` wide open). Rebuilt at the top of every solver trial.
    rand_ranges: HashMap<String, Vec<(i128, i128)>>,
    /// The same for the ELEMENTS of each rand array / collection property:
    /// the hard `inside` domain its elements are constrained to (signed,
    /// exact). Refreshed at the top of every repair pass, so a conditional
    /// domain follows the current values of its condition.
    rand_elem_ranges: HashMap<String, Vec<(i128, i128)>>,
    /// Static class properties — one shared cell per `Class::prop`,
    /// keyed `"ClassName::propname"` where ClassName is the class that
    /// declared the static property.
    class_statics: StaticMap,
    /// §8.25/§6.20.2 (issue #423): lazily materialized per-specialization
    /// elements of class-scope ARRAY constants (header value parameters
    /// with unpacked dims, class localparam arrays), keyed
    /// `DeclClass#canon_sig::name` — or `DeclClass::name`-shaped default
    /// bindings when no specialization is active. See
    /// `materialize_class_param_array`.
    class_param_arrays: HashMap<String, ClassParamArray>,
    /// Per-frame overlay for class/typedef types of frame-locals. The
    /// global var_class_types/var_typedef_types are keyed by BARE name and
    /// shared across every process, so a class-typed local declared in one
    /// subroutine (or another process) silently overwrote the entry a
    /// different frame relied on — `$cast`/`new`-resolution then read a
    /// stale class and a valid cast reported a definite mismatch. Each
    /// local frame carries its own (class, typedef) maps, consulted
    /// innermost-first before the globals; the stack mirrors `local_stack`
    /// (push/pop helpers + ProcessContext swaps), so process interleaving
    /// keeps each process's bindings intact.
    local_type_stack: Vec<(HashMap<String, String>, HashMap<String, String>)>,
    /// Active parameterized-class specialization for per-spec static storage:
    /// `(base_class, sig)` where `sig` is the canonical `#(...)` param text. Set
    /// while a `C#(params)::...` static method runs or a `C#(params)::prop` is
    /// accessed; `static_prop_key` folds `sig` into the key ONLY for statics
    /// declared in `base_class` (so unrelated/inherited statics stay shared),
    /// giving each specialization its own cell (§8.25). None = shared cell.
    current_spec: Option<(String, String)>,
    /// Active specialization scope CHAIN (innermost last): a parameterized
    /// `C#(params)::method()` call (`eval_call`) sets `current_spec` to the
    /// callee for the body, so a value-parameter reference in an ACTUAL
    /// argument that belongs to an ENCLOSING specialization (e.g. `string
    /// FIELD` of `uvm_utils` passed into `uvm_config_db#(uvm_object)::get`)
    /// can no longer resolve through `current_spec` alone. This stack keeps
    /// the CALLER specializations alive so value-param resolution can walk up
    /// to them (§13.5.1: args evaluate in the caller's scope).
    spec_scope_stack: Vec<(String, String)>,
    /// §28.16 / §10.3.3 fall delays by driven signal id: a private driver
    /// stage for a resolved net, otherwise the net itself.
    gate_fall_delay_by_id: HashMap<usize, u64>,
    /// `assign #(rise, fall, turnoff)` (§10.3.3): per-driver turn-off delay,
    /// used for a transition to z. Empty unless some assign used the
    /// three-delay form.
    gate_off_delay_by_id: HashMap<usize, u64>,
    /// Declaring class → property → does this property's packed range
    /// depend on a parameter? `fit_class_prop` runs on every property store,
    /// so the common answer (`false`) is memoized to one hash lookup.
    spec_prop_is_dyn: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// Memo for `respec_packed_width`: `(declaring_class, property,
    /// parameter-binding fingerprint)` → re-resolved width, so the packed
    /// range is const-eval'd once per specialization instead of per store.
    spec_prop_width_cache: std::cell::RefCell<HashMap<(String, String, String), Option<u32>>>,
    /// `static_cell_width` per static cell key (class tables are fixed at
    /// run time, and a key names one specialization).
    static_cell_width_cache: std::cell::RefCell<HashMap<String, Option<(u32, bool)>>>,
    /// Tracks class names currently being constructed via the `type_id::create()`
    /// shortcut, to prevent infinite recursion when a parameterized
    /// class's constructor or static initializers re-enter `type_id::create()`
    /// for the same class.
    type_id_create_in_progress: HashSet<String>,
    /// Tracks which `(base, sig)` specializations have already had their
    /// static-call initializers run. Per §8.25/§6.21, each parameterized-class
    /// specialization has its own set of static properties, and side-effecting
    /// static initializers (e.g. `local static bit m__initialized =
    /// __deferred_init()`) must run once per specialization — not just once
    /// for the generic class with default params.
    initialized_spec_statics: std::collections::HashSet<(String, String)>,
    /// Concrete module/package typedef references, excluding aliases hoisted
    /// from generic class definitions without an active specialization.
    referenced_class_typedefs: Vec<DataType>,
    /// Object handles currently mid-`register()` in a factory. Guards
    /// against RE-ENTRANT registration: a factory `register(obj)` whose
    /// first statement `obj.get_type_name()` triggers a parameterized
    /// class specialization's lazy static init, which itself calls
    /// `factory.register(obj)` for the SAME object. The re-entrant call is
    /// skipped so the OUTER call performs the single registration — without
    /// it, the outer call resumes and finds the type already stored, emitting
    /// a spurious UVM `TPRGED` warning.
    factory_reg_in_progress: std::collections::HashSet<usize>,
    /// (handle, class) whose constructor body is currently executing —
    /// unqualified calls on `this` bind within that class's chain (§8.7).
    ctor_class_stack: Vec<(usize, String)>,
    /// (var.prop) null-deref reads already reported (in-method soft errors).
    reported_null_derefs: std::collections::HashSet<String>,
    /// Queue of specializations pending static-init, processed iteratively
    /// by `ensure_spec_statics` to avoid deep Rust stack recursion.
    pending_spec_inits: Vec<(String, String)>,
    /// Recursion guard for `ensure_spec_statics`.
    spec_init_depth: u32,
    /// Recursion guard for evaluating call-bearing instance-property
    /// initializers at construction (see `evaluate_call_init_at_construct`).
    construct_init_depth: u32,
    /// Classes awaiting their §8.7 initialization for each constructing object.
    pending_class_initializers:
        HashMap<usize, Vec<std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>>>,
    /// Class-typed procedural locals — variable name -> class type name.
    /// Lets a later `name = new();` know which class to construct.
    var_class_types: HashMap<String, String>,
    /// Parameterized-class procedural locals — variable name -> the declared
    /// `#(...)` type-parameter args (as expressions). Lets a SEPARATE-statement
    /// `name = new(...)` (which can't see the decl's data_type) construct the
    /// right specialization, so the instance records its type bindings. This is
    /// right specialization, so the instance records its type bindings (IEEE 1800-2023 §8.25).
    /// Needed for per-spec static resolution (get_type vs get_type_handle → GET).
    var_type_args: HashMap<String, Vec<Expression>>,
    /// §15.3/§15.4: built-in container local variable → "mailbox"/"semaphore".
    /// Populated at VarDecl exec time when the declared base type is a
    /// (possibly parameterized) mailbox/semaphore, so a later separate
    /// `name = new(bound)` allocates the right container. get_expr_type_name
    /// can't serve this: for a class property it returns the enclosing class
    /// name, and for an untracked local it returns None.
    var_container_types: HashMap<String, String>,
    /// LRM §6.19.6: typedef-typed local variable → typedef name. So
    /// `<local>.next()/.first()/.last()/.num()/.prev()` resolves the
    /// enum-member list via `module.enum_members[typedef]`. Mirrors
    /// `var_class_types`; populated at VarDecl exec time when the
    /// data type is a `TypeReference` whose name is in `module.typedefs`.
    var_typedef_types: HashMap<String, String>,
    /// The declaration whose name-keyed metadata is installed under each
    /// frame-local's name (an index into `local_meta_origins`, interned
    /// through `local_meta_origin_ids`), and the metadata of the declarations
    /// another one displaced, by name and declaration (see `LocalMeta`). A
    /// name with a cache entry is contested: only its claims are checked when
    /// a process runs again, and a swap moves just that name's own entries.
    local_meta_owner: HashMap<String, u32>,
    local_meta_origins: Vec<MetaOrigin>,
    local_meta_origin_ids: HashMap<MetaOrigin, u32>,
    local_meta_cache: HashMap<String, Vec<(u32, LocalMeta)>>,
    /// The frame-local declarations the running process's live frames
    /// executed, outermost first (see `MetaClaim`); part of its context.
    local_meta_claims: Vec<MetaClaim>,
    /// Names of `string`-typed variables (locals, params, package globals) so
    /// `{a, b}` concatenation involving them is treated as string concat.
    string_signals: HashSet<String>,
    /// Per-call-frame save area for queue/dynamic-array LOCALS. Queue storage
    /// lives globally in `self.signals` (`name.size`, `name[i]`) keyed by bare
    /// name, so a nested method whose local/param queue shares a name with a
    /// caller's local would clobber it. On entry to a method/function/task we
    /// push a frame; whenever a queue param is bound or a queue local is
    /// (re)declared, we snapshot the caller's `name.*`/`name[*]` signals into
    /// the top frame; on exit we restore them.
    queue_frame_saves: Vec<Vec<(String, QueueLocalSave)>>,
    /// Per-call-frame rename map for local dynamic arrays/queues/assoc
    /// locals (bare name -> process-unique storage key). See
    /// `ProcessContext::local_dyn`.
    local_dyn: Vec<Vec<(String, String)>>,
    /// Parallel to `local_dyn`: each call frame's queue / dynamic-array /
    /// associative FORMALS, bare name -> storage key (a per-call copy
    /// `@q#N`, or for `ref` the actual's own storage). The frame's
    /// `local_dyn` table holds an identity entry `(name, name)` for each, so
    /// it shadows an enclosing same-named local and reads as its bare name
    /// from a nested call (§13.3); `dyn_name_lookup` maps the identity hit
    /// in the innermost frame through this table.
    formal_dyn: Vec<Vec<(String, String)>>,
    /// Monotonic counter backing the process-unique keys in `local_dyn`.
    next_dyn_id: u64,
    /// Built-in mailboxes (handle -> queue of values)
    mailboxes: HashMap<usize, std::collections::VecDeque<Value>>,
    /// §15.4 aggregate messages. A queue slot holds one `Value`, which cannot
    /// carry an UNPACKED struct — its members are separate leaf signals. Such
    /// a message is copied member-wise into a shadow variable and the slot
    /// holds the shadow's id instead. Ids are recycled as messages are
    /// consumed, so the shadows are bounded by the deepest the box ever got
    /// rather than growing with every message put.
    mbx_msg_seq: u64,
    mbx_free_ids: Vec<u64>,
    /// A message `put` as an assignment pattern (`m.put('{a: 1, b: 2})`),
    /// keyed by the tagged token queued in its place. The box does not know
    /// its element type, so the pattern is kept with every item already
    /// evaluated and assigned to the `get`/`peek` destination on delivery.
    mbx_patterns: HashMap<u64, Vec<AssignmentPatternItem>>,
    /// §15.4.1 bounded-mailbox capacity (handle -> bound). Absent or 0 means
    /// UNBOUNDED — `try_put` never fails and `put` never blocks. `new(N)` with
    /// N>0 records N here; a full box rejects `try_put` and parks `put`.
    mailbox_bound: HashMap<usize, usize>,
    /// Producers parked on `put` to a full bounded mailbox (handle -> queue).
    mailbox_put_waiters: HashMap<usize, std::collections::VecDeque<MailboxPutWaiter>>,
    /// §18.13 rand_mode: per-instance set of rand properties whose
    /// randomization is currently disabled. `obj.x.rand_mode(0)` adds "x".
    /// `obj.rand_mode(0)` disables ALL rand props.
    rand_mode_disabled: HashMap<usize, HashSet<String>>,
    /// §18.13 constraint_mode: per-instance set of constraint names that
    /// are currently disabled. `obj.cb.constraint_mode(0)` adds "cb".
    constraint_mode_disabled: HashMap<usize, HashSet<String>>,
    /// §18.5.9/§18.6 recursion guard: depth of nested `randomize()` calls made
    /// for `rand` class-handle members (a cyclic object graph would otherwise
    /// recurse forever).
    randomize_depth: usize,
    /// §18.5.9: the objects of the randomize() call being solved, outermost
    /// first: the object `randomize()` was called on, then each rand
    /// sub-object whose own solve is running under it. A sub-object's
    /// constraint that reads a rand variable of an object earlier in the
    /// chain through a handle member (`item.addr`, `item` the handle back
    /// to the enclosing object) belongs to that object's solve.
    rand_solve_chain: Vec<usize>,
    /// Set by `randomize_nested_with` for the solve it starts, which then
    /// extends `rand_solve_chain` instead of starting a chain of its own.
    rand_solve_nested: bool,
    /// §18.6.2: the sub-objects of the running call whose `post_randomize`
    /// waits for the whole call to succeed. A sub-object with an item left
    /// to an enclosing solve (see `rand_solve_chain`) is done before the
    /// variables that item constrains have their final values, which its
    /// `post_randomize` may read (`hist.push_back(item.addr)`).
    rand_post_deferred: Vec<usize>,
    /// The solve in progress left an item to an enclosing solve.
    rand_post_defer_self: bool,
    /// Failed randomize() calls reported so far, per class or
    /// `std::randomize` call site (`rand_fail_warn`).
    rand_fail_reports: HashMap<String, u32>,
    /// Whether the `XEZIM_RAND_DIAG` hint has been printed.
    rand_fail_hinted: bool,
    /// One work/time budget shared by an outer randomize call and every
    /// recursive rand-object solve it starts.
    randomize_budget_depth: usize,
    randomize_budget_remaining: u64,
    #[serde(skip)]
    randomize_budget_deadline: Option<std::time::Instant>,
    randomize_budget_exhausted: bool,
    /// Active enclosing solves retain only their reachable children's rand state.
    rand_child_snapshots: Vec<rand_scope::RandChildTransaction>,
    /// IEEE 1800-2017 §18.5.11 — a `static constraint` block is SHARED by every
    /// instance of the class: `constraint_mode()` on it through ANY instance
    /// (or through the class scope) sets one class-wide flag. Keyed by
    /// (declaring class, constraint name); membership == disabled.
    static_constraint_disabled: HashSet<(String, String)>,
    /// §18.4.3 randc cycles: per-(instance, prop) set of values already
    /// drawn in the current cycle. Each randomize call must pick a value
    /// not in this set; once every value in the prop's range has been
    /// drawn the set resets and a new cycle begins.
    randc_used: HashMap<(usize, String), HashSet<u64>>,
    /// §18.4 randc: values drawn TENTATIVELY by the randomize() call that is
    /// currently in flight, keyed like `randc_used`. The solver retries a
    /// failed solution up to 1000 times; without this each retry would burn
    /// another value out of the permutation cycle, so a 2-bit randc could
    /// visit only two of its four values across four randomize() calls.
    /// A pick is rolled back (removed from `randc_used`) when its trial is
    /// retried or abandoned, so exactly ONE value leaves the cycle per
    /// SUCCESSFUL randomize(). `Some(prev)` records the pre-reset used-set of
    /// a pick that wrapped the cycle, so the rollback can restore it.
    randc_pending: HashMap<(usize, String), (u64, Option<HashSet<u64>>)>,
    /// §18.4.2 randc wider than 16 bits (to 32): the cycle is a keyed
    /// bijection over `[0, 2^w)` walked by a counter, so it needs no set of
    /// drawn values. Per (instance, prop): (key, next index, width).
    randc_wide: HashMap<(usize, String), (u64, u64, u32)>,
    /// The `randc_wide` state before this call's tentative pick, for
    /// `randc_rollback` (None: the variable had no cycle yet).
    randc_wide_pending: HashMap<(usize, String), Option<(u64, u64, u32)>>,
    /// §18.5.4/§18.10 dist schedules, keyed by (instance handle, variable):
    /// one (weight signature, item schedule, next slot) per weight set. See
    /// `pick_dist_value`.
    dist_decks: HashMap<(usize, String), Vec<(u64, Vec<usize>, usize)>>,
    /// Built-in semaphores (handle -> current count)
    semaphores: HashMap<usize, i64>,
    /// Covergroup instance heap (index 0 is null).
    cg_heap: Vec<Option<CovergroupInstance>>,
    /// Running tallies for every distinct `assert`/`assume`/`cover` source
    /// location. Keyed by the statement's `span.start` byte offset so a
    /// single source line aggregates across all dynamic executions.
    /// Emitted at end-of-sim under `[COV] assertion summary` + (when
    /// `XEZIM_COV_DB` is set) the JSON coverage database file.
    assertion_stats: HashMap<usize, AssertionStat>,
    /// Statement label of a labelled assertion site (`ap: assert property`,
    /// §16.5, §16.14), keyed like `assertion_stats`. Filled once when the
    /// site registers; read only by the coverage DB writer.
    assertion_labels: HashMap<usize, String>,
    /// §9.3.1: the name of a named block whose only statement is the
    /// assertion about to execute. The parser hosts a procedural statement
    /// label (`a_imm: assert (x);`) in such a block, and the bytecode
    /// compiler's statement fallback re-wraps an assertion in its enclosing
    /// named blocks the same way, so the block executor sets this and the
    /// assertion takes it as its label.
    assert_wrap_label: Option<String>,
    /// Code coverage sites and toggle state (`--code-coverage`); `None` when
    /// the run collects none. See `code_cov`.
    code_cov: Option<Box<code_cov::CodeCov>>,
    /// Code coverage counters, indexed by the id a `$__xz_cov` marker or an
    /// `Insn::CovHit` carries. Empty when the run collects none.
    code_cov_hits: Vec<u64>,
    /// Toggle coverage candidates, named while `new` still has the
    /// elaboration signal table; resolved to ids in `compile`.
    code_cov_toggle_names: Vec<String>,
    /// Call stack for tracking 'this' and local variables.
    this_stack: Vec<Option<usize>>,
    local_stack: Vec<HashMap<String, Value>>,
    /// Open block scopes; see `ProcessContext::block_scope_saves`.
    block_scope_saves: Vec<BlockScopeSave>,
    /// `(body address, frame generation)` of the synchronous loop body whose
    /// block scope `loop_scope_open` holds open across iterations.
    hoisted_scope: (usize, u64),
    /// Pids whose continuation `continue_stmts_or_trampoline` rescheduled
    /// with their context left live on the simulator.
    trampolined_pids: HashSet<usize>,
    /// §16.4.2: pids whose next `run_process_stmts_inner` entry continues the
    /// running activation (`continue_stmts_or_trampoline`), not a resume
    /// after a suspension: their pending deferred reports and procedural
    /// assertion instances stay queued.
    #[serde(skip)]
    cont_pids: HashSet<usize>,
    /// Pids parked on a delay (`#d`, `#0`, an intra-assignment delay), each
    /// with whether the delay moves time on (`#0` does not). Checkpointed,
    /// with the queued continuations.
    delay_pids: HashMap<usize, bool>,
    /// §16.14.6: pids whose running activation went on after a delay that
    /// moved time on, not at an event control: a procedural concurrent
    /// assertion they reach first is not at its clock's tick, so its first
    /// attempt waits for the next one. A `#0` stays in the time step of the
    /// event that woke the procedure.
    delay_resumed: HashSet<usize>,
    /// Parallel to `local_stack`: the frame generation each slot was pushed
    /// with (see `ProcessContext::local_gen_stack`). Kept on the active
    /// `Simulator` exactly like `local_stack`, and moved into/out of a
    /// `ProcessContext` by snapshot/take/restore.
    local_gen_stack: Vec<u64>,
    /// Context for 'super' resolution: stack of (current_class_name).
    class_context_stack: Vec<Option<String>>,
    /// For each class-method call entered via `exec_method_in_class_hierarchy`,
    /// the `local_stack` depth at entry (before the method's own frame is
    /// pushed). Used by `get_expr_type_name` to decide whether a bare name is
    /// a local of the CURRENT method vs a stale flat-map entry leaked from a
    /// caller's scope — without this, `var_class_types` (never cleared on
    /// method exit) lets a caller's same-named local shadow a class property's
    /// declared type, so `prop = new(...)` constructs the wrong class (and can
    /// recurse infinitely when the wrong class is the enclosing class itself).
    method_local_base: Vec<usize>,
    /// Immutable method bodies cached after their first dispatch. Runtime
    /// execution mutates object state, never declarations, so sharing the
    /// parsed body avoids rebuilding a deep AST on every call.
    class_method_cache: std::cell::RefCell<
        HashMap<String, HashMap<String, Option<Arc<crate::ast::decl::ClassMethod>>>>,
    >,
    /// class-perf Step 7: memoized `class_has_method` (whole-chain,
    /// declaration-presence only — includes PureVirtual/Extern prototypes,
    /// unlike `cached_class_method`). Same freeze argument as the
    /// defining-class memo (`method_defining_class`).
    class_has_method_memo: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// class-perf Step 7: memoized `is_static_method` (whole-chain — an
    /// ancestor's static declaration wins, exactly like the walk it
    /// replaces).
    is_static_method_memo: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// Current covergroup instance if in sampling context.
    cg_this: Option<usize>,
    /// Processes waiting for join
    join_waiters: Vec<JoinWaiter>,
    /// Map from child PID -> parent PID
    process_parents: HashMap<usize, usize>,
    /// Processes handed up to a grandparent in `process_parents` because
    /// their own parent finished. They stay its descendants for `disable
    /// fork` (§9.6.3), but are not its immediate children for `wait fork`
    /// (§9.6.1).
    adopted_pids: HashSet<usize>,
    /// Per-process execution context for scheduled class/task processes.
    process_contexts: HashMap<usize, ProcessContext>,
    /// Instance scope (e.g. `"TB.p1"`) for a scheduled process, set from the
    /// originating initial block. `run_scheduled_process` installs it as the
    /// name-resolution hint so AST-evaluated bare names in a multiply-
    /// instantiated module resolve to THIS instance's signals.
    process_scope_hint: HashMap<usize, String>,
    /// §21.2.1.7 `%m`: each process's own lexical scope chain, parked here
    /// while it is suspended. `m_scope_stack` is a single global stack, so a
    /// task that suspends at a `#delay` used to leave its own name installed
    /// for whatever process ran next — every later `%m`, including a
    /// `$monitor` arming in an unrelated instance, inherited that name.
    process_m_scope: HashMap<usize, Vec<String>>,
    /// §21.2.1.7 `%m`: label of a named `initial begin : lbl` / `always
    /// begin : lbl`, per pid. The scheduler FLATTENS a process's outermost
    /// `begin`/`end` into its statement stream (see `run_process_stmts`), so
    /// the `SeqBlock` arm that would push the label onto `m_scope_stack`
    /// never runs for it and `%m` dropped the block name — `top.u_a` where
    /// it should read `top.u_a.lbl`. Nested (non-flattened) blocks still go
    /// through `m_scope_stack` as usual.
    process_m_label: HashMap<usize, String>,
    /// Where each process came from: the creating construct's source span
    /// and its kind ("initial block", "always block", "fork child", …).
    /// Used by stall/waiter reports, X warnings, value and scheduler traces.
    /// Terminated fork children release entries after their last diagnostic
    /// window and native stack unwind, unless deferred diagnostics or a
    /// checkpoint restart can still need them.
    process_origin: HashMap<usize, (crate::ast::Span, &'static str)>,
    /// Ordinary deferrals, indexed by this window's stall hits or still on
    /// a scheduler stack, need only individual lookups at a safe reset.
    #[serde(skip)]
    retired_fork_pids: Vec<usize>,
    /// A native-guard decline needs one sweep at the next safe reset.
    /// Keep no per-child backlog while imported/exported DPI tasks are live.
    #[serde(skip)]
    retired_fork_sweep_needed: bool,
    #[cfg(test)]
    #[serde(skip)]
    retired_fork_reset_visits: usize,
    /// XEZIM_TRACE_SCHED=1|all|<t>|<t0>:<t1> — an ORDERED log of what runs
    /// inside each time slot: the tick header, every clock-generator toggle,
    /// and every process as it is popped from the timing wheel. A race is a
    /// question about intra-timestamp ORDER, which the value/edge traces
    /// cannot answer — they say what changed, never who went first. Windowed
    /// by time because a whole run is unreadable.
    trace_sched: Option<(u64, u64)>,
    /// Instance scope of the edge block currently executing (set around
    /// dispatch, cleared after). `current_module_def` prefers it over the
    /// per-pid hint so $time/%t scale to the BLOCK's module timescale, not
    /// the last scheduled process's.
    timescale_scope_override: Option<String>,
    /// Collection name a `return <collection>` recorded (§13.4: functions may
    /// return arrays/queues). The caller's assign-from-call consumes it with
    /// a whole-collection copy; a plain value return leaves it None.
    pending_ret_collection: Option<String>,
    /// The collection the last compiled call instruction got back through
    /// `pending_ret_collection` (None for a plain value return). Only the
    /// `AssignPendingColl` that may follow the call reads it.
    vm_ret_coll: Option<String>,
    /// Flat element values a compiled collection-returning call produced
    /// (`Insn::RetCollection`), stashed straight out of the callee's register
    /// file. `Insn::AssignPendingColl` prefers these over the string-keyed
    /// `vm_ret_coll`/`__xz_ret_coll__` snapshot+copy round trip, which
    /// allocates a formatted key, a HashMap insert, a class-legacy scan and
    /// per-element lookups per returned element. `None` = the AST/string path
    /// produced the return and the historic signal-snapshot copy applies.
    vm_ret_vals: Option<Vec<Value>>,
    /// Innermost-first stack of whether each currently-active function returns
    /// a collECTION (queue / unsized dynamic array). The `Return` statement
    /// reads the top to decide whether a LITERAL return value (`return '{...}`,
    /// which is not an Ident, so the bare-name snapshot path can't fire) must be
    /// materialised into a whole-collection snapshot and handed to the caller
    /// through `pending_ret_collection`. Without it a function whose `return`
    /// expression is a queue literal collapsed to a single-element scalar
    /// value when the caller assigned it to a queue variable — an EMPTY `'{}`
    /// returned size 1 (one phantom element), and `'{a,b}` kept only `a`.
    fn_ret_collection_stack: Vec<bool>,
    /// Parallel to `fn_ret_collection_stack`: the running function returns
    /// an associative array (§13.4).
    fn_ret_assoc_stack: Vec<bool>,
    /// Parallel to `fn_ret_collection_stack`: the declared width of an
    /// integral return, the context `return e` evaluates `e` in (§13.4.1:
    /// as if assigned to the return variable); 0 for other types.
    fn_ret_ctx_stack: Vec<u32>,
    /// Parallel to `fn_ret_ctx_stack`: the running subroutine, for the type
    /// `return '{...}` takes (§10.8, §13.4.1). Empty whenever no function is
    /// running, which is every checkpoint.
    #[serde(skip)]
    fn_ret_frames: Vec<ReturnPatternType>,
    /// Shared return types of free functions, by return-type span then
    /// function name, so a call pushes its frame without allocating.
    #[serde(skip)]
    fn_ret_types: HashMap<(usize, usize), Vec<(String, std::rc::Rc<DataType>)>>,
    /// Stable instance scope of the process currently running (for `%m`).
    /// Unlike `name_resolve_hint`, this is not mutated by name resolution.
    current_scope: String,
    /// Recycled scope-hint strings for the process wake-up path: every
    /// wake-up installed a fresh clone of the process scope into
    /// `name_resolve_hint` and `current_scope` (four allocations for a
    /// compiled-FSM wake-up) and freed them on exit.
    hint_string_pool: Vec<String>,
    /// Gated-clock conversion (cycle design phase 0). A derived clock is a
    /// fused-gate output (ICG `clk & en`, buffer, inverter) whose input
    /// clock chains back to a clock GENERATOR. Those gate entries form the
    /// clock tree: at a generator toggle they are evaluated EAGERLY in
    /// topological order and any derived clock that toggled joins the same
    /// edge pass, so the tree never goes through the settle worklist
    /// (c906: 666 always-active entries, 38% of all comb evaluations, were
    /// the clock tree). `trigger_deps` skips a tree entry when the written
    /// signal is a tree clock; an ENABLE change still reaches it through
    /// the enable's own dependents.
    /// `XEZIM_CYCLE_MODE`: `event` (the event-driven engine, default) or
    /// `cycle`. Everything under the cycle design (the eager clock tree,
    /// later the cycle scheduler and its delta-cycle fallback) is gated on
    /// this so `event` is exactly the engine as it was.
    cycle_mode: bool,
    clock_tree_by_root: HashMap<usize, Vec<usize>>,
    is_clock_tree_entry: Vec<bool>,
    /// Gate lane (see `build_gate_lane`): per comb entry, the index of its
    /// compact gate op when the entry is a fused single-bit gate whose
    /// output no comb entry reads (c906: 47% of all settle evaluations),
    /// else `u32::MAX`. Such gates are queued when triggered and evaluated
    /// in one tight loop at the end of the pass instead of through the
    /// generic entry path.
    gate_lane: Vec<u32>,
    gate_ops: Vec<FusedGate>,
    /// Per lane op: bit0 = destination is an armed edge-block input, bit1 =
    /// destination is edge-sensitive. Decides which write notes the fast
    /// commit must make.
    gate_obs: Vec<u8>,
    gate_queue: Vec<u32>,
    gate_queued: Vec<bool>,
    gate_lane_valid: bool,
    /// Paths of instances whose definition is an interface (see
    /// `is_interface_instance`); filled on first use.
    #[serde(skip)]
    iface_instance_paths: std::cell::OnceCell<std::collections::HashSet<String>>,
    /// Interface instance path by its virtual-interface handle token (see
    /// `iface_handle_token`); filled on first use.
    #[serde(skip)]
    iface_by_token: std::cell::OnceCell<HashMap<u64, String>>,
    /// See `iface_path_parts`.
    #[allow(clippy::type_complexity)]
    #[serde(skip)]
    iface_path_parts: std::cell::OnceCell<(HashSet<String>, HashSet<String>)>,
    /// Quiet signals: neither an armed edge-block input nor edge-sensitive,
    /// so a write to one has no side-table work at all. One bit per signal;
    /// rebuilt when either table changes. A quiet store skips the write
    /// observer with one load (c906: most two-state stores are internal
    /// nets).
    quiet_bits: Vec<u64>,
    quiet_valid: bool,
    prof_quiet_stores: u64,
    prof_observed_stores: u64,
    /// Native two-state streams (XEZIM_TS_JIT=1, jit feature): per comb
    /// entry, the compiled straight-line function, consulted before the
    /// arena executor.
    #[cfg(feature = "jit")]
    #[serde(skip)]
    ts_jit: Option<Box<super::ts_jit::enabled::TsJitModule>>,
    #[cfg(feature = "jit")]
    #[serde(skip)]
    ts_jit_fns: Vec<Option<super::ts_jit::enabled::TsJitFn>>,
    #[cfg(feature = "jit")]
    ts_jit_on: bool,
    /// Per comb entry: two-state evaluations so far (native path only);
    /// a stream is compiled once it crosses `ts_jit_hot`, so only the hot
    /// set turns into machine code and the code stays cache-resident.
    #[cfg(feature = "jit")]
    ts_jit_evals: Vec<u32>,
    #[cfg(feature = "jit")]
    ts_jit_hot: u32,
    is_clock_tree_signal: Vec<bool>,
    prof_clock_tree_evals: u64,
    /// Instance scope of the comb/edge BLOCK currently being evaluated, for
    /// `%m` only — deliberately separate from `current_scope`, which also
    /// steers name resolution (`resolve_hier_name`) and must not start
    /// tracking the settle loop.
    ///
    /// §21.2.1.7: `%m` is the hierarchical name of the scope holding the
    /// format string. `current_scope` is installed by `run_process`, which
    /// only ever runs `initial`/`fork`/delay-`always` processes — a
    /// sensitivity-driven block (`always_comb`, `always @(edge)`) is a
    /// `CombEntry` or a compiled edge block and never passes through there.
    /// So `%m` in the body of any inlined `always_comb` reported the bare TOP
    /// module name, identically for every instance, which makes per-instance
    /// log correlation in a multiply-instantiated design impossible.
    ///
    /// Updated in place (`clear` + `push_str`) rather than assigned, so the
    /// settle hot loop reuses one allocation and pays only a short memcmp.
    m_block_scope: String,
    /// A comb/edge block (not a process) is executing, so `%m` names
    /// `m_block_scope` — empty for a block of the root — and no process
    /// block label. Cleared by `run_process`.
    m_block_active: bool,
    /// Interned scope id per edge block, parallel to `edge_blocks`. Two blocks
    /// in the same instance share an id, so the `%m` bookkeeping in
    /// `exec_bytecode` is a `u32` compare against a contiguous array instead
    /// of a `String` compare that also had to deref the `edge_blocks` `Arc`
    /// and reach into a 3607-entry struct vector. That comparison runs on
    /// every one of ~189 M block fires and measured 3.5% of retired
    /// instructions on the c906 memcpy workload.
    edge_block_scope_id: Vec<u32>,
    /// Which interned scope `m_block_scope` currently holds, or `u32::MAX`
    /// when its contents did not come from `edge_block_scope_id` (any
    /// non-edge-block writer sets it back to `MAX`, so the next block fire
    /// re-syncs rather than trusting a stale id).
    m_block_scope_id: u32,
    /// Stack of the names of the functions/tasks currently executing (innermost
    /// last), so `%m` inside a package subroutine resolves to `<pkg>.<name>`
    /// (matching real simulators) rather than the top-module name. Pushed in
    /// `exec_function_call` / `exec_task_call` around the body and popped after.
    func_call_stack: Vec<String>,
    /// §26.3: the package whose subroutine body is running, innermost last
    /// (`None` for a module-level subroutine). A package subroutine resolves
    /// its free names in ITS OWN package, but package-scope declarations are
    /// hoisted into the shared tables under their bare name — last writer wins
    /// — so a name two packages both declare has to be read back through
    /// `<pkg>::<name>`. See `pkg_ambiguous_names` and `resolve_hier_name`.
    pkg_scope_stack: Vec<Option<String>>,
    /// Package scope for the NEXT `exec_function_call` / `exec_task_call`, set
    /// by a scoped `pkg::f(...)` dispatch (which is the only site that knows
    /// which package was named). Taken on entry, before argument evaluation, so
    /// a call nested in an argument cannot inherit it.
    pending_pkg_scope: Option<String>,
    /// §26.3: package-scope names that MORE THAN ONE package declares — the
    /// only ones whose bare hoisted entry is ambiguous. Empty for a design
    /// without such a collision, which short-circuits the qualified lookup.
    pkg_ambiguous_names: HashSet<String>,
    /// §26.3: every package enum member as `pkg::member` -> (value, width),
    /// for the bytecode compiler. The inliner spells a member two packages
    /// both declare as `pkg.member`; compiled blocks used to resolve that to
    /// the bare signal twin (last package registered wins), so CVA6's `alu`
    /// compared against `fpnew_pkg::ADD` (2) instead of `ariane_pkg::ADD` (0)
    /// and never took the ADD arm.
    pkg_enum_consts: HashMap<String, (u64, u32)>,
    /// §26.3: the package VARIABLE names among `pkg_ambiguous_names` (see
    /// `split_contested_package_vars`): only these resolve through a design
    /// unit's imports or a package class's package.
    contested_pkg_vars: HashSet<String>,
    /// §21.2.1.7 `%m` lexical-scope hierarchy WITHIN the current instance:
    /// task / function / named-block / fork-block names, innermost last.
    /// Pushed on subroutine/named-block entry, popped on exit.
    m_scope_stack: Vec<String>,
    /// IEEE 1800-2017 §6.21 / §13.3.1: `static` variables declared inside a
    /// (possibly automatic) subroutine keep ONE persistent storage across all
    /// runtime calls, and their declaration initializer runs only once. Keyed
    /// `"<subroutine>::<var>"`. Only scalar integral/real statics are tracked;
    /// arrays/queues fall back to per-call storage.
    static_local_vars: HashMap<String, Value>,
    /// Bare names of `static` subroutine locals that are scalar (tracked in
    /// `static_local_vars`). Used as a CHEAP gate before the per-subroutine
    /// `static_local_keys` check, so a bare identifier that is never a static
    /// anywhere skips the key-building cost. (`static_local_vars` cells are
    /// still read/written route to each cell's shared value only when the
    /// SUBROUTINE-PRECISE key below also matches.)
    static_local_names: HashSet<String>,
    /// Module/package subroutine statics allocated before procedural code.
    hoisted_sub_static_names: HashSet<String>,
    hoisted_static_subroutines: HashSet<String>,
    /// First-byte membership filter over `static_local_names`. The eval
    /// preamble tests EVERY bare identifier against that set, so a designs
    /// with any static subroutine local paid a string hash per expression
    /// node; this rejects the overwhelming majority with one array load.
    #[serde(with = "serde_big_array")]
    static_local_first: [bool; 256],
    /// Persistent-cell keys ("`<cls>::<sub>::<nm>`"/"`<sub>::<nm>`") of all
    /// scalar `static` subroutine locals. A name into `static_local_names`
    /// only routes to the live shared cell when this key — which embeds the
    /// declaring subroutine/class/specialization — matches, so a DIFFERENT
    /// subroutine's same-named (plain/automatic) local is never wrongly
    /// shared.
    static_local_keys: HashSet<String>,
    /// True while evaluating elaboration-time constant (local)parameter
    /// initializers that call user functions. Each such constant-function call
    /// is independent (§13.4.3), so `static` locals must NOT share the runtime
    /// persistent store during this window.
    in_const_param_eval: bool,
    /// Per-subroutine-call stack of `(local_name, persistent_key)` recording
    /// which locals in the current call are `static` so their final values are
    /// written back to `static_local_vars` on return. The `String` head is the
    /// subroutine name (used to build the persistent key).
    static_local_syncs: Vec<(String, Vec<(String, String)>)>,
    /// Return value from last function call.
    return_value: Option<Value>,
    /// "string_typed-name" flag snapshots for each active subroutine frame
    /// (free function / method / task). `string_signals` is a flat global set
    /// keyed by bare name; a nested frame's NON-STRING local (e.g. `int m`)
    /// runs `string_signals.remove("m")` (the block-local decl clears stale
    /// string flags), which would permanently clobber an ENCLOSING scope's
    /// `string m` — so a later `m = <long string>` there is no longer treated
    /// as a string and gets resized to the 1024-bit placeholder (128 chars).
    /// Snapshot at frame entry, restore at frame exit, so a
    /// frame can never leak an add OR a remove to its caller.
    string_signals_removed: Vec<Vec<String>>,
    /// §6.21/§13.5/§6.16: string classifications of task formals and scalar
    /// locals, keyed by their frame generation (`local_gen_stack`).
    /// A task can suspend, so its formals must not enter the global
    /// `string_signals` set: while it waits, or after `disable fork` kills
    /// it, another subroutine's same-named packed formal would read as a
    /// string and lose its bit writes. Per-process (moved with
    /// `ProcessContext`), and the frame's non-string formals override a
    /// global string flag of the same name; see `frame_name_is_string`.
    task_string_formals: Vec<TaskFrameStrings>,
    /// §13.4/§6.21: width, signedness and declared-type tables are keyed by
    /// bare name, so a callee's local (`logic [7:0] r;`) overwrote the
    /// entries of a caller's same-named variable, which then kept the
    /// callee's type after the return — a signed `int r` read back unsigned
    /// or truncated, including through an `inout`/`output` copy-back. Each
    /// function, class-method and synchronous task activation logs what its
    /// declarations displaced and puts it back on exit.
    decl_shadow_log: Vec<DeclShadowFrame>,
    /// Bumped whenever other processes may run inside a subroutine call (the
    /// nested scheduler of a synchronous `#delay`). A shadow frame whose
    /// activation saw that happen is dropped, not restored: an interleaved
    /// activation may own the entries by then.
    nested_run_epoch: u64,
    /// Union of every parked `wait(cond)` waiter's read names, rebuilt lazily
    /// (see `cond_read_union_dirty`) so a blocking write can reject in O(1)
    /// without allocating — `check_condition_waiters_for_write` sits on the
    /// interpreter's hottest path.
    cond_read_union: HashSet<String>,
    cond_read_union_dirty: bool,
    /// Static-collection bare names declared by two SIBLING classes (§8.9),
    /// computed once at startup (`colliding_names` in the static-collection
    /// registration loop). `static_coll_name_collides` used to rescan every
    /// class on every static-collection access.
    colliding_static_colls: HashSet<String>,
    colliding_static_colls_ready: bool,
    /// Default random number generator — the stream used by any object or
    /// process that has not been given a private one (§18.14).
    rng: SvRng,
    /// §18.14.1 per-PROCESS random stream. Populated lazily: a process only
    /// gets a private stream once `process::self().srandom()` /
    /// `.set_randstate()` / `.get_randstate()` is called on it. Until then it
    /// draws from `rng`, preserving the historical (entropy) behaviour.
    /// Keyed by pid, so forked threads are independent of each other and of
    /// their parent — their `$urandom` sequence no longer depends on the
    /// scheduling order of the other threads.
    proc_rng: HashMap<usize, SvRng>,
    /// §18.14.1/§18.14.2 per-OBJECT random stream, keyed by class handle.
    /// Populated lazily by `obj.srandom(seed)` / `obj.set_randstate(s)` /
    /// `obj.get_randstate()`; `randomize()` on such an object draws from it, so
    /// the object's sequence is unaffected by any thread reseeding and vice
    /// versa.
    obj_rng: HashMap<usize, SvRng>,
    /// Handles of the objects whose `randomize()` is currently executing
    /// (a stack — a constraint/post_randomize may randomize another object).
    /// `cur_rng` consults the top of this stack first.
    obj_rng_stack: Vec<usize>,
    /// §18.7 — the receiver NAME (a bare handle variable) of the in-flight
    /// `obj.randomize() with {…}` call. Inside the randomized frame that
    /// local is out of scope (eval of it returns 0), so the member-forcing
    /// solver recognises `recv.member` as the object's own `member` when
    /// `recv` is this receiver AND `member` is a rand property.
    rand_receiver: Option<String>,
    /// `method_local_base` entry of the solve frame `rand_receiver` belongs
    /// to; see `active_rand_receiver`.
    rand_receiver_base: usize,
    /// §8.10: property names declared by MORE than one class of some object's
    /// inheritance chain. Non-leaf declarers' copies are stored under
    /// `"<Class>::<name>"`; the set is the cheap gate that keeps every
    /// unshadowed access on the bare-name fast path.
    shadowed_prop_names: HashSet<String>,
    /// `meta_key_bits` of every name in `shadowed_prop_names`: a prefilter that
    /// spares the hot member-access paths a hash per access.
    shadowed_prop_mask: u64,
    /// §18.5/§8.14: the class that declares each constraint block of the
    /// randomize() call in progress, keyed by the address of the block's item
    /// list, for the blocks of a base class of the object when some property
    /// name is shadowed. Their names resolve in that class, not the object's.
    /// Empty outside a solve and whenever no name is shadowed.
    constraint_scopes: HashMap<usize, String>,
    /// §18.7/§8.14: the declared class of the receiver of the
    /// `obj.randomize() with {…}` call about to solve, when it is a base of
    /// the object's class and some name is shadowed: the inline block's names
    /// resolve in it. Taken by the solve that gathers the block.
    inline_constraint_scope: Option<String>,
    /// §6.11.1 / §7.5: instance-scoped storage names (`<handle>#<member>`) of
    /// queue / dynamic-array class properties whose element type is a SIGNED
    /// integral type (`byte`, `shortint`, `logic signed [7:0]`, ...). Their
    /// elements live in the runtime map with no declared signedness of their
    /// own, so a store stamps it from here (`p.dyn[1] = 8'hff` reads -1).
    signed_class_colls: HashSet<String>,
    /// Cache of `fit_coll_store_elem`, replaced round-robin from
    /// `coll_store_fit_next`: (collection store name, the owning object's
    /// class, `integral_queue_elem` of it).
    #[allow(clippy::type_complexity)]
    coll_store_fit_cache: [Option<(String, Option<String>, Option<(u32, bool, bool)>)>; 4],
    coll_store_fit_next: usize,
    /// §8.25 / §8.3: classes whose base class depends on their own
    /// parameters — `class W #(type B = base) extends B;` (the base IS a type
    /// parameter) or `class W2 #(type B) extends W #(B);` (the base is such a
    /// class specialized with this class's parameters). Maps the class to its
    /// DECLARED `extends` name and argument fragments, before linking
    /// rewrote them. Each specialization whose base CLASS differs from the
    /// default's gets its own class entry (see `ensure_spec_class`).
    dep_base: HashMap<String, (String, Vec<String>)>,
    /// The default specialization's base class ENTRY of each `dep_base`
    /// class; the class entry itself carries this base. A specialization
    /// binding a base of another class gets its own entry; one binding the
    /// same base class with other arguments shares this one, the arguments
    /// flowing through the specialization (see `spec_projection`).
    dep_default: HashMap<String, String>,
    /// The `dep_base` keys as a short list: the hot paths (every `new`,
    /// `$cast`, static key and type-parameter lookup) ask whether a class is
    /// one, and a design has a handful at most.
    dep_names: Vec<String>,
    /// `spec_entry` answers for `dep_base` classes, keyed `"<base>\0<sig>"`.
    spec_entry_cache: std::cell::RefCell<HashMap<String, String>>,
    /// Specialized class entry name -> the class it was specialized from
    /// (`"wrap_c<derived_c>"` -> `"wrap_c"`).
    spec_clone_origin: HashMap<String, String>,
    /// Typedef names whose target is a specialization of a `dep_base` class
    /// (`typedef wrap_c #(derived_c) wd_t;`): `$cast` checks a destination
    /// declared through one against that specialization.
    dep_typedefs: HashSet<String>,
    /// Scratch key for the property-initializer loop of instantiation.
    inst_key_scratch: String,
    /// Instantiation templates by leaf class name (see `InstTemplate`).
    inst_templates: std::cell::RefCell<HashMap<String, std::rc::Rc<InstTemplate>>>,
    /// §18.11: when `obj.randomize(a, b)` names a MEMBER SUBSET, only those
    /// properties are solved; every other rand member keeps its current value
    /// and acts as state. Empty/None means the ordinary whole-object form.
    randomize_subset: Option<HashSet<String>>,
    settling: bool,
    in_edge_block: bool,
    /// True while waiter CONTINUATIONS resumed inline in the active region
    /// (blocks-first mode) execute. Their writes happen AFTER this pass's
    /// edge detection, so — exactly like writes made inside an edge block —
    /// they must be recorded into `edge_exec_wrote` for the §9.2 rescan
    /// drain, or the change is silently swallowed when the next snapshot
    /// refreshes `prev` (an AnyEdge comb block sensitive to a TB-driven
    /// signal then never fires again: the write-data-stops-flowing bug).
    in_edge_cont: bool,
    /// Nesting depth of `check_edges_inner`. A `#delay` inside an edge block's
    /// body re-enters the edge machinery through the synchronous
    /// `TimingControl::Delay` arm; now that a nested pass sees the real block
    /// list (see `edge_blocks`) it can genuinely fire blocks, so a block that
    /// re-triggers itself through its own delay could recurse without bound.
    /// Bounded by `EDGE_PASS_DEPTH_LIMIT`.
    edge_pass_depth: u32,
    /// §9.4.2 bit- and part-select level event terms: `(signal_id,
    /// block_idx) -> mask` of the PHYSICAL bits the block's terms on that
    /// signal watch, for `always @(v[3] or v[7:5])`. Edge sensitivity is
    /// otherwise tracked per SIGNAL, so a block watching `v[0]` also woke on
    /// a change to `v[3]`. Applies to the any-change fanout only; an entry
    /// exists only when every term of the block on that signal is such a
    /// select (see `narrow_event_mask`). Kept as a sparse side map rather
    /// than a field on `SensitivityId` because select terms are rare and the
    /// dispatch loop is the hottest path in the simulator: when this map is
    /// empty the extra work is a single already-hot bool test.
    bitsel_edge_sens: HashMap<(usize, usize), event_bits::EventMask>,
    /// Bitset over signal ids: bit set ⇔ `bitsel_edge_sens` has ≥1 entry for
    /// that sid. Lets the edge dispatch loop skip the per-(sid, block) tuple
    /// hash for the overwhelmingly common signals with no bit-select
    /// sensitivity (one such entry anywhere used to put a HashMap get on
    /// every block wake — ~1.5% of c906 memcpy cycles).
    bitsel_sid_bits: Vec<u64>,
    /// Reusable bitmap for `check_edges`. Hoisted out of the per-iteration
    /// `vec![false; blocks.len()]` to avoid ~10 K-byte alloc/drop on every
    /// settle iter (8938 iters × ~10 K blocks = 89 MB churn on c910 hello).
    edge_triggered_bitmap: Vec<bool>,
    /// Reusable work buffers for `check_edges`; populated directly while
    /// walking fired edge signals so we do not scan the whole bitmap to
    /// discover triggered blocks.
    edge_triggered_list: Vec<usize>,
    /// Reusable `check_edges` buffers for the fired positions' detect-time
    /// values (narrow planes / wide values); a fresh pair grew on every pass.
    edge_fired_snap: Vec<(usize, u64, u64)>,
    edge_fired_wide: Vec<(usize, Value)>,
    /// Generation marker for gateable blocks rejected by ARMED filtering
    /// during fanout dispatch. This deduplicates multi-sensitivity blocks
    /// without appending skipped work to `edge_triggered_list`.
    edge_prefilter_seen: Vec<u32>,
    edge_prefilter_generation: u32,
    edge_parallel_work: Vec<usize>,
    edge_sequential_work: Vec<usize>,
    nba_queue: Vec<NbaEntry>,
    /// §4.6 b): signals with a nonblocking update waiting in `nba_queue`
    /// this NBA window, among its first `nba_queue_noted` entries. A later
    /// whole-signal or element nonblocking update to one of them queues
    /// behind it instead of taking `nba_fast`, which commits first
    /// (`nba_behind_queued_partial`). Cleared when the window commits.
    nba_queued_partial_ids: HashSet<usize>,
    nba_queue_noted: usize,
    /// §15.5.2 `->> e` nonblocking event triggers: fired when the NBA region
    /// flushes, not in the active region (a same-slot `.triggered` read in an
    /// active/inactive process must still see 0).
    pending_nba_triggers: Vec<String>,
    /// NBA-deferred `->>m_event` triggers on class-field events, captured as
    /// the resolved `(this_handle, field_name)` key at trigger time (the NBA
    /// drain runs outside the method, with no `this` to reconstruct it).
    pending_nba_instance_triggers: Vec<(usize, String)>,
    /// §9.6.2 `disable <task_name>`: which processes are currently executing
    /// an invocation of each named task. Filled by bind_task_frame, cleared
    /// by unwind_task_frame; a cross-process disable kills the listed pids.
    active_task_pids: HashMap<String, HashSet<usize>>,
    /// §15.5.5 event variables are HANDLES to synchronization objects:
    /// `ev_alias = ev_b` makes both names one object. Maps an event variable
    /// (or element key `arr[i]`) to the event it currently aliases;
    /// `EVENT_NULL_KEY` marks a null handle. Trigger/`.triggered`/wait_order
    /// all chase this map to the canonical key first.
    event_aliases: HashMap<String, String>,
    /// §15.5.5: the event OBJECT a class-member event slot `(handle,
    /// field)` holds once an assignment (`ev = other_ev`) re-bound it, as an
    /// id shared by every slot holding the same event. A slot absent here
    /// holds its own event. `event_obj_slots` is the reverse index.
    event_obj_ids: HashMap<(usize, String), u64>,
    event_obj_slots: HashMap<u64, Vec<(usize, String)>>,
    next_event_obj_id: u64,
    /// The modport names the design declares (`modport_member_dirs`), built
    /// at setup: a dotted name with no inner segment among them is not
    /// modport-qualified (§25.5).
    modport_names: HashSet<String>,
    /// The static class properties a packed-struct member read can name
    /// (`collect_static_struct_props`), built at setup.
    static_struct_props: HashSet<String>,
    /// The class properties that can hold a `real`
    /// (`collect_class_real_props`), built at setup.
    class_real_props: HashSet<String>,
    /// Every class property name declared `event`, built on first use.
    class_event_prop_names: Option<std::rc::Rc<HashSet<String>>>,
    /// Fast-path NBA buffer: pre-resolved (signal_id, value) pairs.
    nba_fast: Vec<NbaFast>,
    /// Pattern target writes collected before scheduling their NBA updates.
    /// Empty at scheduling boundaries; never part of a checkpoint.
    #[serde(skip)]
    pattern_nba_leaves: Option<Vec<(String, Value)>>,
    /// Reverse index: signal_id → most recent index in `nba_fast` for that
    /// signal during the *current* NBA accumulation window. Used by the
    /// partial-range/bit NBA Insns (NbaAssignRange, NbaAssignRangeDyn,
    /// NbaAssignBitDyn) to merge into the existing pending entry instead
    /// of re-scanning `nba_fast` linearly. The previous `iter().rposition`
    /// scan was O(N) per call → O(N²) per cycle when an always block had
    /// many partial-range NBAs (c910 testbench wrappers — thousands per
    /// posedge clk). Cleared by `apply_nba` once the entries are drained.
    nba_fast_index: NbaFastIndex,
    /// Edge-sensitive always blocks. Behind an `Arc` so an edge pass can hold a
    /// cheap handle to the list while `self` stays fully usable: the pass used
    /// to `mem::take` this Vec, which left it EMPTY for the duration. Any
    /// re-entrant lookup — a nested `check_edges` from a `#delay` in an edge
    /// block's body, or the `self.edge_blocks.get(bi)` probes elsewhere — then
    /// saw nothing and silently dropped every block (see `check_edges_inner`).
    edge_blocks: Arc<Vec<EdgeSensitiveBlock>>,
    /// Bytecode-compiled edge blocks (for blocks that compiled successfully).
    /// Index matches edge_blocks. None = fallback to AST interpreter.
    compiled_edge_blocks: Vec<Option<super::bytecode::CompiledBlock>>,
    /// Parallel array to `compiled_edge_blocks`: when feature=jit is on
    /// and `JitModule::try_compile` succeeded at elaboration time, this
    /// holds the native function pointer for the block. `exec_bytecode`
    /// calls it in place of the interpreter loop. None = use interpreter.
    #[serde(skip)]
    jit_fns: Vec<Option<super::jit::JitFn>>,
    /// Native code for COMB (settle) entries, by comb-entry index. Settle is
    /// ~70% of the loop on DUT-heavy RTL while edge blocks are ~25%, so this
    /// is where the JIT pays. cfg-gated: an earlier runtime-checked version
    /// cost +2.0% in the DEFAULT build from the hot-loop check alone.
    #[cfg(feature = "jit")]
    #[serde(skip)]
    comb_jit_fns: Vec<Option<super::jit::JitFn>>,
    /// Template-mode native comb fns: `(body, offset into comb_tpl_map_blob,
    /// map len)` per entry. The mapping arrays are HOST-owned (one flat blob)
    /// so the generated crate carries only the bodies — see
    /// `gen_templated_blocks`.
    #[cfg(feature = "jit")]
    #[serde(skip)]
    comb_tpl_fns: Vec<Option<(super::aot::AotTplFn, u32, u32)>>,
    #[cfg(feature = "jit")]
    comb_tpl_map_blob: Vec<u32>,
    /// Template-mode native EDGE-block fns, same layout as `comb_tpl_fns`.
    /// Checked before `jit_fns` in the edge fast path.
    #[cfg(feature = "jit")]
    #[serde(skip)]
    edge_tpl_fns: Vec<Option<(super::aot::AotTplFn, u32, u32)>>,
    #[cfg(feature = "jit")]
    edge_tpl_map_blob: Vec<u32>,
    /// Consecutive X/Z-bail strikes per entry (retires the fn past a limit).
    #[cfg(feature = "jit")]
    comb_jit_strikes: Vec<u8>,
    /// (ran natively, bailed on X/Z) under XEZIM_PROFILE_TIMING.
    #[cfg(feature = "jit")]
    prof_comb_jit: [u64; 2],
    /// The `JitModule` owns the JIT'd code's memory — it must outlive
    /// all compiled function pointers. Kept on Simulator so the mmapped
    /// code pages stay mapped for the life of the simulation.
    #[serde(skip)]
    jit_module: Option<super::jit::JitModule>,
    /// True for blocks eligible for parallel execution (no StmtFallback).
    edge_block_parallel: Vec<bool>,
    /// Per-edge-block partition / core assignment. Indexed by edge_block
    /// idx; value is the partition (worker thread) that owns the block.
    /// Empty when no partition has been loaded — falls back to the legacy
    /// chunk-by-range scheduling. Loaded from a partition file emitted by
    /// mt-kahypar (or any compatible hMETIS partitioner).
    pub(crate) edge_block_partition: Vec<u32>,
    /// Number of distinct partitions (k) when `edge_block_partition` is
    /// populated. Drives how many worker threads the parallel-edge
    /// dispatcher spawns.
    pub(crate) edge_block_partition_count: u32,
    /// Per-edge-block execution counts. Cheap enough to maintain on the hot
    /// path, and used only for optional end-of-run attribution.
    edge_block_exec_counts: Vec<u64>,
    edge_block_stats_enabled: bool,
    /// Phase-2 profile-guided partitioning: accumulated wall time
    /// (nanoseconds) spent executing each edge block. Populated
    /// only when `edge_block_stats_enabled` is set. Drives the
    /// vertex weight in profile-guided hypergraph emission so
    /// mt-kahypar balances *real* per-thread work, not insn count.
    edge_block_exec_ns: Vec<u64>,
    /// Phase-2 profile-guided partitioning: per-signal toggle count
    /// for signals in the edge-sensitivity universe. Updated lazily
    /// in `check_edge_id` whenever a sampled signal changes. Used as
    /// the hyperedge weight in profile-guided emission — busy
    /// signals are more expensive to share across cores.
    signal_toggle_count: Vec<u64>,
    /// Phase-4 counter: number of NBA writes performed per edge
    /// block during a stats-enabled run. NBAs cost a queue insert +
    /// a later apply pass, both of which are heavier than reads.
    /// Better vertex-weight signal than insn count for highly
    /// asymmetric blocks (small body, many NBAs vs big body, few NBAs).
    nba_count_per_block: Vec<u64>,
    /// Phase-4 counter: per-sensitivity-signal trigger count
    /// (sid → fires). Subset of signal_toggle_count restricted to
    /// signals that actually appear in some block's sensitivity list.
    /// Filters out toggles on signals that never wake anyone.
    sensitivity_trigger_count: Vec<u64>,
    /// Phase-4 counter: per-block cross-block wakeup count. When
    /// block A's NBA write changes a signal that triggers block B,
    /// `cross_block_wakeup_count[A]` increments. Captures the actual
    /// "packet flow" between blocks — much sharper hyperedge weight
    /// than static fanout because it counts only the signals that
    /// CAUSED a wakeup, not all readers.
    cross_block_wakeup_count: Vec<u64>,
    /// Phase-4 counter: per-partition local-quiescence iteration
    /// count — how many settle iterations were resolved within the
    /// partition before any cross-partition signal had to be
    /// merged. Higher = better partition (more local work). Same
    /// indexing as boundary_packet_count.
    local_quiescence_iterations: Vec<u64>,
    /// Per-edge-block: cached scope hint (parent module name from first
    /// sensitivity signal) so sequential-exec path can skip per-call rsplit.
    edge_block_scope: Vec<Option<String>>,
    /// Per-edge-block: true if block contains StmtFallback insns (and thus
    /// needs `name_resolve_hint` to be set for AST-exec). When false, the
    /// save/restore clone of the RefCell string can be skipped.
    edge_block_needs_hint: Vec<bool>,
    /// Per edge block: no fallback, no `Format`, compiled — the block can
    /// neither resolve a bare name nor print `%m`, so the per-firing scope
    /// bookkeeping in `exec_bytecode` (a String clone into
    /// `name_resolve_hint` + the `%m` buffer refresh) is skipped. At ~580
    /// fired blocks/tick the clone alone was allocator-visible in perf.
    edge_block_light: Vec<bool>,
    /// Flat copy of each compiled edge block's instruction count so the hot
    /// dispatch loop doesn't chase the `Option<CompiledBlock>` header.
    edge_block_insn_len: Vec<u32>,
    /// VM register file (reusable across executions to avoid allocation).
    vm_regs: Vec<Value>,
    /// class-perf Step 9d: foreach key arenas for compiled methods — slot
    /// i holds the materialized key `Value`s plus the per-slot position
    /// cursor of the i-th (compile-time numbered) `foreach` in the
    /// currently-executing block. Swapped in `try_run_compiled_method`
    /// alongside `vm_regs` so a nested compiled call never clobbers an
    /// outer loop's keys; grown lazily to the block's `foreach_slots`.
    /// The cursor is a `u32` (never `usize::MAX`-sentinel — the executor
    /// compares against `keys.len()`).
    foreach_arena: Vec<(Vec<Value>, u32)>,
    /// Spare register files for the combinational and continuous-assignment
    /// bytecode a settle evaluates (`enter_private_regs`). A compiled method
    /// or process loop waiting in an interpreted call keeps its live
    /// registers in `vm_regs`; a settle that call starts runs on a file of
    /// its own (§13.4, §8.11). A stack, so evaluations may nest; the files
    /// hold stale scratch only.
    #[serde(skip)]
    vm_regs_pool: Vec<Vec<Value>>,
    /// Built-in clock generators (optimized always #N clk = ~clk)
    clock_generators: Vec<ClockGen>,
    /// Source position of every clock generator, for its time-0 rank:
    /// `(index into clock_generators, instance scope, source offset)`.
    clock_gen_src: Vec<(usize, String, usize)>,
    /// Pids of `initial` blocks that cannot suspend (no timing control,
    /// no blocking call); see `order_time0_processes`.
    t0_timing_free: HashSet<usize>,
    /// Time-0 activation index of each process queued at time 0, consulted
    /// by `clock_rank_on_pop` until the first drain settles the generators.
    t0_pid_ord: HashMap<usize, usize>,
    /// Dynamic-delay simple assignments, keyed by their scheduled process id.
    fast_delay_always: HashMap<usize, FastDelayAlways>,
    /// Initial blocks of module instances queued at time 0 by their pending
    /// (source-shared) form, by pid: each is materialized for its instance
    /// when its process starts, so the owned bodies are never all held at
    /// once. Empty from the end of the time-0 slot on.
    #[serde(skip)]
    lazy_initial: HashMap<usize, super::elaborate::PendingInitial>,
    /// Compiled process FSMs by pid (roadmap 11-12, opt-in XEZIM_PROC_FSM=1).
    proc_fsm: HashMap<usize, Box<ProcFsm>>,
    /// Resume pc for the next `exec_insns` call (consumed at entry; 0 for
    /// ordinary block executions).
    fsm_start_pc: u32,
    /// Set by the Wait insn arms: (resume pc, what to wait on). `exec_insns`
    /// breaks its loop right after setting this.
    fsm_suspend: Option<(u32, FsmWait)>,
    /// Precision diff of the CURRENTLY EXECUTING process FSM (the Wait arm
    /// needs it to quantize the delay register).
    fsm_precision_diff: i32,
    event_queue: TimingWheel,
    next_pid: usize,
    current_pid: usize,
    /// Monotonic counter assigning each pushed local call-frame a unique
    /// generation (see `local_gen_stack`). Used to tell apart two different
    /// method activations that reuse the same `local_stack` slot index — e.g.
    /// a fork child spawned under method-<i>A</i> that keeps running after
    /// <i>A</i> returns must not merge its (stale) locals into the frame slot
    /// that method-<i>B</i> now owns (regression 3627).
    frame_gen: u64,
    /// Value that `$` resolves to in the current evaluation scope
    /// (e.g. queue upper bound during `q[a:$]`). Stack of overrides.
    dollar_bound: Vec<i64>,
    break_flag: bool,
    continue_flag: bool,
    /// Set by a `return` (alongside break_flag). Distinguishes a task/function
    /// `return` from a loop `break`: a return must unwind to the enclosing
    /// task's `ScopePop` boundary (which clears it via `saved_return`), whereas
    /// a `break` only exits the nearest loop. Without this, an inlined blocking
    /// task's `return` leaked break_flag past its ScopePop into the caller's
    /// continuation.
    return_flag: bool,
    rs_return_flag: bool,
    /// Set when `exec_statement`'s Wait handler parks the process via
    /// `exec_park_cont`. Must propagate through method-frame save/restore.
    parked_from_exec: bool,
    /// When `exec_statement` encounters a blocking `wait(cond)` with a
    /// FALSE condition inside a loop body (foreach/while/for), it cannot park
    /// (it has no continuation). `run_process_stmts` sets this to the full
    /// continuation `[stmt, stmts[i+1..]]` before calling `exec_statement`, and
    /// `exec_statement`'s Wait handler reads it to park the process.
    exec_park_cont: Option<ProcCont>,
    /// Depth of DPI-exported subroutines running for a C caller. Their
    /// statements run on the synchronous path and cannot park, so a wait
    /// there runs the scheduler nested instead (`run_nested_until`).
    dpi_export_depth: u32,
    /// §35.5.2/§35.9: imported tasks running on their own stacks
    /// (`dpi_task.rs`): the SV names of the imported tasks a loaded library
    /// implements (empty: none, and the mechanism costs nothing), the live
    /// calls by id, the one running now, and whether that one is unwinding
    /// after a disable.
    dpi_task_names: HashSet<String>,
    #[serde(skip)]
    dpi_tasks: HashMap<u64, Box<dpi_task::DpiTask>>,
    /// Strings an exported subroutine handed to C (a `string` result or
    /// output), kept until its next call (§35.5.6).
    dpi_export_cstrings: HashMap<usize, Vec<std::ffi::CString>>,
    /// Numbers the hidden variables of each call from C (`dpi_export.rs`).
    dpi_export_seq: u64,
    /// The time slot batches `run_events_until` is running, outermost first:
    /// their unrun processes (`requeue_slot_batches`).
    slot_batches: Vec<(u64, EventList)>,
    dpi_cur_task: Option<u64>,
    dpi_unwinding: bool,
    next_dpi_task: u64,
    /// Wake markers fired by waiters registered from that nested path.
    sync_wakes: HashSet<u64>,
    /// The run ended while a task entered from C was still waiting: nothing
    /// was left to wake it, or `--max-time` came first (`run_nested_until`).
    /// `finished` is raised to stop it, but no `$finish` was called.
    ended_waiting_in_c: bool,
    next_sync_wake: u64,
    /// File names of open multichannel descriptors (`vpi_mcd_name`).
    mcd_names: HashMap<i32, String>,
    /// No-progress guard for the `foreach` replay fallback: how many times
    /// a given `(pid, statement offset)` has parked without the loop ever
    /// completing. That fallback restarts at index 0 on every resume, so a
    /// run of parks means the loop is stuck rather than iterating.
    foreach_replay_parks: HashMap<(usize, usize), u32>,
    /// Consecutive parks tolerated before the guard fires. 0 disables it.
    /// Override with `XEZIM_FOREACH_REPLAY_LIMIT`.
    foreach_replay_limit: u32,
    /// SV-2023: target named block for `disable <name>` propagation.
    disable_target: Option<String>,
    /// §9.6.2: a `disable` just executed whose target has not been located
    /// yet — the process loop checks its own continuation first, then the
    /// processes the block may belong to (`disable_remote_block`).
    disable_check_pending: bool,
    /// §9.6.2: address of each flattened NAMED block frame → the block name,
    /// so a `disable` can find where the block ends in a continuation chain.
    named_block_frames: HashMap<usize, String>,
    /// §9.6.2: the forking process's continuation at each run-loop fork
    /// child's spawn — the named blocks the child was forked inside.
    fork_spawn_cont: HashMap<usize, ProcCont>,
    fork_spawn_prune_at: usize,
    /// §9.6.2: processes unwound out of a named block or task by another
    /// process's `disable`, applied when each next runs.
    pending_block_disable: HashMap<usize, String>,
    /// Label of a process-level named block (`initial begin : worker`) -> its
    /// pid, so `disable worker` from another process can terminate it.
    disable_labels: HashMap<String, usize>,
    /// A named `fork : blk ... join*` maps to the processes it spawned, so
    /// `disable blk` can terminate them (§9.6.2). Without this, disabling a
    /// join_none fork block — which has already returned — unwound the
    /// DISABLING process instead, dropping its continuation.
    fork_block_children: HashMap<String, HashSet<usize>>,
    /// Associative arrays already warned about an x/z index.
    warned_assoc_index: HashSet<String>,
    /// `#delay` sites already warned about a pathological (non-finite / absurdly
    /// large) real delay value, keyed by the delay expression's source offset —
    /// warn once per site so a spiking clock names itself without spamming.
    warned_delay_spikes: HashSet<usize>,
    /// Assignment patterns already warned about an uncovered member.
    #[serde(skip)]
    warned_uncovered_patterns: HashSet<(usize, usize)>,
    /// Names the storage of each typed pattern value held at once.
    #[serde(skip)]
    typed_pattern_seq: u64,
    /// §6.19.5: the enumeration a type name denotes, for method receivers
    /// (`enum_receivers`), valid for `type_tables_gen` of the second field.
    #[serde(skip)]
    enum_recv_cache: HashMap<String, Option<std::rc::Rc<enum_receivers::EnumRecvType>>>,
    #[serde(skip)]
    enum_recv_cache_gen: u64,
    /// §13.5: the slice actuals of the `output`, `inout` and `ref` array
    /// formals of the calls running now, by caller key (`bind_slice_array_arg`):
    /// each formal index and the slice element it copies back to.
    #[serde(skip)]
    slice_arg_targets: HashMap<String, Vec<(i64, Option<String>)>>,
    /// Numbers the caller keys of `slice_arg_targets`.
    #[serde(skip)]
    slice_arg_seq: u64,
    /// PIDs whose first-at-time-0 variable `#(expr)` delay evaluated to 0 and
    /// was deferred once past the time-0 settle (so the period signal — e.g. a
    /// real cont-assign feeding `always #(p/2) clk=~clk` — settles before the
    /// clock's phase is fixed). Guards the one-shot re-park against a genuine
    /// zero-period loop re-parking forever at t=0.
    t0_delay_deferred: HashSet<usize>,
    /// Prepared frames for loops that suspend once per iteration.
    suspended_loop_frames: HashMap<usize, HashMap<(usize, usize), Arc<[Statement]>>>,
    /// Bytecode cached for non-suspending process loops after their enclosing
    /// local frame is known.
    process_loop_bytecode: HashMap<(usize, usize, usize), Option<super::bytecode::CompiledBlock>>,
    /// Processes owning an entry in the four per-process execution caches
    /// (`process_loop_bytecode`, `process_cond_bytecode`, `forever_cont_cache`,
    /// `forever_sens_cache`), so releasing one that owns none skips the scan.
    exec_cache_pids: HashSet<usize>,
    /// Bytecode cached for the CONDITION of a per-cycle self-check `if` whose
    /// statement as a whole cannot compile (e.g. its fail branch touches a
    /// name-keyed initial-scope variable). The condition — typically a wide
    /// compare against an inlinable helper call — is the hot part; the chosen
    /// branch still executes through the AST path.
    /// The u8 is the SELF-AUDIT countdown: the first activations evaluate the
    /// condition through BOTH the bytecode and the AST and compare; any
    /// disagreement permanently pins the span to the AST (None). Only
    /// side-effect-free conditions are cached, so the double evaluation is
    /// unobservable.
    process_cond_bytecode: HashMap<
        (usize, usize, usize),
        Option<(super::bytecode::CompiledBlock, super::bytecode::RegId, u8)>,
    >,
    /// SV-2023: PIDs killed by `disable fork` — skip dispatch on these.
    killed_pids: HashSet<usize>,
    /// Next simulated time `growth_trace` prints at.
    growth_trace_next: u64,
    /// §9.7 `process` class: PIDs that have completed execution (child_finished).
    finished_pids: HashSet<usize>,
    /// §9.7 `process` class: PIDs explicitly suspended via `suspend()`. The
    /// continuation + original delay info is in `suspended_proc_info`.
    suspended_pids: HashSet<usize>,
    /// §9.7: detailed resumption info for each suspended pid.
    suspended_proc_info: HashMap<usize, SuspendedProc>,
    /// §9.7 `process::await()`: processes waiting for another's termination.
    await_waiters: Vec<AwaitWaiter>,
    /// Names of init-declared (automatic) `for`-loop variables that — because
    /// the enclosing process had no local frame — currently live in the signal
    /// table. A `fork` child must capture these BY VALUE at fork time so the
    /// canonical `for (int i…) fork … join_none` idiom sees its per-iteration
    /// `i` instead of the loop's final value (LRM §9.3.2). Pushed/popped as a
    /// stack by the `for` executor to handle nested loops.
    auto_loop_vars: Vec<String>,
    /// SV-2023: redirected lvalues for `ref` formal parameters during a call.
    /// Top-of-stack scope is consulted when assigning to a known formal name.
    ref_binding_stack: Vec<HashMap<String, Expression>>,
    /// §13.5.2 TRUE ALIASING for the subset of `ref` actuals whose storage is
    /// module-visible (a plain variable, resolvable independent of the caller
    /// frame). Reads of the formal re-read the actual and writes write through
    /// IMMEDIATELY — a parallel process observing the actual mid-call sees the
    /// callee's writes, and no return copy-out clobbers writes the observer
    /// made during the call. Actuals that are caller-frame locals or aggregate
    /// elements keep the legacy copy-in/copy-out path (entry absent here).
    /// Pushed/popped in lockstep with `ref_binding_stack`.
    ref_alias_stack: Vec<HashMap<String, String>>,
    /// §13.5.2 `ref` formals bound by IDENTITY (aggregates with no storage of
    /// their own); element accesses on them are rewritten onto the actual.
    /// Pushed/popped in lockstep with `ref_binding_stack`.
    ref_identity_stack: Vec<Vec<String>>,
    /// Rename-table entries of the queue / associative formals bound so far
    /// in the call being set up (see `bind_queue_param`). They enter the
    /// callee's frame only once every actual is evaluated: an actual must
    /// not resolve to a formal of the call it is being passed to.
    pending_formal_dyn: Vec<(String, String)>,
    /// True while the top ref frame has an aliased or identity-bound formal —
    /// the only state in which `ref_formal_redirect_*` can rewrite anything.
    /// Maintained on push/pop/take/restore so the hot paths test one bool.
    ref_redirect_hot: bool,
    /// §13.5.2: callee frames aliasing a property of each pinned object
    /// (see `ref_pins_acquire`); the pin variable lives while this is > 0.
    ref_pin_users: HashMap<usize, u32>,
    /// §19.7.1 `cg::type_option.<field>` per covergroup TYPE (weight, goal,
    /// comment, strobe, merge_instances), seeded from the body's
    /// `type_option.f = v;` items on first access.
    cg_type_options: HashMap<String, HashMap<String, Value>>,
    /// Per-call-site receiver expressions for `obj.coll.method()` dispatch.
    method_receiver_cache: HashMap<(usize, usize, usize, u32, u32), std::rc::Rc<Expression>>,
    /// Reused buffer for the `<hint>.<name>` probe in `get_signal_value_by_name` (a `&self` path).
    hint_key_scratch: std::cell::RefCell<String>,
    /// `decl_param_scope`'s state: the parameters under each scope, and the
    /// overlaid parameter tables it hands out. A select loop over one
    /// instance's signal rebuilds nothing, and a move to another instance
    /// changes only the scopes the two do not share instead of copying the
    /// table.
    #[serde(skip)]
    decl_param_scope_cache: std::cell::RefCell<DeclParamScopeCache>,
    /// Reused buffer for the `__vif_local__` key probes (`&self` paths).
    vif_key_scratch: std::cell::RefCell<String>,
    /// §8.25: specialization-argument texts already folded to their value
    /// (`2 + 2` -> `4`, `'h4` -> `4`), or None when the text is not a
    /// constant integer expression (see `fold_const_spec_frag`).
    spec_frag_fold_cache: RefCell<HashMap<String, Option<String>>>,
    /// The leaf name of every design parameter key (`LP` of `tb.LP`), built
    /// on first use: the gate before a scoped parameter lookup.
    #[serde(skip)]
    param_leaf_names: std::cell::OnceCell<HashSet<String>>,
    /// §6.22.1: canonical type text of each design-level unpacked typedef
    /// (`int[$]`) -> the typedef that stands for that type in
    /// specialization keys (built on first use; see `unpacked_typedef_rep`).
    #[serde(skip)]
    unpacked_typedef_reps: std::cell::OnceCell<HashMap<String, String>>,
    /// Every typedef name (design or class scope, bare and qualified) that
    /// carries unpacked dimensions or aliases one that does: the cheap gate
    /// of `unpacked_typedef_rep`.
    #[serde(skip)]
    unpacked_typedef_names: std::cell::OnceCell<HashSet<String>>,
    /// Reused buffer for `frame_leaf_key_of`'s marker probe.
    marker_key_scratch: String,
    /// See `lvalue_root_is_unpacked_struct_prop`.
    #[serde(skip)]
    unpacked_struct_prop_names: std::cell::OnceCell<HashSet<String>>,
    /// See `struct_root_possible`.
    struct_root_names: HashSet<String>,
    struct_root_names_built: bool,
    /// See `multidim_members_possible`.
    multidim_members: Option<bool>,
    /// See `struct_prop_name_possible`.
    #[serde(skip)]
    struct_capable_prop_names: std::cell::OnceCell<HashSet<String>>,
    /// Member names that appear as dynamic/queue/associative members of a
    /// struct type a class property (or a type-parameter binding, which can
    /// only name a typedef) can resolve to — the cheap gate in front of
    /// `class_struct_coll_name`'s receiver walk.
    #[serde(skip)]
    struct_coll_member_prop_names: std::cell::OnceCell<HashSet<String>>,
    /// §7.4.5: names whose ELEMENTS are collections — variables declared as
    /// a fixed array of queues / dynamic / associative arrays (`int q3[3][$]`)
    /// and struct members declared that way (`T sp[4][$]`). Gates the
    /// whole-collection assignment of a selected element (`q3[0] = q3[1]`).
    #[serde(skip)]
    indexed_coll_roots: std::cell::OnceCell<HashSet<String>>,
    /// Bit `len % 64` per name in `indexed_coll_roots` (0: none), so the
    /// common miss costs no hashing.
    indexed_coll_mask: std::cell::Cell<u64>,
    /// Bit `len % 64` set for the length of every name in
    /// `struct_coll_member_prop_names`: rejects most names without hashing.
    struct_coll_member_len_mask: std::cell::Cell<u64>,
    /// Struct-member collections that live inside an associative-array
    /// ELEMENT (`aa[k].q`): growing one creates the element (§7.8.6), see
    /// `mark_struct_coll_owner_present`.
    assoc_elem_colls: HashSet<String>,
    /// Element collections (`@v#3[0]`, ...) of each automatic local array of
    /// collections, keyed by the local's storage; dropped with it (see
    /// `declare_local_coll_array`, `cleanup_dyn_storage`).
    local_coll_elems: HashMap<String, Vec<String>>,
    /// §7.4.5: the declared shape of the local being declared, registered
    /// under the per-call storage key `declare_local_dyn` gives it.
    pending_local_shape: Option<CollShape>,
    /// Whether a subroutine-local collection of collections was declared,
    /// which the indexed-collection gates then also look up per frame.
    local_coll_rows_live: bool,
    /// Declared shapes of subroutine locals under their per-call keys
    /// (`@l#n`), apart from `module.coll_shapes`: one entry per call comes
    /// and goes, which in the large module table forced rehashes.
    local_coll_shapes: HashMap<String, CollShape>,
    /// Declared associative tails of dynamic roots and their scoped rows.
    dynamic_assoc_dims: HashMap<String, Vec<crate::ast::types::UnpackedDimension>>,
    /// `(callee member, caller member)` collection copy-backs of the dynamic
    /// members of an output/inout/ref struct formal, drained into the call's
    /// queue writebacks (see `bind_unpacked_struct_formal`).
    pending_struct_coll_wb: Vec<(String, String)>,
    /// The staged dynamic members of a struct a `return` handed back (see
    /// `stage_ret_struct_colls`), for the caller's assignment to copy.
    pending_ret_struct: Option<String>,
    /// See `class_member_names`.
    #[serde(skip)]
    class_member_names_cell: std::cell::OnceCell<ClassMemberNames>,
    /// Every bare name a `__vif_local__` key was ever created for (never
    /// shrinks): a name outside it has no such key.
    vif_local_names: HashSet<String>,
    /// See `nonvirtual_target_class`.
    #[allow(clippy::type_complexity)]
    nonvirtual_target_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<String>>>>,
    /// `class_is_a` answer cache: (derived, base) -> bool.
    class_is_a_cache: std::cell::RefCell<HashMap<(String, String), bool>>,
    /// `resolve_interface_port` answer cache: raw name -> bound string.
    interface_resolve_cache: std::cell::RefCell<HashMap<String, Option<String>>>,
    /// `canonicalize_spec_sig` cache: (class_name, sig) -> canonical signature.
    spec_sig_cache: std::cell::RefCell<HashMap<(String, String), String>>,
    /// Whether any class definition in the design has an N-D array property.
    has_any_array_nd_properties: std::cell::Cell<Option<bool>>,
    /// `class_has_method` answers by class then method (see there).
    has_method_cache: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// See `member_string_keyed_unbound`.
    #[allow(clippy::type_complexity)]
    string_keyed_member_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<bool>>>>,
    /// See `method_defining_class`.
    #[allow(clippy::type_complexity)]
    method_def_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<MethodDef>>>>,
    /// Every covergroup key and its leaf after the last `::` or `.` (see
    /// `covergroup_def_for`).
    #[serde(skip)]
    covergroup_leaf_names: std::cell::OnceCell<HashSet<String>>,
    /// Class -> the `string_properties` of it and its ancestors, in chain
    /// order (a method call marks them in `string_signals`).
    class_string_props: std::cell::RefCell<HashMap<String, std::rc::Rc<Vec<String>>>>,
    /// Class -> `class_enclosing_module` answer.
    class_enclosing_cache: std::cell::RefCell<HashMap<String, Option<String>>>,
    /// See `plain_class_ref`.
    plain_class_types: std::cell::RefCell<Option<HashMap<String, bool>>>,
    /// See `coll_member_possible`.
    coll_member_names: std::cell::RefCell<Option<HashSet<String>>>,
    /// Reused `<name>.size` key buffer of the queue-frame save/restore.
    queue_key_scratch: String,
    /// See `prop_owners`.
    #[allow(clippy::type_complexity)]
    prop_owner_index: std::cell::RefCell<HashMap<String, std::rc::Rc<PropOwners>>>,
    /// `class_prop_width_impl` answers that no instance can change, by class
    /// then property; `None` marks a width the instance's bindings decide.
    #[allow(clippy::type_complexity)]
    prop_width_memo: std::cell::RefCell<HashMap<String, HashMap<String, Option<Option<u32>>>>>,
    #[serde(skip)]
    struct_member_offset_memo: std::cell::RefCell<HashMap<(String, String), (u32, u32)>>,
    #[serde(skip)]
    packed_struct_member_memo: std::cell::RefCell<
        HashMap<
            (String, u32, u32, String),
            Option<(u32, u32, bool, crate::ast::types::DataType, Vec<(i64, i64)>)>,
        >,
    >,
    /// Per-(handle, prop) cache of `class_prop_packed_dims` and
    /// `class_prop_signed_of`: a class property's packed-dimension / signed
    /// status is a per-object invariant, but the helpers walk the class
    /// hierarchy per call (per foreach element) with identical arguments.
    /// The key carries the minimum-dimension count the caller asked for
    /// (`class_prop_packed_dims` vs `class_prop_packed_dims_min1`): a walk
    /// that finds nothing at two dimensions may still find one.
    #[serde(skip)]
    class_prop_dims_memo:
        std::cell::RefCell<HashMap<(usize, String, usize), Option<Vec<(i64, i64)>>>>,
    #[serde(skip)]
    class_prop_signed_memo: std::cell::RefCell<HashMap<(usize, String), bool>>,
    /// Per-randomize cache of resolved `dist` geometries in foreach fast
    /// paths, keyed by the object, the constraint item (address and source
    /// span) and its item count. Cleared by each `randomize()`; only a `dist`
    /// whose bounds and weights read no `foreach` index, no rand variable and
    /// call nothing is memoized (`dist_memo_eligible`), as only those stay
    /// constant through one solve.
    #[serde(skip)]
    dist_geometry_memo: std::cell::RefCell<
        HashMap<
            (usize, usize, usize, usize, usize),
            crate::compiler::simulator::inline_constraints::ResolvedDist,
        >,
    >,
    /// Emptied formal-metadata snapshot vectors of the class-method call path,
    /// reused across calls.
    formal_parts_pool: Vec<Vec<(usize, FormalMetaParts)>>,
    /// Emptied `queue_frame_saves` / `local_dyn` frames and saved-name
    /// buffers, reused by the next subroutine entry.
    queue_save_pool: Vec<Vec<(String, QueueLocalSave)>>,
    local_dyn_pool: Vec<Vec<(String, String)>>,
    queue_name_pool: Vec<String>,
    /// Emptied `current_spec` save buffers (see `clone_spec_pooled`).
    spec_buf_pool: Vec<(String, String)>,
    /// Open class-method metadata saves, innermost last (see `MetaFrame`).
    meta_frames: Vec<MetaFrame>,
    /// How many of `meta_frames` have not taken their snapshot yet.
    meta_pending: usize,
    /// Call plans by `Arc<ClassMethod>` address (see `MethodCallPlan`).
    method_plans: std::cell::RefCell<HashMap<usize, std::rc::Rc<MethodCallPlan>>>,
    /// Declaration-scope typedefs before a procedural typedef shadows a name.
    /// The tables share storage until a write (§6.18, §23.9).
    declaration_typedefs: Option<(
        super::elaborate::TypedefTable,
        super::elaborate::SharedMap<DataType>,
    )>,
    /// Moves whenever a procedural `typedef` or an inline-enum local edits the
    /// type tables at run time (see `MethodCallPlan`).
    type_tables_gen: u64,
    /// Packed struct layouts of named formal types by qualified type name,
    /// each with the `type_tables_gen` it was built under. Laying out a
    /// `pkg::T` struct copies the whole parameter table to see the
    /// package's names bare, too costly to repeat on every call.
    formal_struct_layouts: HashMap<String, (u64, Option<Vec<(String, u32, u32)>>)>,
    /// Emptied `local_type_stack` overlay pairs, reused by `push_local_frame`
    /// (capacity kept, contents cleared) instead of allocating a map pair for
    /// every activation that records a class/typedef-typed name.
    local_type_pool: Vec<(HashMap<String, String>, HashMap<String, String>)>,
    /// Emptied subroutine names of popped `static_local_syncs` frames, reused
    /// by the next frame push instead of allocating a copy of the name.
    sync_name_pool: Vec<String>,
    /// Scratch key for `cleanup_dyn_storage`.
    dyn_key_scratch: String,
    /// Interned scope hints and head identifiers for `method_receiver_cache` keys (see there).
    method_receiver_hint_ids: HashMap<String, u32>,
    /// class-perf Step 4b: cache of fully-lowered class-FUNCTION `CompiledBlock`s,
    /// keyed by an INTERNED (class, method) id pair PLUS the resolved width
    /// signature (formal widths + result width). The width signature captures
    /// all instance-dependent compile inputs (parameterized formals / return),
    /// so a cached block is always byte-identical to a fresh compile of the
    /// same instance. String keys are never cached (HashMaps intern to u32).
    compiled_class_method_ids: HashMap<String, u32>,
    compiled_method_block_cache: HashMap<
        (u32, u32, u32, std::rc::Rc<Vec<u32>>, u32),
        super::bytecode::CompiledMethodOutcome,
    >,
    compiled_method_admission: Option<std::rc::Rc<super::bytecode::MethodAdmission>>,
    persisted_method_context_hash: Option<u64>,
    /// class-perf Step 7b: prebound compiled-method gates, keyed (cid, mid).
    /// Present only for methods that PASSED the compile gate; the per-call
    /// type-resolution work (return kind, formal seed widths) runs once at
    /// plan creation instead of per call.
    compiled_method_plans:
        HashMap<(u32, u32, u32), std::rc::Rc<super::bytecode::PreboundCompiledMethod>>,
    /// class-perf decision cache: interned (class, method) id pairs whose
    /// method can NEVER run as bytecode (task-form, string return, void, a
    /// ref/output formal, or a non-scalar return). These decisions are
    /// deterministic per method and instance-independent (`scalar_formal_integral`
    /// resolves widths from module scope), so once decided they never flip;
    /// memoizing them skips the gate + type-resolution re-validation on every
    /// one of the (mostly negative-cached) UVM calls.
    compiled_method_skip: HashSet<(u32, u32, u32)>,
    /// class-perf adaptive tiering: per-interned-(cid, mid) call counters.
    /// Bumped ONLY by methods that passed the skip cache (plausibly
    /// compilable) and only while below the tier threshold; once a method
    /// crosses, it compiles and the counter is never touched again (the
    /// compiled path above the threshold is counter-free). Keyed by the
    /// (cid, mid) ids the block cache uses, so interning is done once.
    compiled_method_call_counts: HashMap<(u32, u32, u32), u32>,
    /// class-perf loop-heat boost (MEMOIZATION / pass B): whether a method
    /// is a wait-free loop candidate, keyed (cid, mid, spec_id). Computing it
    /// runs a structural statement walk (`body_has_loop` + a wait-free probe),
    /// which the maintainer flagged as a per-call cost; it is a pure function
    /// of the declaration (instance-independent), so it is memoized once and
    /// never re-walked. `None` = not yet decided; `Some(true)` = candidate.
    compiled_method_boost_qual: HashMap<(u32, u32, u32), bool>,
    /// class-perf loop-heat boost (MEASURED HEAT / pass C): stack of interned
    /// (cid, mid, spec_id) keys for the wait-free loop-candidate methods
    /// currently interpreting on the AST function path. `exec_statement`'s
    /// loop arms add their back-edge count to the innermost key, so heat is
    /// attributed to the actual method running the loop (nested calls
    /// attribute to the innermost active one). Empty when the boost is off,
    /// keeping the default path flat.
    compiled_method_heat_stack: Vec<(u32, u32, u32)>,
    /// class-perf loop-heat boost (MEASURED HEAT / pass C): accumulated loop
    /// back-edges (iterations) a method's body actually ran while on the AST
    /// interpreter, keyed (cid, mid, spec_id). A body crosses the tier here
    /// -> treated as loop-hot (compiles) instead of compiling any short loop
    /// on first call.
    compiled_method_loop_heat: HashMap<(u32, u32, u32), u64>,
    /// Per (class, method, spec): does the body contain a loop? Such a
    /// method compiles on its first call under the default tier: one call
    /// can run many iterations, so a call count says little about its cost.
    compiled_method_has_loop: HashMap<(u32, u32, u32), bool>,
    /// class-perf P3: direct VM->VM dispatch entries for `CallMethod`,
    /// keyed (cid, mid) of the DEFINING class. `Pending` re-probes after
    /// the plan / block caches fill; `Never` is a permanent decline
    /// (param-dependent shape, writeback formals, fallback-heavy body);
    /// `Entry` runs the callee's block in-place with a fresh register
    /// file, skipping arg materialization + interpreter binding.
    compiled_fast_calls: HashMap<(u32, u32, u32), FastCallState>,
    /// Static-method twin of `compiled_fast_calls` (class-perf Change 3):
    /// memoized direct-call entries for no-arg static class functions, keyed
    /// (cid, mid, 0). A separate map because `vm_fast_entry_for` permanently
    /// declines static methods in the instance map, which would poison this
    /// path's memoization.
    compiled_fast_static_calls: HashMap<(u32, u32, u32), FastCallState>,
    /// Phase 2 (class-perf): intern a `(base_class, spec_sig)` specialization to a
    /// u32 id (0 = non-specialized / no spec), so the plan / skip / fast-call caches
    /// can be keyed by `(spec_id, cid, mid)` instead of `(cid, mid)`. Without this, a
    /// parameterized class's method (e.g. `Cache#(string)::getv`) shared one plan
    /// across ALL specializations, forcing it to the AST interpreter whenever a
    /// type-param formal/return wasn't module-scope-resolvable. Interned lazily from
    /// `inst.spec` at the plan-lookup point.
    compiled_spec_intern: HashMap<(String, String), u32>,
    /// `resolve_typeref_class_name` memo: name -> scope -> class ctx -> (class-table size, answer).
    #[allow(clippy::type_complexity)]
    typeref_class_memo: std::cell::RefCell<
        HashMap<String, HashMap<String, HashMap<String, (usize, Option<String>)>>>,
    >,
    /// Deferred teardown for inlined blocking task/method calls (LIFO). Each
    /// `ScopePop` sentinel pops and replays the top entry. Per-process (carried
    /// in ProcessContext across suspension).
    task_cleanup: Vec<TaskCleanup>,
    /// Names carrying a TRUSTED `var_typedef_types` binding — created by an
    /// active `foreach` key-type binding (§6.19.6/§12.7.3) and restored by
    /// `restore_loop_vars`. Consulted before every other type source.
    fe_trusted_types: HashSet<String>,
    /// `wait(expr)` whose condition references no signals — it depends on class
    /// members / locals another same-time process mutates.
    /// Such a process parks here
    /// (pid + continuation whose first stmt is the Wait) and is re-checked in a
    /// fixpoint after the same-time batch drains, suspending until the
    /// condition genuinely flips instead of falsely proceeding.
    condition_waiters: PidVec<ProcCont>,
    /// Waiters from `condition_waiters` whose conditions became true upon
    /// blocking writes, recorded in write-satisfaction order (§9.7.4).
    ready_condition_waiters: PidVec<ProcCont>,
    /// Ident/member names read by the wait condition of each parked waiter.
    cond_waiter_reads: HashMap<usize, HashSet<String>>,
    /// Wait condition expression for each parked waiter.
    cond_waiter_conditions: HashMap<usize, Expression>,
    /// Per parked condition waiter: `Some(bits)` when everything its
    /// condition reads lives in the string-keyed stores (see
    /// `cond_gate_for`), with the `name_bit`s of each name it reads. Such a
    /// waiter is skipped by the bulk re-schedule until a write notes all the
    /// bits of one of those names — nothing it can observe has changed; a
    /// constant condition (`wait(0)`) reads no name and is never re-checked.
    /// `None` keeps the unconditional re-schedule (RTL signals, calls).
    cond_waiter_gate: HashMap<usize, Option<Box<[u64]>>>,
    /// Timestamp of each UNGATED parked waiter's last unconditional re-check
    /// by a re-pass drain (`drain_condition_waiters(_, from_repass: true)`);
    /// see that method for why one re-check per timestamp is enough.
    cond_repass_recheck: HashMap<usize, u64>,
    /// IEEE 1800-2017 §4.4.2.3 Inactive region: continuations of `#0`
    /// delays park here instead of in the event_queue. The event_queue's
    /// batch drain in `run_one_tick` re-fetches same-time entries into the
    /// SAME active pass (before `apply_nba`), so scheduling a `#0`
    /// continuation there made it resume BEFORE the NBA region committed —
    /// an NBA posted before the `#0` was invisible after it. Commercial
    /// simulators all make an NBA posted before a
    /// `#0` visible after it, so entries parked here are promoted back into
    /// the event queue only at the END of the current tick, after
    /// `apply_nba` has run (see `promote_inactive_to_active`).
    inactive_queue: PidVec<ProcCont>,
    /// Processes waiting for the NBA region to commit (e.g.
    /// `uvm_wait_for_nba_region`: `nba <= next_nba; @(nba)` where `nba` is a
    /// procedural variable). Resumed in `run_one_tick` after all active +
    /// inactive (`#0`) deltas have settled and `apply_nba` has committed.
    nba_region_waiters: Vec<(usize, ProcCont)>,
    /// Bumped whenever a `wait(expr)` condition evaluates true (a parked waiter
    /// proceeds). The condition-waiter fixpoint uses it to detect "no progress
    /// this round" and stop (genuine stall).
    cond_progress: u64,
    /// Parked `wait(cond)` waiters a drain rescheduled to re-evaluate
    /// (`[PROF] condition_waiter_rechecks`).
    cond_waiter_rechecks: u64,
    /// Compiled-to-compiled calls dispatched directly (`vm_try_direct_call_from`),
    /// without the interpreter's binding (`[PROF] vm_direct_calls`).
    vm_direct_calls: u64,
    /// Queue elements a `pop_front`, `push_front`, `insert` or `delete`
    /// moved one at a time by name, because the queue's elements are not a
    /// single run of plain values (`[PROF] queue_elem_moves`).
    queue_elem_moves: u64,
    /// Stores to an element or member of a packed vector wider than 64 bits
    /// that read the whole vector out and stored it back instead of editing
    /// it in place (`[PROF] select_store_copies`).
    select_store_copies: u64,
    /// Checks of the running process's claims for displaced local metadata
    /// (`reapply_local_meta`, `[PROF] local_meta_reapplies`), the displaced
    /// declarations installed again (`[PROF] local_meta_swaps`), and the
    /// captures that scanned the tables for a struct local's `name.member`
    /// keys (`[PROF] local_meta_scans`, one per declaration at most).
    local_meta_reapplies: u64,
    local_meta_swaps: u64,
    local_meta_scans: u64,
    /// SV-2023: blocked mailbox `get`/`peek` proxies keyed by mailbox handle.
    /// Each waiter records the suspended process pid, the destination lvalue
    /// expression to receive the value, and the post-get continuation
    /// statements. `put` drains a waiter, assigns the value into its lvalue,
    /// and re-schedules the continuation at the current time.
    mailbox_get_waiters: HashMap<usize, std::collections::VecDeque<MailboxGetWaiter>>,
    /// IEEE 1800-2023 §9.3.2: a fork child's WRITES to the parent's automatic
    /// variables must be visible to the parent after the child finishes. xezim
    /// gives each child a COPY of the parent's locals (see
    /// `inherit_fork_child_context`), then `propagate_fork_locals_to_parent`
    /// merges the child's frames back. To avoid clobbering a parent variable
    /// that the child only INHERITED (never wrote) — which the parent may have
    /// child CHANGED relative to this baseline.
    /// Immutable fork-time snapshots are shared across process resumptions.
    fork_baselines: HashMap<usize, Arc<Vec<HashMap<String, Value>>>>,
    /// Automatic collection storage retained by surviving fork children.
    fork_collection_captures: HashMap<usize, HashSet<String>>,
    /// Number of live children capturing each key in
    /// `fork_collection_captures`, so a cleanup check is one lookup.
    fork_capture_refs: HashMap<String, usize>,
    /// Live fork children holding each activation (frame generation), and
    /// the generations each child was indexed under.
    fork_gen_pids: HashMap<u64, HashSet<usize>>,
    fork_pid_gens: HashMap<usize, Vec<u64>>,
    /// Lexical block identities of each child's captured locals.
    fork_local_scopes: HashMap<usize, HashMap<u64, HashMap<String, u64>>>,
    deferred_collection_cleanup: HashSet<String>,
    /// IEEE 1800-2023 §6.21/§9.3.2: automatic variables declared in a
    /// procedural block (`initial`/`always`) with NO enclosing call frame live
    /// in the global `self.signals` map (not a `local_stack` frame). At fork
    /// time `inherit_fork_child_context` copies them (via `auto_loop_vars`)
    /// INTO the child's `local_stack` frame so each child gets its own
    /// snapshot. That creates a storage MISMATCH: the child's writes land in
    /// its private frame, while the parent reads from `self.signals`. The
    /// `local_stack` merge alone cannot bridge this — the parent has no frame
    /// for these vars. This map records, per child, the bare names that were
    /// captured FROM `self.signals`, so the propagate path can write the
    /// child's changed values back INTO `self.signals` (true shared storage,
    /// per §6.21's enclosing-scope rule).
    fork_signal_captures: HashMap<usize, Vec<String>>,
    /// Processes blocked in `semaphore.get()` on an under-full semaphore.
    semaphore_get_waiters: HashMap<usize, std::collections::VecDeque<SemGetWaiter>>,
    /// IEEE 1800-2017 §9.4.5 intra-assignment delay (`lhs = #d rhs`): RHS
    /// values captured at statement execution, keyed by the slot index baked
    /// into the synthesized `$__xz_intra_saved(idx)` continuation. Taken
    /// (removed) when the delayed assignment fires.
    intra_saved: HashMap<u64, Value>,
    intra_saved_next: u64,
    /// Armed values of value-change waits on class properties (see
    /// `VC_CHANGED_MARKER`); removed when the wait resumes.
    vc_saved: HashMap<u64, (usize, Value)>,
    /// LRM §15.5.3: per-named-event last-trigger time. `e.triggered`
    /// returns 1 iff its dense stamp matches the current simulation time
    /// (i.e. the event has been triggered in the same time slot as the
    /// query). EventTrigger handler stamps this.
    event_triggered_time: EventStampStore,
    /// Processes waiting for signal edge events (@(posedge clk), etc.),
    /// with the per-signal index a drain picks the ones to examine by.
    event_waiters: event_waiters::EventWaiters,
    /// `XEZIM_WAITER_INDEX` (default on, and only with the dirty-driven
    /// edge scan, whose write notes it shares): a drain examines the
    /// waiters whose signals were written, not every parked waiter.
    waiter_index_on: bool,
    /// `XEZIM_WAITER_INDEX_SHADOW=1`: examine every waiter anyway and stop
    /// on one the index would have skipped although its signal moved.
    waiter_index_shadow: bool,
    /// Processes parked on a class-field named event (`@m_event` inside a
    /// method on `this`). Woken directly by `fire_instance_event` when the
    /// matching `->m_event` runs; see `InstanceEventWaiter`.
    instance_event_waiters: InstanceEventWaiters,
    /// Monotonic epoch bumped each time a LATE-region resume (Re-NBA waiters,
    /// reactive initials, deferred clocking continuations) runs a continuation
    /// inline. The run_one_tick tail watches it to know those resumes produced
    /// activity that may need another edge/waiter pass in the SAME time slot
    /// (a bare signal flip or `-> e` schedules nothing by itself).
    late_resume_epoch: u64,
    /// Source file of each process's module, for assertion tallies.
    pid_src_file: HashMap<usize, Option<u32>>,
    /// Covergroups waiting for sampling events
    cg_event_waiters: Vec<(usize, Vec<SensitivityId>)>,
    /// Covergroups sampled on a named event (`@(ev)`), by the event's
    /// signal id: (instance, the event term's `iff`). Sampled when the
    /// event fires; a trigger right after a process wakes on an edge was
    /// missed by the edge scan.
    cg_named_event_waiters: HashMap<usize, Vec<(usize, Option<Expression>)>>,
    /// Class-embedded covergroups sampled on an event property of their
    /// object, by (object, event name).
    cg_instance_event_waiters: HashMap<(usize, String), Vec<(usize, Option<Expression>)>>,
    /// Reused per tick: the triggered continuations of
    /// `drain_triggered_event_waiters` and the clock generators' tree
    /// roots were fresh Vecs on every tick, and `drain_deferred_comb`
    /// hands its snapshot vectors back here for `note_comb_ran_in_process`.
    triggered_conts_buf: Vec<(usize, ProcCont)>,
    /// Scheduling depth of the comb-driven signals (see
    /// `build_sig_wake_rank`): one more per continuous assignment on the way
    /// from a procedurally written signal, which is depth 0 and not stored.
    /// Orders the waiters one drain wakes (`drain_triggered_event_waiters`).
    sig_wake_rank: HashMap<usize, u8>,
    wake_rank_built: bool,
    /// Depth of the input-port names collapsed onto their actual's id
    /// (`collapse_identity_port_nets`), which the id alone cannot carry.
    name_wake_rank: HashMap<String, u8>,
    /// Scratch: wake rank of each continuation in `triggered_conts_buf`.
    wake_rank_buf: Vec<u8>,
    cg_tree_roots_buf: Vec<usize>,
    deferred_comb_pool: Vec<Vec<(usize, Value)>>,
    /// Write set of the process FSM currently executing (see `ProcFsm::writes`).
    cur_fsm_writes: Option<Arc<[u32]>>,
    /// Per comb entry: (identity of the FSM write set last checked, verdict
    /// = the FSM can clobber this entry). The same pair recurs every cycle.
    fsm_clobber_cache: Vec<(usize, bool)>,
    /// VCD dump state
    /// `--wave`: waveform dumping was requested for this run. Latched from the
    /// global at construction so the model is built consistently — an active
    /// dump changes which loops can compile (see `for_loop_needs_ast`).
    wave: bool,
    vcd_file: Option<String>,
    #[serde(skip)]
    vcd_writer: Option<super::vcd_sink::VcdSink>,
    /// Per-traced-signal table: (signal_table index, output id code). Built
    /// once at dump start; the per-cycle loop walks just this vector instead
    /// of iterating the full id_to_name (36M entries on c910) and hashing
    /// every name. The identifier code is an `Arc<str>` so the VCD per-change
    /// clone is a refcount bump, not a heap allocation.
    vcd_trace: Vec<(usize, Arc<str>)>,
    vcd_enabled: bool,
    vcd_last_time: u64,
    /// Optional size limit for the VCD dump ($dumplimit, §21.7.1.5).
    /// Previous emitted value per entry in `vcd_trace` (parallel vector).
    /// A plain Vec<Value> — no duplicated name strings, no hashing.
    vcd_prev_signals: Vec<Value>,
    /// Optional scope/signal-name filters captured from `$dumpvars(level, sig...)`.
    /// When non-empty, only signals whose hierarchical name matches one of these
    /// scopes (or is a signal name given exactly) are emitted to the VCD. The
    /// strings are ABSOLUTE (they include the top module); `dump_filter_prefixes`
    /// rewrites them to the top-RELATIVE form the signal table uses.
    vcd_filter_scopes: Vec<String>,
    /// `$dumpvars` named arguments but not one of them was a usable name, so
    /// the VCD dump must select NOTHING. An empty `vcd_filter_scopes` means
    /// "no filter" = the WHOLE design, so "select nothing" has to be said out
    /// loud; without this a `$dumpvars` whose arguments all failed to parse
    /// failed open into a whole-design dump. Set by `$dumpvars`, cleared by
    /// `$vcdpluson` and by any `$dumpvars` that kept at least one argument.
    vcd_dump_none: bool,
    /// §21.7.1.4 `$dumpvars` depth argument: 0 = every level below the given
    /// scope, N = N levels starting at it.
    vcd_dump_depth: u32,
    /// §21.7.1.2: the `(scopes, depth)` selections of the `$dumpvars` calls
    /// executed before the latest one at the time the dump started. Each call
    /// adds its objects to the dump (`vcd_filter_scopes` and `vcd_dump_depth`
    /// hold the latest call's).
    vcd_earlier_selections: Vec<(Vec<String>, u32)>,
    /// The simulation time the VCD dump started at (`u64::MAX` before).
    vcd_start_time: u64,
    /// §21.7.2.1 var type per `vcd_trace` entry (parallel vector). Drives both
    /// the `$var` declaration and the runtime record shape (an `event` emits a
    /// bare `1<id>` pulse, a `real` an `r<decimal>` record).
    vcd_var_kinds: Vec<VcdVarKind>,
    /// Last sim time at which each `vcd_trace` entry that is an `event` emitted
    /// its trigger pulse — dedups repeat triggers inside one time slot.
    vcd_event_last: Vec<u64>,
    /// Incremental (dirty-set) change detection, SHARED by every dump writer
    /// (VCD, XTrace, FST). The full scan is O(all dumped signals) per timestep;
    /// for a large dump where few signals move per step (e.g. a 1.5M-signal
    /// flattened netlist) that dominates wall time — measured at ~19x the cost
    /// of the simulation itself on c906. Instead every real signal write marks
    /// the signal dirty (a SUPERSET of actual changes — each writer's
    /// prev-value compare still guarantees only genuine changes are emitted),
    /// so a flush visits O(written signals) not O(all). Coverage: both value
    /// write choke points (the `write_sig!` macro and `after_signal_write`)
    /// call `vcd_mark!`; SVA preponed swaps deliberately do not (they restore).
    /// `dump_dirty` is the per-flush list of dirty signal ids; `dump_dirty_mark`
    /// dedups within a flush via a rolling epoch (mark==epoch ⇒ already listed).
    /// The list is consumed by every active writer in `dump_write_changes` and
    /// cleared once, after the last of them — so the writers must READ it and
    /// never clear it themselves.
    /// `vcd_id_to_trace` / `xtrace_id_to_trace` / `fst_id_to_trace` map a signal
    /// id to that writer's (possibly aliased) trace slots.
    /// `vcd_event_indices` are the event trace slots, always checked (events
    /// fire via event_triggered_time, not signal writes). Disabled — falling
    /// back to the full scan — via XEZIM_VCD_FULL=1 / XEZIM_DUMP_FULL=1 or when
    /// the maps are unbuilt.
    dump_dirty: Vec<u32>,
    dump_dirty_mark: Vec<u64>,
    dump_dirty_epoch: u64,
    vcd_id_to_trace: Vec<Vec<u32>>,
    vcd_event_indices: Vec<usize>,
    dump_dirty_active: bool,
    /// A dump writer or toggle coverage is open under a forced full scan
    /// (`XEZIM_VCD_FULL=1` / `XEZIM_DUMP_FULL=1`), so `dump_dirty_active` is
    /// off with something still to flush.
    dump_full: bool,
    /// §21.7.1.8 `$dumplimit`: byte budget for the dump, and the running
    /// (approximate) count of bytes handed to the sink. `vcd_limit_hit` latches
    /// once the budget is spent and permanently stops the dump.
    vcd_limit: Option<u64>,
    vcd_bytes: u64,
    vcd_limit_hit: bool,
    /// XEZIM_BSP_SHADOW=1: number of time>0 settles compared so far.
    bsp_shadow_checks: u64,
    /// Buffered stdout sink for $display/$write. Lazily initialized on first
    /// write; runs its own background writer unless XEZIM_DUMP_INLINE=1.
    #[serde(skip)]
    stdout_sink: Option<super::stdout_sink::StdoutSink>,
    /// XTrace dump state. Independent from VCD — can run alongside.
    /// Set `xtrace_file` to enable. Emits the XTrace v1.0 "minimal" profile:
    /// dictionary + per-cycle signal deltas (VCD-equivalent payload). When the
    /// filename ends in `.zst`/`.zstd` the byte stream is zstd-compressed.
    pub xtrace_file: Option<String>,
    /// Optional hierarchical scope filters for XTrace. When non-empty, only
    /// signals matching one of these (exact, or `<scope>.` prefix) are dumped.
    pub xtrace_scopes: Vec<String>,
    #[serde(skip)]
    xtrace_writer: Option<super::vcd_sink::VcdSink>,
    /// Per-traced-NET table: (signal_table index, XTrace signal id). One entry
    /// per backing net — an aliased name (§9.2 `alias=`) has its own id in the
    /// dictionary but emits no deltas of its own.
    xtrace_trace: Vec<(usize, Arc<str>)>,
    /// Signal id → `xtrace_trace` slots, for the shared incremental dirty-set
    /// change detection (see the `dump_dirty` field docs). Empty ⇒ full scan.
    xtrace_id_to_trace: Vec<Vec<u32>>,
    /// Previous emitted value per entry in `xtrace_trace` (parallel vector).
    xtrace_prev_signals: Vec<Value>,
    /// Whether entry `i` of `xtrace_trace` is a `real` (parallel vector): its
    /// value token is a decimal number, not a hex bit pattern (§15.1).
    xtrace_real: Vec<bool>,
    /// Whether entry `i` of `xtrace_trace` is a `string` (parallel vector): its
    /// value token is a §15.4 quoted, escaped string rather than a bit pattern.
    xtrace_string: Vec<bool>,
    /// The t=0 `N,full` snapshot is emitted lazily, on the first in-window
    /// `xtrace_write_changes` call — by then the time-0 initial blocks have run
    /// and settled, so the checkpoint carries real values (matching the
    /// reference producer) instead of the pre-init all-X image.
    xtrace_snapshot_pending: bool,
    /// SV `event` objects (signal_table index, XTrace signal id). An event has
    /// no level, so it is NOT in `xtrace_trace`: it emits an §10.4 `X` record
    /// per trigger instead of a value delta.
    xtrace_events: Vec<(usize, Arc<str>)>,
    /// Time of the last `X` record emitted for each `xtrace_events` entry, so a
    /// 0→1→0 toggle inside one time slot still counts as exactly one trigger
    /// (mirrors `vcd_event_last`).
    xtrace_event_last: Vec<u64>,
    xtrace_last_time: u64,
    /// Change-emitting steps since the last durable flush of the XTrace
    /// writer. Bounds how much trailing trace a crash/SIGKILL can lose
    /// (the binary builds with `panic = "abort"`, so `Drop` does not run on
    /// panic, and the memory watchdog SIGKILLs outright).
    xtrace_dirty_steps: u32,
    /// XTrace time window (#8) in nanoseconds, from `--xtrace-from`/`--xtrace-to`.
    /// `xtrace_to_ns == u64::MAX` means no upper bound. Set by the CLI.
    pub xtrace_from_ns: u64,
    pub xtrace_to_ns: u64,
    /// The same window converted to simulation ticks (tick_s units), cached at
    /// dump start. `xtrace_to_t == u64::MAX` means unbounded.
    xtrace_from_t: u64,
    xtrace_to_t: u64,
    /// FST dump state (GTKWave binary format via the `fst-writer` crate).
    /// Independent of VCD/XTrace — enabled by `--fst <file>`.
    pub fst_file: Option<String>,
    /// Optional hierarchical scope filters for FST (same semantics as
    /// `--xtrace-scope`): keep signals whose name equals or sits under a scope.
    pub fst_scopes: Vec<String>,
    /// `$fsdbDumpvars` named arguments but not one of them was a usable name:
    /// the FST dump must select nothing rather than fail open into a
    /// whole-design dump. Same purpose as `vcd_dump_none`; see there.
    pub fst_dump_none: bool,
    /// The waveform EXCLUDE list (`--wave-suppress`), applied by every dump
    /// format from one matcher — see `waveforms::WaveSuppressor`. Entries are
    /// hierarchical paths or module names, resolved against the elaborated
    /// instance tree at dump time.
    pub wave_suppress: Vec<String>,
    /// The FST body writer (post-header phase). `None` until `fst_start_dump`.
    /// Value packing, block compression and I/O run on a background thread
    /// (`XEZIM_DUMP_INLINE=1` keeps them here); see `fst_sink::FstSink`.
    #[serde(skip)]
    fst_writer: Option<super::fst_sink::FstSink>,
    /// Named-cast targets resolvable at COMPILE time: name -> (width,
    /// signed). Built once at construction; handed to every bytecode
    /// compiler so `type'(expr)` compiles instead of falling back.
    cast_widths: HashMap<String, (u32, bool)>,
    /// §7.4.1: variables declared as a packed array of a named signed type
    /// (`lane_t [0:8] x`), with the number of selects that reaches that
    /// element type (see `bytecode::signed_named_packed_elem_depth`). Built
    /// at construction from the declared types; empty in most designs, which
    /// keeps the element-read check to one length test.
    #[serde(skip)]
    packed_named_signed: HashMap<String, usize>,
    /// The same for class properties, by property name: the names some
    /// class declares as a packed array of a named signed type.
    #[serde(skip)]
    packed_named_signed_props: HashMap<String, usize>,
    /// The `Index` expression whose signed element read is in progress (its
    /// address), so the re-entrant evaluation reads the slice itself.
    #[serde(skip)]
    packed_named_sign_guard: usize,
    /// §7.4.1: whether the design declares a signed integral type by name
    /// at all. Only then can a subroutine or block variable be a packed
    /// array whose elements are signed, so only then does an element select
    /// of one consult its declaration.
    packed_named_signed_types: bool,
    /// §7.2, §7.4.1: names of structure members that are packed arrays of a
    /// named signed type (see its construction).
    packed_named_signed_members: HashSet<String>,
    /// The arithmetic or bitwise operand (its address) being evaluated
    /// whose parent operator found no context-determined shift in it
    /// (`has_context_shift`), so its own operator does not scan its operands
    /// again: each level of a chain `a ^ b ^ ...` scanned the whole chain
    /// under it.
    #[serde(skip)]
    shift_free_operand: usize,
    /// `module.functions` holds each `FunctionDeclaration` by value, so the
    /// obvious `get(name).cloned()` deep-copies a whole function AST — body,
    /// statements and all — on EVERY call. On a design that calls a function
    /// from a continuous assign that is the dominant cost: cloning the decl
    /// and the allocator traffic behind it accounted for roughly a third of
    /// the run. Hand out a refcounted handle instead; the deep copy happens
    /// once per function, then it is a refcount bump.
    fn_decl_cache: HashMap<String, std::rc::Rc<FunctionDeclaration>>,
    /// class-perf P2: one-shot Rc snapshot of `module.functions` for the
    /// method compiler (free-function calls). Built lazily on the first
    /// class-method compile; cloned per call site by Rc only.
    free_fn_table: Option<std::rc::Rc<HashMap<String, crate::ast::decl::FunctionDeclaration>>>,
    /// `class_handle_member_types` over every class, shared by method
    /// compiles; cleared with `compiled_method_admission` when a class is
    /// added.
    class_handle_member_types_cache: Option<std::rc::Rc<HashMap<String, HashMap<String, String>>>>,
    /// Every class-scoped enum literal as `Class::MEMBER` -> value
    /// (`class_enum_member`), for compiled method bodies that name another
    /// class's literal (`pdata::NONE`). Built on first use.
    class_scoped_enum_cache: Option<std::rc::Rc<HashMap<String, Value>>>,
    /// DPI import function prototypes by SV name, built once on the first
    /// method compile (see `dpi_import_fd_table`).
    dpi_import_fds: Option<std::rc::Rc<HashMap<String, crate::ast::decl::FunctionDeclaration>>>,
    /// Package name ("" for none) -> interned `__free_fn__<pkg>` class name.
    free_fn_class_names: HashMap<String, std::rc::Rc<str>>,
    /// Call name -> package its callee runs in (see `free_call_pkg`).
    free_call_pkgs: HashMap<String, Option<std::rc::Rc<str>>>,
    /// The same for `module.tasks`: every interpreted task call cloned the
    /// whole `TaskDeclaration`, and the clone plus its drop was about a
    /// quarter of a behavioural memory model's run (a device calling a
    /// handful of timing-check tasks every clock).
    task_decl_cache: HashMap<String, std::rc::Rc<TaskDeclaration>>,

    /// Memoized answer to "is this module-scope function side-effect free?",
    /// used to decide whether a call may be skipped when its result provably
    /// cannot affect the expression. Conservative: anything the walker does
    /// not positively recognise as safe is recorded impure.
    fn_pure_cache: HashMap<String, bool>,
    /// Lazily-built index over `module.packed_signal_elem_widths`: every
    /// dot-terminated prefix of every key, i.e. exactly the set of names for
    /// which some `name.<rest>` key exists. Formal-metadata bookkeeping runs
    /// on EVERY function/task call and used to answer that question by
    /// linear-scanning the whole map — three times per call (snapshot,
    /// clear, and clear again inside restore). The map scales with the
    /// DESIGN (packed arrays and struct members everywhere), so on a
    /// 55k-signal design a single assign with four function calls on its
    /// RHS cost ~10ms per evaluation. With the index, a formal with no
    /// dotted keys — every non-struct formal — is O(1).
    ///
    /// Invariant: MAY over-approximate (stale entries only cost a scan),
    /// must NEVER under-approximate — every insertion of a dotted key goes
    /// through `note_elem_width_key`. Built on first use so every
    /// construction path (fresh elaboration, artifact load) is covered.
    elem_dotted_bases: RefCell<Option<HashSet<String>>>,
    /// Per-net trace: (signal-table index, (FST handle, immutable encoding width)).
    /// A zero encoding width denotes a real; fixed vectors use at least one.
    #[serde(skip)]
    fst_trace: Vec<(usize, (FstSignalId, u32))>,
    /// Signal id → `fst_trace` slots, for the shared incremental dirty-set
    /// change detection (see the `dump_dirty` field docs). Empty ⇒ full scan.
    fst_id_to_trace: Vec<Vec<u32>>,
    /// Previous emitted value per `fst_trace` entry (parallel vector).
    fst_prev_signals: Vec<Value>,
    /// SV event objects are pulses rather than levels, so they are kept out of
    /// `fst_trace` and emitted once for each trigger.
    #[serde(skip)]
    fst_events: Vec<(usize, FstSignalId)>,
    /// Last emitted trigger time for each `fst_events` entry.
    fst_event_last: Vec<u64>,
    /// Pre-computed combinatorial entries with sensitivity sets.
    comb_entries: Vec<CombEntry>,
    /// Optional content-addressed cache for the compiled combinational
    /// worklist. The elaborated-design cache key is embedded in this path.
    prepared_comb_cache_path: Option<PathBuf>,
    /// Static topological level per comb entry (longest producer chain).
    /// Populated when the level-BSP settle path is enabled; entries within one
    /// level write disjoint signals (no read-after-write), so they can be
    /// evaluated in parallel behind a per-level barrier. Empty = not built.
    comb_level: Vec<u32>,
    /// Per comb entry: true if its compiled body contains NBA insns. The
    /// isolated settle eval (`eval_comb_entry_isolated`) collects deferred NBAs
    /// into a local and DROPS them, whereas canonical `exec_insns` schedules
    /// them into the real NBA queue. So the BSP driver must route these through
    /// the full `&mut self` path. Built alongside comb_level.
    comb_has_nba: Vec<bool>,
    /// Per comb entry: it reads a signal it also writes (a packed vector
    /// whose reads survive write exclusion), so its own writes must not
    /// re-queue it. Other drivers of the vector still do. Rebuilt with the
    /// levels and whenever its length differs from `comb_entries`.
    comb_self_exempt: Vec<bool>,
    /// Per comb entry: a level-sensitive `always` whose writes no other
    /// entry reads (an observer, `always @* assert (...)`), which the settle
    /// runs only once every other entry has converged (`build_comb_observer`).
    /// `comb_has_observer`: any is set. Rebuilt whenever its length differs
    /// from `comb_entries`.
    comb_observer: Vec<bool>,
    comb_has_observer: bool,
    /// Per comb entry: evaluable by the static isolated path (no `&mut self`,
    /// disjoint writes) — not AST, no NBA, no unsupported insn. Only par_safe
    /// entries run on worker threads in the parallel BSP level dispatch.
    comb_par_safe: Vec<bool>,
    /// Reusable per-level buckets for the BSP settle driver (one Vec per static
    /// level). Taken out/restored each settle call so capacity is retained —
    /// avoids re-allocating ~num_levels Vecs on every call. Inner Vecs are
    /// cleared as each level is drained.
    bsp_buckets: Vec<Vec<usize>>,
    /// Reusable "re-trigger for next sweep" list for the BSP driver.
    bsp_next_seed: Vec<usize>,
    /// XEZIM_BSP_WIDTHS=1 diagnostic: histogram of per-level evaluated-entry
    /// counts during time>0 (steady-state) settles, bucketed by width
    /// [1, 2-15, 16-63, 64-255, 256-1023, 1024+]. Decides whether intra-level
    /// parallelism has enough width to amortize dispatch in steady state.
    bsp_width_buckets: [u64; 6],
    bsp_widths_on: bool,
    /// XEZIM_BSP_SETTLE=1: route combinational settle through the level-BSP
    /// wavefront driver instead of the canonical chaotic-iteration worklist.
    bsp_settle: bool,
    /// XEZIM_BSP_PAR=1: evaluate each level's par_safe entries across worker
    /// threads (intra-level parallelism). Requires bsp_settle.
    bsp_par: bool,
    /// Minimum par_safe entries in a level before the parallel dispatch fires
    /// (smaller levels run serial — dispatch wouldn't amortize). XEZIM_BSP_PAR_THRESHOLD.
    bsp_par_threshold: usize,
    /// XEZIM_BSP_SHADOW=1: per-settle compare BSP vs canonical, abort on diff.
    bsp_shadow: bool,
    /// Precomputed indices of comb_entries with has_unresolved_reads=true.
    /// Built once alongside comb_entries; scanning this short list every
    /// settle call is vastly cheaper than iterating all num_entries bits
    /// (can be ~500K on designs like c910).
    comb_unresolved_idx: Vec<usize>,
    /// Co-activation census (`XEZIM_COACT_CENSUS=1`): per-entry predecessor
    /// CSR over the dep graph, a same-slot counter per (entry, pred) edge,
    /// and the last simulated time each entry evaluated. Sizing data for
    /// convex-cluster fusion: an entry whose evals are >=90% same-slot with
    /// one predecessor can join that predecessor's cluster with bounded
    /// waste — even across fanout. Empty unless the env is set.
    coact_pred_off: Vec<u32>,
    coact_pred_ent: Vec<u32>,
    coact_edge_count: Vec<u64>,
    coact_epoch: Vec<u64>,
    coact_enabled: bool,
    /// Precomputed indices of comb_entries that should fire unconditionally
    /// at time=0: entries with empty read_signal_ids, or always_comb blocks.
    /// Avoids an O(num_entries) scan on every settle call while time=0.
    comb_time0_idx: Vec<usize>,
    /// Latches once `comb_time0_idx` has been seeded into settle_triggered.
    /// Every subsequent settle call at time=0 can skip the re-seed; the
    /// worklist and dirty propagation already keep things moving.
    comb_time0_fired: bool,
    /// §9.2.2.2: always_comb blocks with an EMPTY inferred sensitivity list
    /// (no inputs, e.g. `always_comb y = 1'b0;`). The LRM says these trigger
    /// exactly once at time zero, AFTER all initial/always procedures have
    /// STARTED (run to their first delay). Firing them in the pre-initial
    /// settle would let an initial block read the output as already-settled;
    /// a reference simulator (and the standard) has that same-time pre-delay read return X.
    /// So we hold these out of the normal time-0 settle and fire them once at
    /// the END of the time-0 active region (see run_one_tick). Cloned entries,
    /// evaluated directly.
    comb_time0_deferred: Vec<CombEntry>,
    /// Latches once the deferred empty-sensitivity always_comb blocks fired.
    comb_time0_deferred_done: bool,
    /// §9.2.2.2: the entries with time-0 effects and the items taken out of
    /// them (a `Noop` stands in) until every process has started; see
    /// `hold_time0_effect_entries`.
    comb_time0_held: Vec<(usize, CombItem)>,
    /// Reverse index: signal_id → list of comb_entry indices that read
    /// this signal. CSR layout — `comb_dep_entries[comb_dep_offsets[id]
    /// .. comb_dep_offsets[id+1]]` gives the dependents of signal `id`.
    /// Compared to the prior `Vec<Vec<usize>>` this drops 24 B / signal
    /// of empty-Vec-header overhead (≈14 MB on c910's 585K signals)
    /// and packs the entry indices contiguously for better cache use
    /// in `settle_combinatorial`'s hot loop. u32 is sufficient — both
    /// signal_id and comb_entry index fit comfortably in 32 bits at
    /// any practical design size.
    comb_dep_offsets: Vec<u32>,
    /// Lazily-lowered two-state bodies for comb entries (P5): indexed by
    /// entry, `Untried` until first evaluation. Cleared whenever the entry
    /// list is rebuilt.
    ts_comb: Vec<TsSlot>,
    /// Per-comb-entry settle dispatch plan, resolved on first evaluation.
    /// Collapses the per-eval chain (env OnceLock, TsSlot bounds+match,
    /// comb_jit_fns bounds+copy, strike bookkeeping) — measured 15% of a
    /// JIT'd ibex run as settle-loop self time — into one enum load.
    /// `ts_comb`/`comb_jit_fns` stay the sources of truth at resolution;
    /// a JIT strike-out downgrades the plan in place.
    comb_plan: Vec<CombPlan>,
    /// Two-state code arena: every lowered comb stream appended contiguously
    /// (in first-evaluation order, which tracks the worklist order), plus a
    /// dense per-entry header. `kind == 0` means "no arena stream".
    ts_arena: Vec<super::bytecode::TsInsn>,
    ts_hdr: Vec<TsHdr>,
    /// Same, for edge blocks (indexed like `compiled_edge_blocks`).
    ts_edge: Vec<TsSlot>,
    /// Two-state scratch register file (u64 words).
    ts_regs: Vec<u64>,
    /// x/z planes of `ts_regs` for the x-plane executor.
    ts_xregs: Vec<u64>,
    /// Per arena header: the stream is x-plane runnable (`TwoStateBlock::tsx`).
    ts_hdr_tsx: Vec<u8>,
    /// x-plane executor enabled (`XEZIM_TS_X=0` disables).
    tsx_on: bool,
    prof_tsx_evals: u64,
    /// Wide (65..=128-bit) two-state register bank; same index space as
    /// `ts_regs` (a register's bank is static, decided at lower time).
    /// Wide two-state banks, flat words viewed as `[u64; N]` registers:
    /// N = 2 (widths ≤ 128) and N = 8 (≤ 512).
    ts_wregs: Vec<u64>,
    ts_wregs8: Vec<u64>,
    /// Two-state eval hits, for the profile report.
    prof_ts_evals: u64,
    /// Why the two-state engine declined an eval — reported so a profile
    /// from a real design says WHY islands are inert instead of just "0".
    prof_ts_bail_warnx: u64,
    prof_ts_bail_forced: u64,
    prof_ts_bail_xread: u64,
    prof_ts_bail_abort: u64,
    /// Set by `ts_guard_and_exec`: the last `false` came from a MID-EXEC
    /// abort (partial ts work + full 4-state replay), not a cheap guard
    /// bail. Callers use it to DEMOTE chronically aborting blocks — the TS
    /// NBA admission exposed blocks whose guards pass but whose bodies hit
    /// an abortable op (blocking reads of X RAM data) on most evals: 67 M
    /// aborts/run, each paying guard + partial exec + replay, a net LOSS vs
    /// never admitting. Demotion returns such a block to exactly its
    /// pre-admission engine; clean runners keep the fast path.
    ts_exec_aborted: bool,
    /// Set by a two-state executor that read an x/z bit; the guard reports
    /// it as an x-read bail (no demotion), not as an abort.
    ts_xread_bail: bool,
    /// `TsInsn::SaveSig` records (signal, value bits, x bits) for the
    /// evaluation in progress; restored on a bail, truncated on success.
    ts_save_list: Vec<(u32, u64, u64)>,
    /// Comb entry whose two-state stream is executing (u32::MAX for an
    /// edge block, whose runner sets the scope itself): a `TsInsn::Fallback`
    /// sets the `%m` buffer and the resolve hint from its scope, as the
    /// four-state comb path does before evaluating an entry.
    ts_cur_eidx: u32,
    /// The settle takes `comb_entries` out of `self` for its duration; this
    /// is a view of the taken vector so `ts_fallback_scope` can still read
    /// an entry's scope hint. Null outside the settle.
    #[serde(skip)]
    settle_entries_view: (*const CombEntry, usize),
    /// Census: x-read bails per comb entry (count, executor kind).
    #[cfg(feature = "opcode-census")]
    ts_xbail_by_entry: HashMap<usize, (u64, u8)>,
    /// Per-slot mid-exec abort counts; at `TS_ABORT_DEMOTE` the slot flips
    /// to No / Interp for the rest of the run. (Re-promotion after init
    /// settles is future work.)
    ts_edge_abortn: Vec<u16>,
    /// x-read bail backoff. A block that bails on an x read every time (an
    /// unwritten memory element it keeps reading) paid the executor entry
    /// and the guard on every evaluation before running interpreted anyway.
    /// After `TS_XBAIL_STREAK` bails in a row it is put to SLEEP: its plan
    /// or slot is swapped for the demoted form, so the hot path pays
    /// nothing for it, and a record on this heap (keyed by the successful
    /// two-state evaluation count) wakes it later, sleeping twice as long
    /// each time the bails continue. Per-entry streak state is touched only
    /// on a bail; a long clean stretch resets it.
    ts_sleep: std::collections::BinaryHeap<std::cmp::Reverse<(u64, u32)>>,
    ts_sleep_recs: Vec<Option<TsSleepRec>>,
    ts_sleep_free: Vec<u32>,
    /// `(streak, last bail at, probe until)` per comb entry / edge block.
    /// `probe until` is set at wake-up to one evaluation cadence of the
    /// block (measured while it slept): a bail before it means the first
    /// attempt after waking still read x, and the streak escalates; a bail
    /// after it means the block ran clean in between, and the streak
    /// restarts.
    ts_comb_xb: Vec<(u8, u64, u64)>,
    ts_edge_xb: Vec<(u8, u64, u64)>,
    /// Interpreted evaluations of a SLEEPING block, plus one (0 = awake):
    /// counted on the interpreter paths, which a sleeping block takes.
    ts_asleep_comb: Vec<u32>,
    ts_asleep_edge: Vec<u32>,
    /// Set by `ts_guard_and_exec` when the failure was an x read.
    ts_last_xbail: bool,
    ts_comb_abortn: Vec<u16>,
    comb_plan_abortn: Vec<u16>,
    /// XEZIM_PROFILE_REPORT=1: reference-style ranked profile (by design
    /// unit / instance / construct). Implies per-entry and per-edge-block
    /// nanosecond attribution.
    profile_report: bool,
    /// XEZIM_PROC_FSM: 1 = every compilable initial body is a process FSM,
    /// 0 = none, unset (2) = the plain-stimulus shape only. Read per
    /// simulator so a test can opt out before it builds one.
    proc_fsm_mode: u8,
    comb_dep_entries: Vec<u32>,
    /// `comb_dep_entries` with bit-granular sensitivity, for the settle
    /// loop: each dependent with the bits of the signal it reads. A write
    /// inside the settle loop whose changed bits miss the mask does not
    /// trigger that dependent — `assign lo = bus[3:0]` stays idle while
    /// `bus[7:4]` moves. Empty until the first settle; entries narrow as
    /// they lower two-state.
    comb_dep_edges: Vec<DepEdge>,
    /// Entries lowered since the masks were last refined.
    bit_sens_pending: Vec<u32>,
    /// XEZIM_BIT_SENS=0 turns the masks off (every dependent triggers).
    bit_sens_on: bool,
    /// Controlling-value skip (`CtlGate`): the gates, indexed by the
    /// `CombItem::CtlGate` slot. Rebuilt with the dependency edges.
    ctl_gates: Vec<CtlGate>,
    /// Per signal: some dependency edge from it may be masked by a
    /// `CtlGate`, so the settle's dirty seed must test the edge masks.
    /// Empty when no gate qualifies.
    ctl_sig: Vec<bool>,
    /// `(destination signal, slot)` of every `CtlGate`, sorted: an external
    /// write to a destination (a deposit) must re-arm its gate.
    ctl_dsts: Vec<(u32, u32)>,
    /// Slots re-armed while the settle held the dependency edges.
    ctl_rearm_pending: Vec<u32>,
    /// XEZIM_CTL_MASK=0, or a settle mode that evaluates entries outside the
    /// canonical loop (partitioned / level-BSP), turns the skip off.
    ctl_on: bool,
    mux_guard_on: bool,
    mux_guards: Vec<MuxGuard>,
    mux_guard_by_entry: Vec<Option<std::num::NonZeroU32>>,
    prof_mux_skipped: u64,
    ts_mux_stream_on: bool,
    settle_pop_word: bool,
    ts_range_delta: bool,
    /// Dirty records of the stores a two-state entry makes in the settle
    /// loop (direct mode), with each store's changed bits (`u64::MAX` when
    /// unknown): `ts_rec_n` records, the first in `ts_rec0` (most entries
    /// store one signal), the rest in `ts_recs`. Kept apart from
    /// `dirty_list` so every record carries its own mask; the settle loop
    /// consumes them right after the entry.
    ts_rec0: (usize, u64),
    ts_rec_n: usize,
    ts_recs: Vec<(usize, u64)>,
    /// Bitvec: dirty_signals[signal_id] = true if signal changed since last settle.
    dirty_signals: Vec<bool>,
    /// Explicit list of dirty signal IDs (maintained alongside dirty_signals bitvec)
    /// This avoids O(num_signals) scan in settle_combinatorial.
    dirty_list: Vec<usize>,
    dirty_any: bool,
    /// Set while the settle loop runs a two-state entry: its stores append
    /// to `dirty_list` directly, without the `dirty_signals` dedup flag (a
    /// random access into a 35M-entry table per store on c906). The loop
    /// re-reads and clears that span right after the entry, and
    /// `trigger_deps!` dedups by entry, so duplicates are harmless.
    ts_direct_writes: bool,
    /// When true, signal_table has been modified and signals HashMap is stale.
    table_modified: bool,
    /// Scratch buffer reused across event_loop iters: positions (into
    /// `edge_signal_ids`) of clock signals that toggled during the current
    /// iter's `fire_clock_generators`. When the iter ends up doing no other
    /// work (no NBA apply, no settle, no process, no delayed update), the
    /// edge-detect scan can be restricted to just these positions — cuts
    /// per-iter edge_detect work from O(|edge_signal_ids|) ~10k to ~|clocks|.
    /// See `event_loop` for the dispatch.
    toggled_clock_positions: Vec<usize>,
    /// XEZIM_EDGE_SCAN_STATS=1 diagnostic: edge-detect positions scanned vs
    /// positions that actually changed (fired). Measures the dirty-driven
    /// detect ceiling before building it.
    edge_scan_scanned: u64,
    edge_scan_changed: u64,
    edge_scan_stats: bool,
    /// XEZIM_ARRAY_SOA_SHADOW=1 (with XEZIM_INLINE_BITS=1): redirect array-cell
    /// READS to the compact signal_inline_bits store and assert they match the
    /// canonical signal_table — Stage-0 validation toward dropping the 32-byte
    /// Value array cells. signal_table stays canonical (safe).
    soa_shadow: bool,
    /// Per-signal eligibility for the SoA read fast path: width ≤ 64, not
    /// real, not string. Built alongside `signal_inline_bits` (empty ⇒ the
    /// path is off and every read falls back to `signal_table`).
    soa_read_ok: Vec<bool>,
    /// XEZIM_OPCODE_CENSUS=1: dynamic opcode + adjacent-pair execution counts
    /// for fusion planning. `census_counts[op]`, `census_pairs[prev*64+op]`.
    census_enabled: bool,
    census_counts: Vec<u64>,
    census_pairs: Vec<u64>,
    /// Two-state executor census (opcode-census builds only): adjacent
    /// opcode pairs and singles by variant name.
    #[cfg(feature = "opcode-census")]
    ts_census: std::collections::HashMap<(&'static str, &'static str), u64>,
    /// Clock-tree dedup (see `build_edge_sig_groups`). `edge_group_of` is
    /// parallel to `edge_signal_ids`: the group a POSITION belongs to, or
    /// `u32::MAX` for a signal that must be checked on its own. Members of one
    /// group are exact copies of a common driver, so the first member scanned in
    /// a pass computes the edge and the rest read it out of `edge_group_memo`.
    /// The memo is stamped with `edge_group_generation` (bumped once per detect
    /// pass) so it never needs clearing.
    edge_group_of: Vec<u32>,
    edge_group_memo: Vec<EdgeGroupMemo>,
    edge_group_generation: u64,
    edge_group_count: usize,
    /// `XEZIM_CLKTREE_PROBE=1`: instead of trusting the memo, recompute every
    /// member and count how often the leader's answer disagreed. The dedup is
    /// only sound if this is zero, and it is cheaper to measure than to argue.
    clktree_probe: bool,
    clktree_probe_hits: u64,
    clktree_probe_mismatch: u64,
    /// Signal ids of the first few probe mismatches, resolved to names in the
    /// summary (the detect loop holds a mutable borrow and cannot name them).
    clktree_probe_sids: Vec<usize>,
    /// Dirty-driven edge detect (XEZIM_DIRTY_EDGE=1). `sig_to_edge_pos[id]` =
    /// position of signal `id` in `edge_signal_ids`, or -1 if not edge-sensitive.
    /// Writes to edge-sensitive signals (via write_sig!/after_signal_write)
    /// push their position into `changed_edge_pos` (deduped by `edge_pos_seen`),
    /// so check_edges can scan only changed positions instead of all of them.
    sig_to_edge_pos: Vec<i32>,
    changed_edge_pos: Vec<usize>,
    /// Retained buffer that becomes `changed_edge_pos` while a scan runs, so
    /// positions written during the scan do not grow a fresh Vec every pass.
    edge_pos_scratch: Vec<usize>,
    edge_pos_seen: Vec<bool>,
    dirty_edge: bool,
    dirty_edge_shadow: bool,
    /// Edge positions written by a BLOCKING assign while an edge-triggered
    /// block was executing (`in_edge_block`), deduped via `edge_exec_seen`.
    /// §9.2: such a change must (re-)trigger blocks sensitive to it —
    /// `drain_edge_exec_rescan` re-detects exactly these positions in a
    /// follow-up delta pass instead of silently coalescing them away.
    edge_exec_wrote: Vec<usize>,
    edge_exec_seen: Vec<bool>,
    /// Nesting depth of a scheduled PROCESS activation. §4.5: a blocking write
    /// schedules the blocks sensitive to it in the ACTIVE region; the writing
    /// process keeps running until it suspends, and only then do they execute.
    /// The blocking-assign arm used to settle INLINE, so an `always @(*)` ran
    /// in the middle of another process's statement sequence — and if that
    /// process later wrote the comb block's target, the comb result was
    /// clobbered and never recomputed (`initial begin a=0; b=1; g=0; end`
    /// against `always @(*) if (a^b) g=…` left g at 0 instead of the value).
    /// While this is non-zero, procedural comb entries triggered mid-process
    /// are parked in `deferred_comb` and run at the process's exit instead.
    proc_depth: u32,
    /// Nesting of `run_process_stmts` activations on the native stack. A
    /// process that calls a task which can wait but does not resumes the
    /// caller from inside the callee's activation, so a call chain that never
    /// waits nests one level per call (see `process_nesting_exceeded`).
    stmts_depth: u32,
    /// (comb entry index, outputs it produced) for procedural entries that ran
    /// while a process was mid-flight. Checked for clobbering at process exit.
    deferred_comb: Vec<(usize, Vec<(usize, Value)>)>,
    /// True while `drain_edge_exec_rescan` is re-running check_edges for
    /// exec-written positions; makes check_edges_inner count block
    /// executions into `edge_rescan_block_hits` for stall attribution.
    in_edge_rescan: bool,
    /// block index -> executions during the current rescan drain. Read by
    /// `report_edge_rescan_stall` to name the ping-ponging always blocks.
    edge_rescan_block_hits: HashMap<usize, u64>,
    /// Edge-block executions accumulated while simulated time is not
    /// advancing. Unlike `edge_rescan_block_hits`, this spans outer scheduler
    /// iterations so progress/recovery diagnostics can identify a delta-cycle
    /// loop whose writes are separated by NBA or process scheduling phases.
    stall_edge_block_hits: HashMap<usize, u64>,
    /// Counter — number of iters whose check_edges ran over just the
    /// toggled clock subset instead of the full edge_signal_ids scan.
    prof_clocks_only_detect: u64,
    /// Lookup parallel to `signal_table`: true iff this signal id is in
    /// `edge_signal_ids` AND is NOT a clock generator's output.  Used by the
    /// clocks-only fast-path to decide whether an NBA write actually moved
    /// a signal whose change could be missed if check_edges scans only the
    /// toggled clocks.  Built once at the end of elaboration.
    is_edge_signal_non_clock: Vec<bool>,
    /// Per-iter scratch flag set by `write_sig!` whenever a write targets a
    /// non-clock edge signal — regardless of whether the value actually
    /// changes (most callers gate write_sig! on a prior equality check, so
    /// no-op writes don't reach the macro).  Also set in the parallel
    /// apply_nba path (which bypasses the macro).  Cleared at the top of
    /// each `event_loop` iter; read just before check_edges to decide
    /// between the full scan and the clocks-only fast path.
    nba_touched_edge_non_clock: bool,
    /// Counter — number of `Insn::NbaAssign` evaluations whose value
    /// already matched `signal_table[sig_id]` and were therefore not
    /// pushed to `nba_fast`.  Captures the no-op fraction of NBA writes
    /// (~70% on c910 flop-heavy workloads) before they incur queue/apply
    /// overhead.  Counted in the main interpreter path only; the
    /// parallel `exec_insns_isolated` path also elides but doesn't bump.
    prof_nba_elided: u64,
    /// In profile mode, wide partial NBAs that skipped or needed a new queue value.
    prof_wide_nba_quiet: u64,
    prof_wide_nba_cloned: u64,
    // O1 event-driven-edge MEASUREMENT (XEZIM_EVENT_EDGE_MEASURE=1, no
    // behavior change): per-edge-block static DATA-read set (LoadSignal sids
    // minus sensitivity), whether the block is gateable (no dynamic reads),
    // a "changed since last main-clock posedge" accumulator, and counters for
    // how many main-clock flops WOULD be skipped (no data input changed).
    event_measure: bool,
    // event_skip: the REAL skip (XEZIM_EVENT_EDGE=1) — actually drop gateable
    // flop fires whose data didn't change. event_measure just counts.
    event_skip: bool,
    // Skip only at sim_time >= this (XEZIM_EVENT_AFTER). Boot/load uses write
    // paths ($readmemh, testbench drives, DPI) that may bypass the change
    // hook; running boot WITHOUT skip avoids that false-skip window. The 99%
    // waste is in steady state.
    event_after: u64,
    edge_block_data_reads: Vec<Vec<u32>>,
    edge_block_gateable: Vec<bool>,
    /// Gateable only through write-arming: the block reads/writes a dense
    /// unpacked array with a dynamic index, and every element of that array
    /// is registered as an arm-only input (no value snapshot). Once armed it
    /// always executes; the snapshot compare cannot see element changes.
    edge_block_arm_only: Vec<bool>,
    /// Gateable block with a >64-bit data read: no compact value snapshot
    /// exists, so under write-arming an UNARMED block skips on arming alone
    /// (no input write since its last execution ⇒ no change).
    edge_block_wide_read: Vec<bool>,
    /// XEZIM_ARM_CENSUS=1: per-signal (writes, block-arms) through note_armed_write.
    arm_census: Option<Vec<(u64, u64)>>,
    // Timestamp-based change tracking (gating-agnostic): per-signal iteration
    // at which it last changed, and per-edge-block iteration at which it last
    // fired. A flop is skippable iff none of its data inputs changed since the
    // flop's previous fire (max data last-change <= flop last-fire).
    sig_last_change: Vec<u64>,
    flop_last_fire: Vec<u64>,
    // Snapshot-compare skip (correct-by-construction, no change-hook dependency):
    // per gateable edge-block, the data-input signal VALUES captured at its last
    // snapshot-verified fire. Epoch-fast executions invalidate this snapshot;
    // one unconditional execution rebuilds it before skipping is possible. On
    // the next verified edge we compare CURRENT values directly and skip iff
    // bit-identical (Q provably unchanged). Reads ground truth, so it cannot
    // false-skip regardless of which write paths exist (unlike a hook-only
    // timestamp scheme, which silently corrupts if any write bypasses the hook).
    // Inline (v,x) raw_bits snapshots: gateable blocks restrict to <=64-bit
    // reads (see build_event_measure_state), so two u64 compares replace a
    // full Value::eq — orders of magnitude cheaper at 555M compares/run.
    // Flat CSR layout (one allocation each) keeps the per-fire compare in
    // contiguous memory: edge_block_off[bi]..edge_block_off[bi+1] indexes
    // both reads_flat and snap_flat for block bi.
    edge_block_reads_flat: Vec<u32>,
    /// Parallel to `edge_block_reads_flat`: the (lo, width) BIT SLICE of the
    /// signal that the block actually reads. A full read of a narrow signal
    /// is (0, width). A constant bit/part select of a WIDE bus records just
    /// the selected slice, which is what lets a 1-bit synchronizer tap of a
    /// 1000+-bit bus stay gateable (whole-signal raw_bits would alias).
    edge_block_reads_meta: Vec<(u16, u16)>,
    edge_block_snap_flat: Vec<(u64, u64)>,
    edge_block_off: Vec<u32>,
    edge_block_snap_valid: Vec<bool>,
    // Blocks whose inputs change on nearly every fire temporarily use epochs
    // to bypass the redundant snapshot compare. Epochs may only force an
    // execution; every skip is still verified against the value snapshot.
    edge_block_change_streak: Vec<u8>,
    edge_block_epoch_probe_left: Vec<u8>,
    /// Self-heal for the flop-fire skip: consecutive SKIPPED clock edges per
    /// gateable block. After XEZIM_EVENT_EDGE_HEAL consecutive skips (default
    /// 16, 0 disables) the block is force-executed once and its snapshot
    /// rebuilt. For a correctly-idle flop the forced execution changes nothing
    /// (inputs unchanged ⇒ same outputs, NBA elided), so this is semantically
    /// invisible — but it BOUNDS the damage of any skip-bookkeeping wedge
    /// (e.g. a divider flop frozen for a dozen cycles while its clock toggles,
    /// as seen on a customer PLL model) to at most HEAL clock edges.
    edge_block_skip_streak: Vec<u16>,
    /// Heal threshold (consecutive skipped edges before a forced execution).
    event_heal: u16,
    /// Count of forced heal executions — nonzero means a skip streak hit the
    /// threshold; a LARGE count with wrong waveforms points at a skip bug.
    event_healed: u64,
    /// ARMED event-edge prefilter. Writes first test a compact signal bitmap;
    /// only data-input hits walk the sparse signal -> gateable-block fanout.
    /// An unarmed block can skip without reading its operand snapshots.
    armed_edge: bool,
    armed_edge_shadow: bool,
    /// XEZIM_EDGE_FIRE_TRACE=<substr>: log every edge-block fire whose
    /// instance scope or sensitivity-signal name contains <substr> — with
    /// the SCHEDULER WINDOW it fired from (active / cascade / clocking-mirror
    /// / rescan) and its gateable data-input values at fire time. Built to
    /// attribute one-cycle-early flop captures (a fire in any window other
    /// than `active` samples post-NBA state).
    edge_fire_trace: Option<String>,
    /// Lazily-built per-block match bitmap for `edge_fire_trace` — the
    /// string scan runs ONCE per block, not once per fire (the naive scan
    /// measurably slowed a customer run with thousands of fires per tick).
    edge_fire_match: Option<Vec<bool>>,
    /// XEZIM_EDGE_FIRE_WATCH=<name1,name2>: signals whose CURRENT values
    /// are appended to every traced fire line. Resolved lazily by substring
    /// match against the signal table (first match per pattern).
    edge_fire_watch: Vec<(String, usize)>,
    edge_fire_watch_init: bool,
    /// Label of the dispatch window the current check_edges belongs to.
    edge_dispatch_phase: &'static str,
    /// XEZIM_VALUE_TRACE=<substr>[,<substr>...] — print every committed
    /// change of any signal whose hierarchical name contains a pattern:
    /// time, name, old→new value, dispatch phase, and the WRITING process's
    /// origin (file:line). The tool for "where does the data stop flowing
    /// and who wrote it" on a design we cannot see. The write macro runs
    /// under disjoint-field borrow constraints, so hits are queued in a
    /// RefCell (same shape as warn_x_pending) and formatted at the drain.
    /// None = disabled, Some(ids) = resolved watch set.
    value_trace_ids: Option<std::collections::HashSet<usize>>,
    value_trace_pending: std::cell::RefCell<Vec<(usize, u64, Value, Value, usize, &'static str)>>,
    value_trace_emitted: u64,
    value_trace_limit: u64,
    /// One bit per signal id (u64 words): is it a data input of a gateable
    /// edge block? A bitset keeps the whole table cache-resident where one
    /// byte per signal (1.4 MB with array elements) did not.
    armed_input_bitmap: Vec<u64>,
    /// Start/end in armed_input_blocks and start in armed_input_masks
    /// (u32::MAX for a fanout that arms every reader).
    armed_input_ranges: Vec<(u32, u32, u32)>,
    armed_input_blocks: Vec<u32>,
    armed_input_masks: Vec<u64>,
    edge_block_armed: Vec<u8>,
    /// Edge blocks whose work was folded into a merged block: their
    /// sensitivities are cleared, so they never fire again. The skip census
    /// must ignore them wholesale — counting their ORIGINAL statement's
    /// writes made every merged target look multiply driven, and a
    /// multiply-driven output disqualifies the block that absorbed them.
    edge_block_retired: Vec<bool>,
    armed_fast_skips: u64,
    armed_shadow_checks: u64,
    // Phase-granular monotonic counter: bumped at each check_edges entry AND
    // each settle entry, so a flop's SAMPLE phase (check_edges) and the
    // comb-SETTLE phase that produces its next data have distinct timestamps
    // (loop_iters is per-tick — too coarse to separate them).
    event_phase: u64,
    event_would_skip: u64,
    event_gateable_total: u64,
    event_epoch_fast_exec: u64,
    event_snapshot_checks: u64,
    settle_calls: u64,
    /// Settle census: entry evaluations that actually changed a signal
    /// (i.e. propagated), and dependency edges walked doing so. The ratio
    /// against `entry_evals` says whether the lever is fewer triggers or
    /// cheaper dispatch.
    prof_settle_writes: u64,
    prof_settle_dep_edges: u64,
    /// Dynamic executions of the packed blocking-fill collapse. Reported with
    /// profiling so regression tests can prove the fast path was exercised.
    packed_blocking_fill_collapses: u64,
    // Profiling accumulators (nanoseconds)
    prof_edge_detect: u64,
    prof_edge_exec: u64,
    /// Phase-1 multicore observability: counts how many delta-cycles
    /// the parallel dispatcher took the partition-aware path vs the
    /// legacy chunk-by-range path. Both are bumped on every parallel
    /// dispatch; sum tells you total dispatcher cycles.
    prof_par_dispatch_partition: u64,
    prof_par_dispatch_legacy: u64,
    prof_edge_waiters: u64,
    prof_edge_cg: u64,
    prof_waiter_iters: u64,
    prof_edges_fired: u64,
    prof_insns_executed: u64,
    /// Auto-calibration of the parallel edge-dispatch path (see the gate in
    /// `check_edges_inner`). The old gate assumed threading always pays once a
    /// tick carries >=10k insns, but whether it pays at all depends on the
    /// machine (thread handoff + NBA merge cost vs core count) and on the
    /// block SHAPE (many tiny blocks amortize nothing). Rather than guess a
    /// constant, measure: run the first qualifying ticks each way and keep
    /// whichever is faster. 0 = still calibrating sequentially, 1 = calibrating
    /// in parallel, 2 = locked.
    par_cal_phase: u8,
    par_cal_ticks: u32,
    par_cal_seq_ns: u128,
    par_cal_seq_insns: u64,
    par_cal_par_ns: u128,
    par_cal_par_insns: u64,
    par_cal_use_parallel: bool,
    /// Ticks dispatched through the parallel edge path, and blocks so
    /// dispatched. Reported in `[PROF]` because those blocks' instructions are
    /// executed on worker threads and are NOT included in `insns=`.
    prof_par_ticks: u64,
    prof_par_blocks: u64,
    /// Zero-delay (delta) livelock detection: how many event-loop iterations
    /// have run at the CURRENT simulation time without it advancing. A design
    /// whose processes keep re-arming at the same timestamp (`forever #0;`, a
    /// zero-delay event ping-pong, a `wait` on an already-true condition inside
    /// a `forever`) can never advance, and xezim used to spin silently forever
    /// — the user just saw the sim "stuck at time 0".
    /// Re-entry depth of `exec_forever_sched`'s suspend-aware path. A `forever`
    /// whose leading statement only SOMETIMES blocks (`wait (cond)` with cond
    /// already true) would otherwise recurse once per iteration and overflow the
    /// Rust stack; past this depth the continuation goes back to the scheduler.
    forever_depth: u32,
    /// Continuation frames built by `exec_forever_sched`, keyed by the
    /// process, the loop body's source span, the index of the statement it
    /// parked on, and the site kind. The span identifies the LOOP: the
    /// `ForeverTail` inside each continuation carries a clone of the body,
    /// so an address key would change on every iteration (the first cut of
    /// this cache missed on every wake-up and grew without bound). The
    /// process id keeps two inlined instances of the same source loop —
    /// same span, different rewritten names — apart. Before this cache
    /// every wake-up of a `forever @(posedge clk)` / `always` process
    /// deep-cloned its statement list and its whole body into a fresh
    /// frame: 5 % of the c906 memcpy steady state was that cloning.
    /// Resolved sensitivity ids of a `forever` loop's wait, keyed by (pid,
    /// wait-statement span start): the loop re-parks on the same wait every
    /// iteration, and resolving it meant rebuilding signal-name strings and
    /// hashing them per wake-up (a clocked testbench monitor routed to the
    /// process path did this every cycle).
    forever_sens_cache: HashMap<(usize, u32), (Vec<SensitivityId>, bool)>,
    forever_cont_cache: HashMap<(usize, usize, usize, usize, u8), Arc<[Statement]>>,
    /// Pending-injection bitset for `settle_combinatorial_inner` (kept to avoid reallocation).
    settle_inject_bits: Vec<u64>,
    /// Summary of `settle_inject_bits` (one bit per word); sized from the
    /// entry count like the bitset itself. A fixed 32-word array here
    /// overflowed on c910 (218k entries).
    settle_inject_sum: Vec<u64>,
    /// A `return <virtual interface>` left `__vif_return__` for the next assignment.
    vif_return_pending: bool,
    /// Static lvalue widths of bare-leaf identifiers, per identifier span (see infer_lhs_width).
    lhs_leaf_width_cache: HashMap<(usize, usize), u32>,
    /// See `blocking_cont_frame`. Keyed by `(node_addr, kind)` and retains
    /// the owning `Arc<[Statement]>` alongside the prepared continuation frame
    /// so the AST node address stays valid and cannot be recycled, while all
    /// processes (`pid`s) executing the same shared AST node reuse one frame.
    #[allow(clippy::type_complexity)]
    blocking_cont_cache: HashMap<(usize, u8), (Arc<[Statement]>, Arc<[Statement]>)>,
    /// Computed once from the design and checkpoint command-line options.
    share_task_frames: bool,
    /// Structural continuation eligibility, computed once per declaration. Holding the
    /// declaration keeps its address valid; no statement body is cloned here.
    task_frame_eligibility: HashMap<usize, (std::rc::Rc<TaskDeclaration>, bool)>,
    /// See `task_body_frame`.
    #[allow(clippy::type_complexity)]
    task_frame_cache:
        HashMap<(usize, usize, usize), (std::rc::Rc<TaskDeclaration>, Option<Arc<[Statement]>>)>,
    /// See `class_task_body_frame`.
    #[allow(clippy::type_complexity)]
    class_task_frame_cache:
        HashMap<(usize, usize, usize), (Arc<crate::ast::decl::ClassMethod>, Arc<[Statement]>)>,
    /// §11.8.1: while narrowing relational constraint bounds for a target whose
    /// comparison context is UNSIGNED (the target, or the other operand, is
    /// unsigned), a bound literal must be read by its unsigned value — e.g. the
    /// unsized decimal `4294967290` (0xFFFFFFFA), which `to_i64` would otherwise
    /// sign-extend to -6, collapsing `u >= 4294967290` to `u >= 0`. Scoped to
    /// the `narrow_relational_bounds` / `affine_*` subtree.
    constraint_cmp_unsigned: bool,
    stall_iters: u64,
    stall_time: u64,
    /// Process activations at the current timestamp — the accurate signal for
    /// "who is spinning" (the event queue's own pid_counts is reset per drain).
    stall_pid_hits: HashMap<usize, u64>,
    /// Last sampled functional data/control state of each pid's guard window.
    stall_pid_state: HashMap<usize, u64>,
    /// Last outer-loop state sample; zero means no sample in this timestamp.
    stall_state0: u64,
    /// True when `XEZIM_STALL_LIMIT` was set explicitly: the guard is then a
    /// hard per-process activation cap (opt-in strict mode). When unset, the
    /// guard is progress-aware and only trips when a full window of
    /// activations recurred in the same functional data and control state.
    stall_limit_explicit: bool,
    /// Delta cycles allowed at one timestamp before the run is declared a
    /// zero-delay livelock. Generous by default (real designs settle in a
    /// handful); tune with XEZIM_STALL_LIMIT, 0 disables the check.
    stall_limit: u64,
    /// Set by the per-pid stall check inside `run_process_stmts` when a single
    /// process spins past `stall_limit` at one timestamp (e.g. an
    /// `always #(period) clk=~clk` whose real period glitched to 0). Instead of
    /// aborting there, it flags the outer loop to attempt a defer-and-advance
    /// to the next scheduled event (commercial-sim parity) — a #0 spinner whose
    /// driving value recovers once time moves resumes; only a livelock with
    /// nothing scheduled ahead is fatal. See `try_break_zero_delay_livelock`.
    zero_delay_defer_pending: bool,
    prof_fallback_insns: u64,
    /// Fine-grained phase timing calls `Instant::now` several times per
    /// simulation tick. Keep it opt-in so normal runs do not pay that cost.
    profile_timing: bool,
    /// XEZIM_COMB_PATHS=1: per-comb-entry settle-path counters
    /// [two_state, jit, interpreter], dumped ranked at end of sim.
    trace_comb_paths: bool,
    /// Per entry: [two-state execs, jit execs, interp execs, interp insns].
    comb_path_counts: Vec<[u64; 4]>,
    /// Last interp-path entry noted and the VM insn counter at that point
    /// (the delta at the next note is charged to it).
    comb_path_last: Option<(usize, u64)>,
    /// XEZIM_COMB_PATHS: per edge block [interp fires, interp VM insns].
    edge_path_counts: Vec<[u64; 2]>,
    /// XEZIM_COMB_PATHS: per comb entry [evals, evals repeated within one
    /// time step], plus the last evaluation time.
    comb_eval_counts: Vec<[u64; 2]>,
    comb_eval_last_time: Vec<u64>,
    prof_fallback_by_reason: HashMap<Arc<str>, (u64, u64)>,
    prof_settle_dc_ns: u64,
    prof_settle_ca_ns: u64,
    prof_settle_ab_ns: u64,
    prof_settle_dc_count: u64,
    /// Entry evaluations made in a settle pass after the first.
    prof_settle_repass_evals: u64,
    /// Per-CombItem-variant evaluation histogram (XEZIM_ENTRY_HIST=1).
    prof_entry_hist: [u64; 12],
    /// Per-entry eval counts (XEZIM_PROFILE_TIMING) for shape attribution.
    prof_entry_counts: Vec<u64>,
    /// `--profile` sampler thread (see `prof_sampler`); `prof_cur` is the
    /// slot the simulation thread publishes its current construct into.
    #[serde(skip)]
    prof_sampler: Option<crate::compiler::prof_sampler::ProfSampler>,
    prof_cur: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// XEZIM_CONE: entry is a member of a mergeable chain (static analysis).
    cone_chain_member: Vec<bool>,
    /// XEZIM_CONE: the chains themselves, so the end-of-run report can weigh
    /// the dispatches a merge would SAVE against the recomputation it would
    /// FORCE. A merged kernel fires when any member's inputs change and then
    /// recomputes every member, so its best-case work is max(evals)*len while
    /// the unmerged chain costs sum(evals). Equal-firing members make that
    /// ratio 1.0 (merging is pure win); skewed members make it >1.0 and the
    /// added compute has to be paid for out of the saved dispatch.
    cone_chains: Vec<Vec<u32>>,
    prof_settle_ca_count: u64,
    prof_settle_ab_count: u64,
    /// Persistent buffers for settle_combinatorial (avoid repeated allocation)
    settle_triggered: Vec<bool>,
    settle_dirty_ids: Vec<usize>,
    /// Pre-allocated buffer for always block write_signal snapshots during settle
    settle_prev_values: Vec<(usize, Value)>,
    /// Track which entries were triggered (for selective clearing)
    settle_triggered_list: Vec<usize>,
    loop_iters: u64,
    entry_evals: u64,
    settle_iters: u64,
    max_settle_iters: u64,
    /// Per-comb-entry trigger count (for --activity-mon).
    activity_counts: Vec<u64>,
    /// Per-signal change count (for --activity-mon).
    signal_toggle_counts: Vec<u64>,
    /// Whether activity monitoring is enabled.
    pub activity_mon: bool,
    /// XEZIM_TRACE_ALWAYS: per-fire trace of always blocks. None = off;
    /// Some("") = trace every block; Some(s) = only blocks whose label
    /// (kind, scope, written signal names) contains `s`.
    trace_always: Option<String>,
    /// Runtime plusargs passed from CLI/filelists (e.g. +FOO, +BAR=1).
    plusargs: Vec<String>,
    /// The seed the run chose with `+seed=<n>` / `+seed=random` / `-sv_seed`
    /// (None: the default stream). `XEZIM_INIT_REG=random` mixes it in, so
    /// register X-init values change with the seed.
    run_seed: Option<u64>,

    /// Owned copies of every CLI arg as `CString`s. Required so that
    /// `vpi_argv` (raw `*mut c_char` pointers into these buffers) stays
    /// valid for the lifetime of the simulator — `vpi_get_vlog_info` hands
    /// these pointers back to external C/DPI code which may store them past
    /// the duration of the VPI call.
    vpi_arg_cstrings: Vec<CString>,
    /// Raw C-string pointers parallel to `vpi_arg_cstrings`. The first
    /// entry is the binary name (xezim), the rest are CLI args + plusargs
    /// concatenated. Pointers stay stable across the simulator lifetime.
    #[serde(skip)]
    vpi_argv: Vec<*mut libc::c_char>,
    /// Cache of DPI scope handles keyed by hierarchical scope name.
    /// Each value is a `Box<DpiScope>` raw pointer handed out to C code
    /// via `svGetScopeFromName` and recovered in `svGetNameFromScope`.
    #[serde(skip)]
    dpi_scopes: HashMap<String, *mut libc::c_void>,
    prof_psettle_calls: u64,
    prof_psettle_deferred: u64,
    /// cbValueChange index (§38.36.1): watched signal id -> ids of the
    /// callbacks watching it. Consulted on every `write_sig!`, so it is a
    /// flat field (one emptiness check while nothing is watched); the rest of
    /// the callback state lives in `vpi_cb`.
    dpi_value_change_cbs: HashMap<usize, Vec<u64>>,
    /// Every collection/array member name declared by ANY class — the cheap
    /// pre-filter for chained-handle member resolution (see
    /// `instance_assoc_member`). Built once on first use.
    #[serde(skip)]
    chained_coll_member_names: std::cell::OnceCell<HashSet<String>>,
    /// §18.5.7 foreach solving: the `inside {…}` ranges the current body
    /// imposes on the element being solved (indices bound), so a relational
    /// repick (`arr[i] > arr[i-1]`) lands INSIDE the declared range instead of
    /// anywhere in the element's full domain.
    elem_inside_hint: Vec<(i64, i64)>,
    /// §18.5.8.1: the `<` / `<=` / `>` / `>=` bounds the current foreach
    /// body puts on the element being solved (`arr[i] OP bound`), for the
    /// modulo repair's draw.
    #[serde(skip)]
    elem_rel_bound: Vec<(crate::ast::expr::BinaryOp, Value)>,
    /// Set after several trials failed a strict foreach check: relational
    /// repicks take the BOUND value (minimal/maximal feasible) instead of a
    /// uniform draw, so a globally coupled monotone chain (`g[i][j][k] >
    /// g[i-1][j][k]` over a 4x4x4 grid in [1:2000]) is solved greedily in
    /// index order rather than exhausting the range by luck.
    rand_tight_mode: bool,
    /// VPI callback registry (§38.36, `vpi_cb.rs`). Untouched unless a VPI
    /// client registers callbacks.
    #[serde(skip)]
    vpi_cb: Box<vpi_cb::VpiCbState>,
    /// Assertion callback registry (IEEE 1800-2017 Clause 39,
    /// `vpi_assert.rs`). Untouched unless a VPI client registers assertion
    /// callbacks.
    #[serde(skip)]
    vpi_assert: Box<vpi_assert::VpiAssertState>,
    /// An assertion callback is registered: the gate every assertion hook
    /// tests first, so a run without them pays one load and branch.
    #[serde(skip)]
    vpi_assert_on: bool,
    /// Bit `r` set while a callback of reason `r` is registered (plus
    /// `vpi_cb::MASK_PENDING`): the gate every callback hook tests first, so
    /// a run without VPI callbacks pays one load and branch per hook site.
    vpi_cb_mask: u64,
    /// Time of the last slot whose start (cbNextSimTime, cbAtStartOfSimTime)
    /// has been processed; u64::MAX before the first.
    vpi_slot_started: u64,
    /// `$display` output written since the stdout sink was last synced, so a
    /// VPI callback's own output can be ordered after it cheaply.
    stdout_unsynced: bool,
    /// Self-pointer, installed once at the top of `simulate()`. The
    /// `write_sig!` macro needs a `*mut Simulator` to hand the VPI
    /// callback dispatcher, but it expands in contexts that already hold
    /// a mutable borrow of another field, so it cannot re-borrow `self`.
    /// Reading this field is disjoint from those borrows. Without it the
    /// macro used to install a NULL `ACTIVE_SIMULATOR`, so a callback that
    /// called back into VPI aborted the process.
    #[serde(skip)]
    vpi_self_ptr: *mut Simulator,
    /// Open file handles for $fopen/$fwrite/$fclose, keyed by an internal
    /// channel id derived from the MCD/FD value (see `resolve_channel_key`):
    /// MCD channels use their bit number (1..=30), FD channels use the raw FD
    /// value (0x8000_0000+). FD STDOUT/STDERR/STDIN and MCD bit 0 are never
    /// inserted — they are handled inline at write time.
    #[serde(skip)]
    file_handles: HashMap<i32, sv_file::SvFile>,
    /// `$save`/`$incsave`/`$restart` requests for the end of this slot.
    #[serde(skip)]
    checkpoint_ops: Vec<checkpoint::CheckpointOp>,
    /// A save was requested, or the design can call `$restart` (Annex D.9).
    /// Origins are not checkpointed: retain them even for an external save.
    #[serde(skip)]
    checkpoint_requested: bool,
    /// `--save-at` times still ahead, in ticks, earliest first.
    #[serde(skip)]
    checkpoint_save_at: Vec<(u64, String)>,
    /// The state came from a checkpoint: the event loop resumes it as is
    /// instead of starting time 0.
    #[serde(skip)]
    checkpoint_restored: bool,
    /// A `--restore` is pending: the time-0 settle in `compile` runs blocks
    /// whose output the checkpoint's run already printed.
    #[serde(skip)]
    checkpoint_muted: bool,
    /// `XEZIM_CKPT_AUDIT`: the fields seen changed since setup.
    #[serde(skip)]
    checkpoint_audit: Option<Box<checkpoint::audit::Audit>>,
    /// Some handle holds buffered `$fwrite` output (see `flush_file_writes`).
    file_writes_pending: bool,
    /// §21.3.5 (measured): `$feof` is a STICKY flag set when a read
    /// operation actually hits end-of-file — not a position probe. A read
    /// that consumes exactly up to the final newline leaves it clear.
    /// Cleared by `$fseek`/`$rewind`/`$ungetc` (C stdio semantics).
    file_eof: HashSet<i32>,
    /// Per-fd ungetc pushback buffer (LIFO).
    ungetc_buf: HashMap<i32, Vec<u8>>,
    /// §20.15 stochastic-analysis queues, keyed by q_id. Created by
    /// `$q_initialize`, mutated by `$q_add`/`$q_remove`, queried by
    /// `$q_full`/`$q_exam`.
    queues: HashMap<i64, StochasticQueue>,
    static_task_init: HashSet<String>,
    /// §6.21: the static locals of `initial`/`always`/`final` blocks whose
    /// declaration already ran, by (declarator span, storage name). Such a
    /// variable is created and initialized once; its block's later
    /// activations (and loop iterations) keep its value.
    static_block_decls: HashMap<(usize, usize), Vec<String>>,
    /// Generations of the frames a `for`/`foreach` loop pushes for its
    /// loop variables when its process has no call frame: a block local
    /// declared under one is still a process-block local (§6.21).
    loop_frame_gens: HashSet<u64>,
    current_static_task: Option<String>,
    /// Name of the enclosing subroutine when it is declared `automatic`
    /// (or a class method — automatic by default, §8.6). Mirrors
    /// `current_static_task`, but records the AUTOMATIC half of a
    /// subroutine's lifetime so per-invocation local-array isolation can
    /// be applied to module-scope automatic tasks/functions too, not just
    /// class methods. `None` at top level, and for class methods (whose
    /// locals already get isolated via the `in_method` path).
    current_auto_task: Option<String>,
    /// Best-effort hierarchical context for resolving ambiguous leaf identifiers.
    name_resolve_hint: RefCell<Option<String>>,
    /// The scope of the code CURRENTLY executing as a process/block
    /// activation: `Some(scope)` for an instance-scoped activation,
    /// `Some("")` for a top-scope one (its own names are the bare keys),
    /// `None` outside any activation (build/detection evals). Installed
    /// at activation entry (run_process_stmts for process
    /// activations, the edge-block AST-fallback branch for edge blocks)
    /// and restored on exit. Unlike `name_resolve_hint` it is NEVER
    /// advanced by the resolution ratchet, so a bare name the
    /// activation's own scope declares always wins over a mid-block
    /// hierarchical reference's ratcheted hint (§23.6: an
    /// unqualified name never resolves DOWNWARD into an instance).
    /// Outside activations it stays None so compile-time evals keep
    /// their deliberate hint-first order (clock-gen detection installs
    /// the block's own scope; a `rhs_parent_scoped` port connect
    /// installs the parent's).
    activation_scope: RefCell<Option<String>>,
    /// Depth of running methods of objects of a class declared inside a
    /// (non-top) module (§8, §23.9: each instance holds its own class, whose
    /// methods name that instance's items): such a method runs with the
    /// object's instance (`modclass_instance`) as its activation scope and
    /// resolve hint, and that instance's collections in its rename frame.
    modclass_depth: std::cell::Cell<u32>,
    /// The unpacked arrays, queues, dynamic and associative arrays,
    /// unpacked structs and sub-instances an instance declares (instance
    /// path -> bare names), for the rename frame a module-class method gets
    /// (see `modclass_depth`).
    modclass_colls: RefCell<HashMap<String, std::rc::Rc<Vec<String>>>>,
    /// Per class declared inside a (non-top) module with SEVERAL instances:
    /// those instances' paths (§8, §23.9: each holds its own class, with its
    /// own statics); None for every other class. See `modclass_static_inst`.
    modclass_insts: RefCell<HashMap<String, Option<std::rc::Rc<Vec<String>>>>>,
    /// Whether any such class exists (see `any_multi_modclass`).
    #[serde(skip)]
    multi_modclass: std::cell::OnceCell<bool>,
    /// Whether any class is declared inside a module other than the top
    /// (computed on first use): without one, `modclass_instance` is None.
    has_modclass: std::cell::Cell<Option<bool>>,
    /// Memoized `signal_name_to_id` keys under an assoc-array prefix.
    /// That map is populated only while the signal table is built and
    /// during `collapse_identity_port_nets` — both elaboration-time —
    /// so the set of its keys under a given prefix is FIXED for the
    /// run. `assoc_top_level_keys` used to re-scan the whole map on
    /// every call, `starts_with` per key; on the axi4 UVM AVIP that
    /// single scan was 5.1% of the run (memcmp attributed to it by a
    /// call-graph profile). The live half of the enumeration (elements
    /// in `self.signals`) is NOT cached — that map mutates at runtime.
    assoc_static_keys_cache: RefCell<HashMap<String, Arc<Vec<Arc<str>>>>>,
    /// `assoc_top_level_keys` results, valid while the array's element-key
    /// stamp (`SignalMap::elem_gen_for_prefix`) is unchanged.
    assoc_keys_cache: RefCell<HashMap<String, (u64, Arc<Vec<String>>)>>,
    /// §16.9.3 sampled-value function watches: per call site, a history of
    /// the operand sampled (preponed) at each matching clock edge. hist[0]
    /// is the sample at the most recent edge, hist[1] one edge earlier, …
    sampled_watches: Vec<SampledWatch>,
    /// §16.5.1 default sampled values by signal id: what each signal a
    /// concurrent assertion can name held before time 0 (its declaration
    /// initializer, else its type's default). §16.9.3 reads it for a clock
    /// tick before the first one. Filled by `capture_sampled_defaults`.
    sampled_defaults: HashMap<usize, Value>,
    /// The pre-process time-0 settle has run: a value read now is no longer
    /// a default sampled value.
    time0_settled: bool,
    /// §9.2.2.2 timing: `-> ev` fired from a comb block during the PRE-process
    /// time-0 settle (compile) happens before any `always @(ev)` process has
    /// registered its waiter — the trigger would be lost. Queue them and
    /// re-fire at the end of the time-0 active region.
    pending_t0_triggers: Vec<String>,
    /// §23.3.3: the time-0 update of each input port renamed to a whole
    /// actual signal (see `record_time0_port_edges`): the instance, the
    /// actual's id, the port's value before the update and the actual's
    /// value after the pre-process settle (raw v, x).
    time0_port_edges: Vec<(String, usize, (u64, u64), (u64, u64))>,
    /// §10.3.3: the ids a collapsed alias name (see `SensitivityId::alias_t0`)
    /// maps to whose value the pre-process settle left alone, with that
    /// value (raw v, x): the alias's time-0 update ends there.
    time0_alias_edges: Vec<(usize, (u64, u64))>,
    /// Edge blocks a time-0 alias or port update woke before any process
    /// ran. One that fires on its own at time 0 leaves the list (it runs
    /// once, as in the reference simulator); the rest run from
    /// `deliver_time0_port_edges`.
    edge_t0_pending: Vec<usize>,
    /// Edge blocks the next edge pass runs although no signal they watch
    /// changed: those a time-0 port update wakes.
    edge_forced_fire: Vec<usize>,
    /// True while the compile-time (pre-process) time-0 settle runs.
    in_pre_process_settle: bool,
    /// A combinational entry is being evaluated outside the settle
    /// (`release_time0_held`): its deferred assertion reports are the
    /// block's, as in the settle (`queue_deferred_assert`).
    comb_direct_eval: bool,
    /// Call-site (expression span start) → per activation scope (see
    /// `sampled_scope`) → index into `sampled_watches`. Every instance of a
    /// child module shares the call's span, so the scope tells them apart.
    sampled_watch_site: HashMap<usize, Vec<(Box<str>, usize)>>,
    /// §16.9.3: for a `$rose`/`$fell`/`$stable`/`$changed`/`$past` call with no
    /// explicit `@(...)` argument and no `default clocking`, the clocking event
    /// is inferred from the ENCLOSING procedural block's event control. Filled
    /// in by `register_sampled_watches` (call-site span → (edge code, clock
    /// signal id)) so the history accumulates from time 0. Keyed per
    /// activation scope like `sampled_watch_site`.
    sampled_site_clock: HashMap<usize, Vec<(Box<str>, (u8, usize))>>,
    /// Edge-controlled always blocks that run as PROCESSES (blocking body)
    /// and call a sampled-value function: (body, scope, edge code, clock id),
    /// consumed by `register_sampled_watches` like the edge blocks.
    proc_sampled_blocks: Vec<(Statement, String, u8, usize)>,
    /// The name segments the concurrent assertions in every always block
    /// spell, taken as the blocks are classified: one that runs as a
    /// process is in no list `capture_sampled_defaults` reads.
    #[serde(skip)]
    always_sva_names: HashSet<String>,
    /// The leaf names of the clocks the concurrent assertions in every
    /// always block spell, taken as the blocks are classified.
    #[serde(skip)]
    always_sva_clocks: HashSet<String>,
    /// §16.14.6: the signals that may clock a procedural concurrent
    /// assertion, sorted by id, each with its bit 0 at the start of the
    /// time step `sva_clock_slot_time`. A site registered later in that time
    /// step had its clock's event in it only if the clock moved since.
    #[serde(skip)]
    sva_clock_watch: Vec<(usize, LogicBit)>,
    #[serde(skip)]
    sva_clock_slot_time: Option<u64>,
    /// True while `register_sampled_watches` pre-registers the sites.
    sampled_registering: bool,
}

impl Drop for Simulator {
    /// Release heap allocations that aren't tracked by Rust's lifetime
    /// machinery: the `Box<DpiScope>` pointers `svGetScopeFromName` hands out.
    ///
    /// `dpi_scopes` owns `Box`-es that were leaked to the C side. Drop them
    /// here so the simulator's exit doesn't leak them per-test. (Callback
    /// handles belong to the client, which frees them with `vpi_remove_cb`
    /// or `vpi_free_object`; the registry itself is plain owned data.)
    fn drop(&mut self) {
        // Scope handles
        for (_name, ptr) in self.dpi_scopes.drain() {
            if !ptr.is_null() {
                unsafe {
                    drop(Box::from_raw(ptr as *mut DpiScope));
                }
            }
        }
    }
}

#[cfg(test)]
mod constraint_set_tests;

#[cfg(test)]
mod two_state_dynamic_tests;

#[cfg(test)]
mod named_event_plan_tests {
    use std::rc::Rc;

    #[test]
    fn cached_targets_preserve_spelling_order_and_shared_signal_identity() {
        let mut sim =
            crate::simulate("module tb; event pulse; endmodule", 0).expect("elaborate event");
        let signal_id = *sim.signal_name_to_id.get("pulse").expect("event signal");
        // Two names may address one event signal. Both writes and their
        // covergroup callbacks must survive candidate deduplication.
        for name in [".pulse", "tb.pulse"] {
            sim.signal_name_to_id.insert(name.into(), signal_id);
        }
        let event = sim.event_triggered_time.intern(".pulse");
        let plan = sim.named_event_plan(event);
        let names: Vec<_> = plan
            .targets
            .iter()
            .map(|target| target.name.as_ref())
            .collect();
        assert_eq!(names, [".pulse", "pulse", "tb.pulse"]);
        for target in &plan.targets {
            assert_eq!(target.signal_id, signal_id);
            assert_eq!(sim.event_triggered_time.get(&target.name), None);
        }
        assert!(Rc::ptr_eq(&plan, &sim.named_event_plan(event)));

        let bare = sim.event_triggered_time.intern("pulse");
        let bare_plan = sim.named_event_plan(bare);
        let bare_names: Vec<_> = bare_plan
            .targets
            .iter()
            .map(|target| target.name.as_ref())
            .collect();
        assert_eq!(bare_names, ["pulse", "tb.pulse"]);
        assert!(!Rc::ptr_eq(&plan, &bare_plan));
        let missing = sim.event_triggered_time.intern("unbacked_event");
        let empty = sim.named_event_plan(missing);
        assert!(empty.targets.is_empty());
        assert!(Rc::ptr_eq(&empty, &sim.named_event_plan(missing)));
        assert!(!sim.event_triggered_time.contains_key("unbacked_event"));
    }
}

#[cfg(test)]
mod event_name_tests;

#[cfg(test)]
mod process_cache_lifetime_tests;

#[cfg(test)]
mod finished_process_lifetime_tests;

#[cfg(test)]
mod class_heap_lifetime_tests;
#[cfg(test)]
mod clocks_only_scan_tests;

#[cfg(test)]
mod signal_map_tests;

mod heap_trace;

mod heap_gc;

mod builtins;
mod call_frames;
mod class_state;
mod comb_network;
mod coverage_state;
mod dpi_types;
mod method_tiering;
mod packed_mem;
mod process_state;
mod rand_state;
mod run_options;
mod scheduler_state;
mod signal_map;
mod sva_state;
mod vpi_entry;
mod word_ops;
mod write_tracking;
use builtins::*;
pub(crate) use call_frames::*;
use class_state::*;
pub use comb_network::*;
use coverage_state::*;
use dpi_types::*;
pub use method_tiering::*;
pub(crate) use packed_mem::*;
pub(crate) use process_state::*;
pub use rand_state::*;
pub use run_options::*;
pub use scheduler_state::*;
pub use signal_map::*;
use sva_state::*;
pub use vpi_entry::*;
pub(crate) use word_ops::*;
use write_tracking::*;

