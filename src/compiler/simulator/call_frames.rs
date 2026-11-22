//! Call frames and plans: task frames, method call plans, port and return plans, declaration
//! shadows, block scope saves and task cleanups.

use super::*;

/// Frame-local string classifications for task formals and scalar locals.
/// They remain authoritative while other processes change global metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct TaskFrameStrings {
    /// `local_gen_stack` generation of the task's frame.
    pub(super) fgen: u64,
    /// Its `string` formals.
    pub(super) strings: Vec<String>,
    /// Its non-string formals whose name `string_signals` holds for another
    /// scope.
    pub(super) others: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FormalMetadataSnapshot {
    pub(super) name: String,
    pub(super) parts: FormalMetaParts,
}

/// The name-keyed type metadata of one formal or local (see
/// `snapshot_formal_parts`).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct FormalMetaParts {
    pub(super) declared_type: Option<Rc<DataType>>,
    pub(super) packed_fields: Option<Vec<(String, u32, u32)>>,
    pub(super) packed_element_widths: Vec<(String, u32)>,
    pub(super) packed_dimensions: Option<Vec<(i64, i64)>>,
    pub(super) class_type: Option<String>,
    pub(super) typedef_type: Option<String>,
}

/// Per-frame virtual-interface formal and local aliases (`local_iface_aliases`).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct VifAliasFrame {
    pub(super) is_task_frame: bool,
    pub(super) bindings: HashMap<String, String>,
}

/// Declaration facts of one class method's formals and return value, built
/// once per method (see `Simulator::method_call_plan`). The facts marked
/// "type tables" read `typedefs` / `typedef_types` / `parameters` /
/// `enum_members` / `plain_class_types`, which a procedural `typedef` or an
/// inline-enum local can change at run time: they hold while
/// `Simulator::type_tables_gen` equals `type_gen`, and every use re-derives them
/// otherwise.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct MethodCallPlan {
    /// Keeps the method alive, and with it the address the plan is keyed by.
    pub(super) _method: Arc<crate::ast::decl::ClassMethod>,
    pub(super) type_gen: u64,
    pub(super) ports: Vec<PortPlan>,
    pub(super) ret: Option<RetPlan>,
    /// Every formal has `never_vif == true` (type tables): no formal can
    /// carry a virtual-interface alias.
    pub(super) all_never_vif: bool,
    /// The `formal_meta_name` positions: the names whose metadata a call
    /// saves and restores (declaration only).
    pub(super) meta_ks: Vec<usize>,
    /// `meta_key_bits` of every one of those names, OR-ed.
    pub(super) meta_bloom: u64,
    /// `meta_key_bits` of each of those names, by position.
    pub(super) meta_bits: Vec<u64>,
}

impl MethodCallPlan {
    /// The `k`-th saved name (see `meta_ks`).
    pub(super) fn meta_name(&self, k: usize) -> &str {
        use crate::ast::decl::ClassMethodKind;
        let (ports, ret) = match &self._method.kind {
            ClassMethodKind::Function(f) => (&f.ports[..], Some(f.name.name.name.as_str())),
            ClassMethodKind::Task(t) => (&t.ports[..], None),
            _ => (&[][..], None),
        };
        match ports.get(k) {
            Some(p) => p.name.name.as_str(),
            None => ret.unwrap_or_default(),
        }
    }
}

/// An active class-method activation's formal-metadata save. The snapshot
/// of each formal and return-variable name (see `snapshot_formal_parts`) is
/// taken when that name's metadata is first written — up to then the tables
/// hold the entry state for it, which is what an eager snapshot would have
/// saved — and a name never written has nothing to restore. Every writer of
/// the six tables calls `meta_note` first.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct MetaFrame {
    pub(super) plan: std::rc::Rc<MethodCallPlan>,
    /// Positions in `plan.meta_ks` already saved (all of them once `full`).
    pub(super) saved: u64,
    pub(super) full: bool,
    pub(super) parts: Vec<(usize, FormalMetaParts)>,
    /// `meta_bloom` of this save and of every save below it.
    pub(super) cum_bloom: u64,
    /// `meta_bits` of the names not saved yet, OR-ed.
    pub(super) unsaved_bloom: u64,
}

/// See `MethodCallPlan`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PortPlan {
    /// `port_is_plain_scalar` (declaration only).
    pub(super) plain_scalar: bool,
    /// `port_class_ref` (type tables).
    pub(super) class_ref: Option<bool>,
    /// Width of a built-in integer or literal-range vector formal (type
    /// tables); `None` for any other type.
    pub(super) fixed_width: Option<u32>,
    /// `is_type_signed` of the declared type (declaration only).
    pub(super) signed: bool,
    /// A direct scalar two-state declaration; independent of specialization.
    pub(super) two_state: bool,
    /// `is_virtual_iface_type` is false whatever the specialization: neither
    /// an interface nor a type reference, or a plain class (type tables).
    pub(super) never_vif: bool,
}

/// §13.4.1: the conversion a function's value takes to its declared return
/// type (see `Simulator::ret_convert`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RetConv {
    /// Not a numeric scalar: the value passes through untouched.
    Keep,
    /// Binary64 real and realtime.
    Real,
    ShortReal,
    /// An integral type; `two_state` for bit, byte, int, ... (§6.11.1).
    Int { two_state: bool },
}

/// See `MethodCallPlan` (functions only).
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RetPlan {
    /// `method_return_is_string`, when no specialization can change it (type
    /// tables).
    pub(super) is_string: Option<bool>,
    /// The declared type is `string` (declaration only).
    pub(super) decl_string: bool,
    /// `dyn_ret_type` of the call path (declaration only).
    pub(super) dyn_ret: Option<DataType>,
    /// `unpacked_struct_of` (type tables).
    pub(super) unpacked: Option<crate::ast::types::StructUnionType>,
    /// `resolve_type_width` of the declared type (type tables).
    pub(super) width: u32,
    pub(super) signed: bool,
    pub(super) real: bool,
    pub(super) two_state: bool,
    /// A declared class-type return (the class tables are fixed).
    pub(super) class: Option<String>,
    /// `fn_returns_collection` (type tables).
    pub(super) collection: bool,
}

/// The name-keyed type metadata a subroutine-local declaration displaced
/// (see `decl_shadow_log`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DeclShadow {
    pub(super) width: Option<u32>,
    pub(super) signed: bool,
    pub(super) real: bool,
    pub(super) string: bool,
    pub(super) meta: FormalMetadataSnapshot,
}

/// `Simulator::decl_param_scope`'s state (§23.10).
#[derive(Default)]
pub(super) struct DeclParamScopeCache {
    /// Every scope that has a parameter at or below it: each dot-prefix of
    /// a key of the design-wide table (`u` and `u.g` for `u.g.L`). Built on
    /// first use, in one pass over the table.
    pub(super) owners: Option<HashSet<String>>,
    /// The prefix asked for last, and its scope.
    pub(super) last: Option<(String, Option<DeclScope>)>,
}

impl DeclParamScopeCache {
    /// Note a key added to the design-wide table after `owners` was built.
    pub(super) fn note_param(&mut self, key: &str) {
        if let Some(owners) = &mut self.owners {
            for (i, _) in key.match_indices('.') {
                if !owners.contains(&key[..i]) {
                    owners.insert(key[..i].to_string());
                }
            }
        }
        self.last = None;
    }
}

/// §23.10: the instance scope a declaration stored under `<prefix>.<name>`
/// is sized in (see `DeclScopeParams`). Cheap to clone.
#[derive(Clone)]
pub(super) struct DeclScope {
    /// The declaration's instance path (`u.g` for `u.g.par`).
    pub(super) prefix: std::rc::Rc<str>,
    /// The lengths of the enclosing scopes of `prefix` (`u`, `u.g`) that
    /// have parameters, innermost first.
    pub(super) owners: std::rc::Rc<[usize]>,
    /// The bindings as one table, for the rare whole-table reads
    /// (`ParamLookup::rows`), built on the first.
    pub(super) flat: std::rc::Rc<std::cell::OnceCell<HashMap<String, Value>>>,
}

/// One selected region of a declared packed value (§7.4.1 / §11.5.1).
pub(super) struct DeclaredPackedAccess {
    pub(super) storage: String,
    /// §23.10: the instance scope the declaration is sized in when the
    /// storage lives in an instantiated module whose scopes have parameters;
    /// `None` for the top scope (the design-wide table).
    pub(super) params: Option<DeclScope>,
    pub(super) is_signed: bool,
    pub(super) static_target: Option<(String, Option<(String, String)>, String)>,
    pub(super) offset: u32,
    pub(super) width: u32,
    pub(super) dimensions: Vec<(i64, i64)>,
    pub(super) element_type: DataType,
    pub(super) unpacked: usize,
}

/// A known static root either supports packed access or retains its own path.
pub(super) enum DeclaredStaticAccess {
    Packed(DeclaredPackedAccess),
    Other,
}

/// One subroutine activation's displaced declarations, restored when it
/// returns. `frame_gen` is the activation's local-frame generation; `epoch` the
/// `nested_run_epoch` at entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct DeclShadowFrame {
    pub(super) frame_gen: u64,
    pub(super) epoch: u64,
    pub(super) saves: Vec<DeclShadow>,
    /// §6.21: fresh names a `string` declaration of this activation added
    /// to `string_signals`, taken back out when it ends.
    pub(super) fresh_strings: Vec<String>,
    /// Every FRESH name this activation declared (no metadata to put back):
    /// the geometry such a declaration registers under the bare name is
    /// removed when it ends (see `drop_fresh_local_geometry`, #916).
    #[serde(default)]
    pub(super) fresh_names: Vec<String>,
}

/// §6.21: the scope of a `begin…end` block that declares variables, opened
/// when the block is entered with a call frame on the stack. Block-locals
/// live in the enclosing frame under their bare names, so on exit the names
/// the block declared are removed again and any enclosing variable they
/// shadowed is put back. Only shadowed names cost a copy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct BlockScopeSave {
    /// Distinguishes repeated activations of flattened block declarations.
    pub(super) scope_gen: u64,
    pub(super) declared_names: Vec<String>,
    /// `frame_key` of the flattened block's frame (blocking path); 0 for a
    /// synchronous block.
    pub(super) frame_key: usize,
    /// Generation of the frame the block declares into: exit leaves a frame
    /// of another activation alone.
    pub(super) frame_gen: u64,
    /// Enclosing frame entries (bare names and unpacked-struct leaves) the
    /// block's declarations shadow.
    pub(super) shadowed: Vec<(String, Value)>,
    /// §6.21: declared layouts and scalar types displaced by this block.
    pub(super) declarations: Vec<DeclShadow>,
    /// The enclosing frame's string-kind overlay, not the global name table.
    pub(super) string_kinds: Vec<(String, Option<bool>)>,
    /// Virtual-interface aliases hidden by this block.
    pub(super) vif_aliases: Vec<(String, Option<String>)>,
    /// A suspend-aware for/foreach loop closes its variable scope at the same sentinel.
    pub(super) loop_vars: Vec<LoopVarSnapshot>,
    pub(super) owns_loop_frame: bool,
    /// Enclosing collection renames (`local_dyn`) the block's collection
    /// declarations shadow, moved aside so the inner declaration cannot
    /// clean the outer storage up.
    pub(super) dyn_shadowed: Vec<(String, String)>,
    /// `(depth, len)` of `static_local_syncs` at entry when the block
    /// declares a `static` variable.
    pub(super) static_mark: Option<(usize, usize)>,
    /// The subroutine's `static` locals that a non-static block variable of
    /// the same name hides, as `static_local_syncs` depth and the
    /// `(name, key)` entries taken out of that frame until the block ends.
    // Each entry keeps the position it was taken from.
    pub(super) static_hidden: Option<(usize, Vec<(usize, (String, String))>)>,
    /// `next_pid` at entry: a block that forked keeps its collection storage
    /// (a child may still use it).
    pub(super) pid_mark: usize,
}

/// Deferred teardown for an inlined blocking task/method call: the work
/// `exec_task_call` would normally run synchronously after the body, replayed
/// at the body's `ScopePop` sentinel so the process can suspend in between.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct TaskCleanup {
    /// Instance scope an inlined HIERARCHICAL/interface task body resolves
    /// bare names under (`bus.snoop()` → "bus"). Installed at bind and on
    /// every process RESUME while this frame is on top (a suspend inside the
    /// body would otherwise lose it), restored to `prev_hint` at unwind.
    pub(super) frame_scope_hint: Option<String>,
    pub(super) prev_hint: Option<String>,
    /// Caller activation and hierarchy scope restored before free-task copy-out.
    pub(super) caller_body_scope: Option<(Option<String>, String)>,
    /// §21.2.1.7 `%m`: the m_scope_stack to restore on task return (a task
    /// call RESETS the lexical scope to just the task, since tasks are
    /// declared at module/package level — the CALLER's scope is not part of
    /// the callee's `%m`).
    pub(super) saved_m_scope: Vec<String>,
    /// Name of the inlined task, for §9.6.2 `disable <task>` bookkeeping
    /// (`active_task_pids`). None for frames without a stable name.
    pub(super) task_name: Option<String>,
    pub(super) output_bindings: Vec<(String, crate::ast::expr::Expression)>,
    /// §13.5.3: omitted outputs copy back in the declaration's scope.
    pub(super) default_output_bindings: Vec<(String, crate::ast::expr::Expression)>,
    pub(super) assoc_params: Vec<(String, String, bool)>,
    pub(super) array_params: Vec<String>,
    /// `(formal, actual, lo, hi)` for an `output`/`inout`/`ref` ARRAY formal:
    /// its elements are copied back to the caller's array on return.
    pub(super) array_writebacks: Vec<(String, String, i64, i64)>,
    /// `(formal, actual)` for an `output`/`inout`/`ref` QUEUE / dynamic-array
    /// formal: the callee's collection is copied back to the caller's on
    /// return. Functions always did this; tasks bound no collection formal at
    /// all, so a `task push(ref int q[$])` pushed into storage named after the
    /// FORMAL and the caller's queue never changed.
    pub(super) queue_writebacks: Vec<(String, String)>,
    /// This frame pushed an entry on `task_string_formals` (popped on unwind).
    pub(super) pushed_string_formals: bool,
    pub(super) saved_break: bool,
    pub(super) saved_continue: bool,
    pub(super) saved_return: bool,
    pub(super) prev_static: Option<String>,
    /// The caller's `current_auto_task` to restore on unwind (paired with
    /// `prev_static`).
    pub(super) prev_auto: Option<String>,
    /// Set when a blocking-method inline (run_process_stmts) pushed a receiver
    /// `this`/class-context for an `obj.method()` call; pop them on unwind.
    /// Default false (free-task/same-`this` inlines don't touch those stacks).
    pub(super) pushed_method_this: bool,
    pub(super) saved_spec: Option<(String, String)>,
    pub(super) formal_metadata: Vec<FormalMetadataSnapshot>,
    /// The name-keyed select metadata this activation registered for its
    /// packed struct and packed array formals, installed again when the
    /// process resumes (see `Simulator::reinstall_struct_formal_meta`).
    #[serde(default)]
    pub(super) struct_formal_meta: Vec<(String, FormalMetaParts)>,
    /// The method task of a class declared inside a module runs in its
    /// object's instance (see `Simulator::modclass_depth`): the process's
    /// activation scope while this frame is on top.
    pub(super) modclass: Option<String>,
}

/// A `fn_ret_frames` entry: the declaration whose return type a pattern in
/// `return` takes.
#[derive(Clone)]
pub(crate) enum ReturnPatternType {
    /// A return type no pattern can target.
    NotPacked,
    Func(std::rc::Rc<DataType>),
    Method(Arc<crate::ast::decl::ClassMethod>),
}

/// Caller-frame state for a queue/dynamic-array LOCAL that a nested call may
/// clobber. Bare-name locals share the global `signals` and `module.*`
/// registration tables, so a nested method declaring a local of the same name
/// (even a non-queue, e.g. its own `int p`) overwrites the caller's
/// bookkeeping. We snapshot the caller's `name.size` / `name[i]` signals AND
/// its `module.arrays`/`dynamic_arrays`/`descending_arrays`/`widths`/
/// queue-limit registration, restoring both on return. Without the
/// registration restore, the caller's queue stays unregistered after the
/// nested call returns, so its later `delete()` and element reads become
/// silent no-ops (IEEE 1800-2023 §6.21 automatic-variable per-invocation storage).
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct QueueLocalSave {
    pub(super) signals: Vec<(String, Value)>,
    pub(super) arrays_entry: Option<(i64, i64, u32)>,
    pub(super) was_dynamic: bool,
    pub(super) was_descending: bool,
    pub(super) queue_max: Option<u32>,
    pub(super) width: Option<u32>,
    /// The caller's `module.associative_arrays` entry (`true` = string
    /// keys): a same-named callee local drops it at its declaration, and an
    /// int-keyed array re-registered as string-keyed reads its keys as text.
    pub(super) assoc_entry: Option<bool>,
}
