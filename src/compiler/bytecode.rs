//! Bytecode VM for high-performance simulation execution.
//! Compiles AST expressions and statements into a flat instruction array
//! that can be executed without pointer-chasing through Box<Expression> trees.

mod function_purity;
mod two_state;
mod instructions;

mod arrays;
mod case;
mod class_methods;
mod collections;
mod const_eval;
mod expressions;
mod fallbacks;
mod inlining;
mod locals;
mod lookup;
mod loops;
mod packed_members;
mod patterns;
mod peephole;
mod reals;
mod selects;
mod statements;
mod strings;
mod targets;
mod types;
mod widths;
pub use instructions::*;

pub use two_state::*;
#[cfg(test)]
use two_state::fuse_ts_pairs;
#[cfg(test)]
mod tests;

use super::simulator::NameMap;
use super::value::Value;
use super::{elaborate, fb_scan, struct_sra};
use crate::ast::decl::{FunctionDeclaration, TaskDeclaration};
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::types::PortDirection;
use std::sync::Arc;
use xezim_core::hasher::{HashMap, HashSet};

pub trait SignalWidthLookup {
    fn declared_width(&self, name: &str) -> Option<u32>;
}
impl SignalWidthLookup for HashMap<String, u32> {
    fn declared_width(&self, name: &str) -> Option<u32> {
        self.get(name).copied()
    }
}
impl SignalWidthLookup for super::simulator::SignalWidths {
    fn declared_width(&self, name: &str) -> Option<u32> {
        self.get(name).copied()
    }
}

/// §7.2: unpacked-struct variables by container signal id → (member path,
/// leaf signal name) in declaration order. An empty list: a struct whose
/// leaves are not all plain signals (array or union members).
pub type UnpackedStructLeaves = HashMap<usize, Vec<(String, String)>>;

/// §13.4.2/§23.10: function-table suffixes shared by compilers using
/// the same declarations, preserving admission of external state reads.
pub(crate) struct FunctionScopeIndex<'a> {
    functions: &'a HashMap<String, FunctionDeclaration>,
    qualified_leaves: HashSet<&'a str>,
}

impl<'a> FunctionScopeIndex<'a> {
    pub(crate) fn new(functions: &'a HashMap<String, FunctionDeclaration>) -> Self {
        let qualified_leaves = functions
            .keys()
            .filter_map(|name| name.rsplit_once('.').map(|(_, leaf)| leaf))
            .collect();
        Self {
            functions,
            qualified_leaves,
        }
    }

    pub(crate) fn qualified_leaf_count(&self) -> usize {
        self.qualified_leaves.len()
    }
}

const MAX_INLINE_DEPTH: usize = 8;

/// Sentinel for a method receiver whose declared class could not be resolved
/// (a handle local/formal with no carried declared type). `emit_arg_slot_writeback`
/// treats UNTYPED receivers with the conservative name-global writeback rule.
const UNTYPED: &str = "\u{0}";

/// Immutable admission facts shared by all method compiles in one design.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct MethodAdmission {
    /// class-perf writeback: non-input (output/inout/ref) formal positions,
    /// keyed by the class that DECLARES the method. Keying by class (not by
    /// bare method name) stops names that un-related classes share (one
    /// class's `get(output ...)` vs another's input-only `get`) from
    /// spuriously declining the other call's compile.
    pub writeback_positions: HashMap<(String, String), HashSet<usize>>,
    /// Formal positions with unpacked dimensions (directly or through a
    /// typedef), keyed like `writeback_positions` by the class that declares
    /// the method; free functions and tasks under the class `""` and their
    /// unqualified name.
    pub coll_formal_positions: HashMap<(String, String), HashSet<usize>>,
    /// Direct supervclass (extends target, spec args stripped) per declared
    /// class. `emit_arg_slot_writeback` walks this chain to gather the
    /// non-input formal positions inherited by a receiver's class.
    pub class_parents: HashMap<String, String>,
    pub instance_fields: HashMap<String, HashSet<String>>,
    /// §11.6.1: declared width of each fixed-width integral instance field,
    /// the context width of a compiled `recv.field = rhs`. Strings, reals,
    /// handles and parameter-sized fields are absent (stored self-determined).
    pub instance_field_widths: HashMap<String, HashMap<String, u32>>,
    /// The `instance_field_widths` fields declared signed (§11.8.1: a
    /// member operand's signedness types the expression it is in).
    pub instance_field_signed: HashMap<String, HashSet<String>>,
    /// The `instance_fields` that are strings (§6.16), so a dotted read
    /// through a handle of that class is a string operand.
    pub instance_string_fields: HashMap<String, HashSet<String>>,
    /// The `instance_fields` that are STRING QUEUE / dynamic / associative
    /// arrays (§6.16 + §7.10/§7.11), keyed by declared class: a dotted
    /// element read `obj.m_q[i]` through a handle of that class is a
    /// string operand (`string_cmp`/`StrOp` typing, not the integral
    /// fallback).
    pub instance_string_colls: HashMap<String, HashSet<String>>,
    /// §8.20: declared class -> method -> the class whose body a call
    /// through a handle of that declared type binds to, for every
    /// NON-virtual method that some subclass redeclares (the only case where
    /// the object's dynamic class would pick a different body).
    pub nonvirtual_targets: HashMap<String, HashMap<String, String>>,
    /// §8.23: class -> the STATIC methods it declares. A static method needs
    /// no object (§8.7/§8.25), so a call through a NULL handle of a known
    /// declared class is legal and must dispatch on that class; `CallMethod`
    /// dispatches on the live object's class, which does not exist there.
    pub static_methods: HashMap<String, HashSet<String>>,
    /// Instance collection members per declared class, for element access
    /// through a handle of that class (`p.contents[i]`). Only members whose
    /// elements load as one plain value join: string and multi-dimensional
    /// packed elements need facts the compiler keeps for `this` alone.
    pub instance_colls: HashMap<String, ClassColls>,
    /// Fixed scalar integral return (width, signed) of class methods, keyed by
    /// `(class, method_name)` including inherited methods.
    pub method_return_types: HashMap<(String, String), (u32, bool)>,
    /// Free-function facts the method compiles keep asking for; the function
    /// table they come from is the same for every method compile.
    pub fn_facts: FnFacts,
}

/// See `MethodAdmission::instance_colls`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ClassColls {
    /// §7.4.5: selecting these members once still yields an unpacked row.
    pub row_members: HashSet<String>,
    pub members: HashSet<String>,
    /// Queues and dynamic arrays: a `foreach` index is a plain integer.
    pub dense: HashSet<String>,
    /// Collections of integral elements, by the elements' declared sign.
    pub signed_elems: HashSet<String>,
    pub unsigned_elems: HashSet<String>,
    /// Declared element width of the integral collection members.
    pub elem_widths: HashMap<String, u32>,
}

/// Memoized free-function facts (see `MethodAdmission::fn_facts`).
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct FnFacts {
    /// (declaration name and source span, instance prefix, external reads
    /// allowed) -> `fn_is_pure_in_ext`. Only verdicts that do not depend on
    /// the recursion-depth cutoff are kept.
    #[allow(clippy::type_complexity)]
    pure: std::cell::RefCell<HashMap<(String, usize, usize, Option<String>, bool), bool>>,
    /// Callee name -> no instance-qualified twin exists (`compile_pure_call`).
    ext_reads_ok: std::cell::RefCell<HashMap<String, bool>>,
}

/// A register in the bytecode VM. Registers hold Values. The compact u16
/// encoding keeps each instruction at 24 bytes; the allocator uses a wider
/// counter and falls back before an ID would overflow this representation.
pub type RegId = u16;

/// Unpacked two-dimensional bounds and the width of one element.
type Array2dShape = ((i64, i64), (i64, i64), u32);

/// Packed dimensions of an unpacked array's element, outermost first, and
/// whether their range labels are the declared ones.
type ElemPackedDims = (Vec<(i64, i64)>, bool);

/// A signal-table index inside an instruction. `u32`, not `usize`: the
/// largest design measured here has 35.1 M signals and `u32` covers 4.29 B,
/// so the extra four bytes per field were pure footprint. Fourteen `Insn`
/// variants carry one, and they are what pushed the enum to 24 bytes — an
/// awkward size that costs a three-instruction `lea/add/lea` to address and
/// packs only 2.67 instructions per 64-byte cache line.
type SigId = u32;

/// Narrow a `usize` signal-table index to the in-instruction [`SigId`].
///
/// Every id ultimately comes from a `Vec` index, so on any design that fits
/// in memory this cannot overflow — but a silent wrap would be catastrophic
/// and invisible: the instruction would read and write a completely
/// unrelated signal for the rest of the run, with no diagnostic. Checked
/// unconditionally (not `debug_assert!`) because this runs once per emitted
/// instruction at compile time, never in the VM dispatch loop.
#[inline]
pub(crate) fn as_sig_id(id: usize) -> SigId {
    assert!(
        id <= SigId::MAX as usize,
        "signal id {id} does not fit in a {}-bit Insn signal field \
         (limit {}); Insn::SigId must widen before such a design can run",
        SigId::BITS,
        SigId::MAX
    );
    id as SigId
}

/// Number of `LoadSignal ; LoadArrayElem ; NbaAssign` triples collapsed into
/// `Insn::NbaAssignArrayRead` across every block compiled in this process.
/// Reported once by the simulator's `[PROF]` summary so the static fusion
/// count can be compared against the dynamic opcode census.
static FUSED_ARRAY_READ_NBA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Static count of array-read→flop fusions performed. See
/// [`Insn::NbaAssignArrayRead`].
pub fn array_read_nba_fusions() -> u64 {
    FUSED_ARRAY_READ_NBA.load(std::sync::atomic::Ordering::Relaxed)
}

/// Number of element-wise packed identity NBA sites lowered to whole-vector
/// NBAs across every block compiled in this process.
static PACKED_LOOP_NBA_COPIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn packed_loop_nba_copies() -> u64 {
    PACKED_LOOP_NBA_COPIES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Per-kind count of `LoadConst ; <binop>` pairs collapsed into
/// `Insn::BinOpConst`, indexed by `BinOpConstKind as usize`. See
/// [`BytecodeCompiler::fuse_binop_const`].
static FUSED_BINOP_CONST: [std::sync::atomic::AtomicU64; BinOpConstKind::COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; BinOpConstKind::COUNT];

/// Static count of `Move ; <assign>` pairs where the Move was forwarded into
/// the assign's value operand (see `forward_move_into_assign`).
static FUSED_MOVE_FWD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FUSED_COPY_FWD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Store folds kept in the persisted bytecode (`XEZIM_FOLD_STORES`, default
/// 1: dynamic-bound range stores only; the array element folds live in the
/// two-state lowering's copy).
const FOLD_STORES_PERSISTED: u32 = 1;
const FOLD_STORES_LOWERING: u32 = 6;

/// Static count of `ClearSigned` insns deleted because the register provably
/// held an unsigned value already (see `elide_provably_unsigned_scrubs`).
static ELIDED_SCRUBS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Static count of `AddC ; AddC` pairs merged into one `BinOpConstAdd2`
/// dispatch (see `fuse_addc2`).
static FUSED_ADDC2: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn addc2_count() -> u64 {
    FUSED_ADDC2.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn elided_scrub_count() -> u64 {
    ELIDED_SCRUBS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn census_pair_fusions() -> u64 {
    FUSED_MOVE_FWD.load(std::sync::atomic::Ordering::Relaxed)
}

/// Per-statement interpreter fallback inside compiled method bodies
/// (`XEZIM_METHOD_STMT_FALLBACK`, default on).
pub(crate) fn method_stmt_fallback_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("XEZIM_METHOD_STMT_FALLBACK").map_or(true, |v| v != "0"))
}

/// Copies forwarded into their readers by `forward_copies` (static sites).
pub fn copy_forward_count() -> u64 {
    FUSED_COPY_FWD.load(std::sync::atomic::Ordering::Relaxed)
}

/// Static count of constant-operand ALU fusions performed, per
/// [`BinOpConstKind`] (same index order as the enum).
pub fn binop_const_fusions() -> [u64; BinOpConstKind::COUNT] {
    std::array::from_fn(|i| FUSED_BINOP_CONST[i].load(std::sync::atomic::Ordering::Relaxed))
}

/// A compiled bytecode program for one always block or continuous assign.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]

pub struct CompiledBlock {
    pub instructions: Vec<Insn>,
    pub num_regs: u32,
    /// True when any instruction is a `StmtFallback` (AST-interpreted). Those
    /// resolve bare names through `resolve_hier_name`, which needs the owning
    /// entry's scope hint installed — the pure-bytecode insns pre-resolve their
    /// signal ids and don't. Precomputed here so the settle hot loop pays one
    /// bool test instead of scanning the insn stream.
    pub has_fallback: bool,
    /// True when some signal is the target of MORE THAN ONE nonblocking write
    /// in this block — the only situation in which §10.4.2 last-write-wins can
    /// be observed within a single block, and so the only one where a queued
    /// NBA entry has to be located and overwritten instead of the value simply
    /// being compared against the signal table.
    ///
    /// Precomputed because the isolated executors have no O(1) index into
    /// their per-block queue and would otherwise pay a linear scan on EVERY
    /// nonblocking write; measured at ~6.5% on an NBA-heavy design. The
    /// overwhelming majority of blocks write each target once and take the
    /// plain push path.
    pub nba_dup_targets: bool,
    /// class-perf Step 9d: how many distinct foreach key slots this block
    /// addresses (compile-time indices into the runtime arena). Zero for
    /// every block without a compiled member-collection foreach.
    pub foreach_slots: u32,
    /// class-perf §6.21: the METHOD body's `static` locals, as
    /// `(name, reg, default)`; empty for module blocks. The method
    /// dispatch shell seeds each register from `static_local_vars`
    /// before the block runs and copies the final register back into
    /// the locals frame afterwards so the exit sync persists it. The
    /// block itself never seeds the register (see
    /// `method_static_locals`).
    /// Cold for module blocks: boxed so the comb settle hot loop's
    /// `CombItem::CompiledContAssign { compiled }` variant keeps its
    /// cache-compact layout (see `comb_entry_layout_stays_cache_compact`).
    pub static_locals: Box<[(String, RegId, Value)]>,
}

/// class-perf Step 7: one resolved method-dispatch target, shared by `Rc`
/// across all calls reaching it. Produced by `Interpreter::method_dispatch`

/// class-perf Step 7b: a PREBOUND compiled-method gate. Everything between
/// the skip cache and the block cache in `try_run_compiled_method` — return-
/// kind resolution, per-formal type walks, seed widths — is a pure function
/// of the class declaration plus module scope (instance-independent by the
/// decision-cache rule), so it is computed ONCE per (class, method) and
/// reused on every call. The one genuinely per-instance input, the result
/// width of a param-bounded packed return, is flagged (`param_able_result`)
/// and re-resolved against the instance's param scope per call; formals'
/// seed widths are static (module-scope resolution) so the compiled block
/// for a width signature is shared across instances.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PreboundCompiledMethod {
    /// (formal name, seed width); width 0 = keep the caller's width.
    pub formals: Vec<(String, u32)>,
    /// Seed widths only — the block-cache key component; `Rc` so a call
    /// clones a refcount instead of rebuilding the `Vec`.
    pub formal_widths: std::rc::Rc<Vec<u32>>,
    /// Class-typed formal names (heap-member access inside the body).
    pub class_formals: std::rc::Rc<HashSet<String>>,
    /// Declared return type (only read when `param_able_result`).
    pub return_type: std::rc::Rc<crate::ast::types::DataType>,
    /// Implicit result-variable name — a compile-time input only (the block
    /// bakes it in); never touched on plan hits.
    pub fn_ret_name: String,
    /// Return is a class TypeReference: pass handles through untouched.
    pub is_class_result: bool,
    /// Module-scope result width (0 = pass-through / keep source).
    pub static_result_width: u32,
    pub result_signed: bool,
    /// Return is a param-bounded packed vector: resolve the effective width
    /// against the instance's param scope on every call.
    pub param_able_result: bool,
    /// Return is a `string` (§6.16): the result Value is a byte-vector
    /// passed through untouched, like a class handle.
    pub is_string_result: bool,
    /// String-typed formal names (§6.16). The compiler bails when one is
    /// used in any non-pass-through context (indexing, string methods,
    /// relational/arith operands): the interpreter routes those through the
    /// `string_signals` registry, which the compiled block cannot see.
    pub string_formals: std::rc::Rc<HashSet<String>>,
    /// class-perf collection-return: the return type is a queue / unsized
    /// dynamic array — the body returns via the `pending_ret_collection`
    /// side channel; the result cell is a width-0 passthrough.
    pub is_collection_result: bool,
    /// class-perf Step 9b: member-collection names callable with a value-
    /// arg builtin (`size`/`num`, `exists`, `delete`, `push_back`/
    /// `push_front`, `pop_front`/`pop_back`, `insert`) — see the simulator's
    /// `class_coll_member_names`. Feeds the compiler's CallCollMethod
    /// receiver admission (instance collections).
    pub coll_members: std::rc::Rc<HashSet<String>>,
    /// The `static_collections` names of the class chain. BARE receivers
    /// only: their stores resolve through the interpreter's §8.10 rewrite
    /// (per-spec key, collision-aware); a dotted static receiver must stay
    /// AST (it reaches `exec_method_call`'s broken zero path).
    pub static_coll_members: std::rc::Rc<HashSet<String>>,
    /// class-perf Step 9e: the return is VOID (`function void`) or the
    /// method is a constructor (`function new`): no observable return
    /// value — the result register is a passthrough cell seeded from the
    /// frame's implicit return local (no resize, no signedness stamp).
    pub is_void_result: bool,
    /// class-perf Step 9b-ii: BARE element-access admission (`q[i]`,
    /// `aa[k]`) — the union of `coll_members`/`static_coll_members` MINUS
    /// every module-scope collection-table name. The AST WRITE funnel
    /// (`assign_value_index`) probes the module queues/arrays/assoc tables
    /// BEFORE its class-member arm, so a bare name that is also a module
    /// collection writes the MODULE store — an element insn would write the
    /// class store instead. (AST READS are class-first, so reads of a
    /// colliding name already diverge from writes in the interpreter;
    /// excluding the name keeps both shapes on the interpreter's exact
    /// behavior.) Element reads/writes only — CALLS stay admitted for the
    /// name (the AST call funnel is class-first too).
    pub coll_elem_members: std::rc::Rc<HashSet<String>>,
    /// issue-301 shape: for admitted coll-element members whose ELEMENT
    /// type is a MULTI-dimensional packed array (`bit [8:0][7:0] arr[3]`,
    /// queues/dynamic/associative alike), the element's full packed dims.
    /// A further select on the loaded element (`arr[j][2]`) is a packed
    /// ELEMENT select (an ew-bit slice), but the generic Index tail lowers
    /// it to a 1-bit bit-select of the loaded element — the compiler needs
    /// the dims to slice instead. Absent ⇒ single-vector elements, where
    /// the old bit-select lowering is correct.
    pub coll_elem_packed_dims: std::rc::Rc<HashMap<String, Vec<(i64, i64)>>>,
    /// Declared ELEMENT width of every member in the element-access plan
    /// set (fixed arrays, queues, associative members) — the width table
    /// `expr_max_width` consults for a member element select. Without it
    /// the select sized at 1 bit (see `member_elem_plan_width`).
    pub coll_elem_widths: std::rc::Rc<HashMap<String, u32>>,
    /// class-perf row 2: names of NON-INPUT formals (output / inout / ref —
    /// the interpreter's `output_bindings` copy-out set) admitted to the
    /// compiled path as plain scalars (integral / string / enum / class
    /// handle, no dimensions). After `exec_insns` the runtime copies these
    /// formals' registers back into the bound locals frame so the
    /// interpreter epilogue's write-back to the caller's actual observes
    /// the body's final values, exactly like the AST path's frame locals.
    /// Identity-bound aggregates (ref queues/arrays/structs) are NOT here
    /// — they still decline.
    pub writeback_formals: std::rc::Rc<Vec<String>>,
}

/// class-perf Step 4b: a cached, fully-lowered class-FUNCTION body plus the
/// register ids of its seeded `this` / result cells. One entry per compiled
/// (class, method, width-signature); shared across all calls to that method
/// so the body is lowered ONCE instead of re-compiled per call.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CompiledMethodEntry {
    pub block: std::rc::Rc<CompiledBlock>,
    pub this_reg: crate::compiler::bytecode::RegId,
    pub result_reg: crate::compiler::bytecode::RegId,
}

/// Result of probing / populating the compiled-class-method cache for a
/// (class, method, width-signature) key. `Block` holds the lowered body;
/// `Nil` marks a method that FAILED to lower (negative cache) so it is not
/// re-compiled on every call — it stays on the AST interpreter forever.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum CompiledMethodOutcome {
    Block(std::rc::Rc<CompiledMethodEntry>),
    Nil,
}

/// Decodes one on-disk method-cache entry (`m<key>.bin`). The cache's wire
/// format is the artifact format; readers outside the simulator (tests)
/// go through here so a format change touches one place.
pub fn decode_persisted_method_outcome(bytes: &[u8]) -> Option<PersistedMethodOutcome> {
    bincode::serde::decode_from_slice(bytes, xezim_core::xez_bincode_config())
        .ok()
        .map(|(outcome, _)| outcome)
}

/// class-perf persistent cache (XEZIM_METHOD_CACHE): the disk form of one
/// compiled class-method outcome — the Rc-free mirror of
/// `CompiledMethodOutcome`, bincode-serialized under a content key in the
/// method-cache directory. Loaded on the memory-cache miss path right
/// before re-running admission/compilation, so a warm run pays neither.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PersistedMethodOutcome {
    Block {
        block: CompiledBlock,
        this_reg: RegId,
        result_reg: RegId,
    },
    /// Negative outcome: this method declines compilation. Persisted so a
    /// warm run skips the admission walk too.
    Nil,
}

/// Compiler state for converting AST → bytecode.
#[derive(Clone)]
struct LocalArrayBind {
    regs: Vec<RegId>,
    lo: i64,
    elem_w: u32,
    is_real: bool,
    /// The elements' declared signedness when every element is stamped with
    /// it (an inlined array formal); None when unknown.
    elem_signed: Option<bool>,
}

/// A fixed unpacked local addressed by constant numeric index.
#[derive(Clone, Copy)]
struct LocalArrayBank {
    base: RegId,
    width: u32,
    len: usize,
    lo: i64,
    descending: bool,
    element: Option<LocalKind>,
}

/// A member of a packed-struct formal that an inlined body reads as a slice
/// of the formal's register.
#[derive(Clone)]
struct FormalMember {
    off: u32,
    width: u32,
    signed: bool,
    /// Declared range (`[7:4]`) that selects on the member are labeled by;
    /// None for a 1-bit member without one, or one of several dimensions.
    dim: Option<(i64, i64)>,
    /// The packed dimensions of a multi-dimensional packed-array member
    /// (`logic [3:0][4:0] sh`), outermost first: a select on it picks an
    /// element (`compile_formal_member_elem`), not a bit.
    elems: Option<Box<[(i64, i64)]>>,
    /// The struct is 2-state: an out-of-range element reads 0, not x.
    two_state: bool,
}

/// `format!("{}.{}", scope, name)` without the formatting machinery: the
/// compiler builds one such scoped key per identifier lookup.
fn cat_dot(scope: &str, name: &str) -> String {
    let mut s = String::with_capacity(scope.len() + 1 + name.len());
    s.push_str(scope);
    s.push('.');
    s.push_str(name);
    s
}

/// Per-register read counts over one instruction stream, kept in step with
/// edits: "does any instruction outside these slots read `r`" becomes a
/// count comparison instead of a scan of the whole block (the peephole
/// passes ask it once per candidate, which made them quadratic).
struct RegReaders {
    reads: Vec<u32>,
    fallbacks: u32,
}

impl RegReaders {
    fn new(insns: &[Insn]) -> Self {
        let mut rr = RegReaders {
            reads: Vec::new(),
            fallbacks: 0,
        };
        for insn in insns {
            rr.add(insn);
        }
        rr
    }

    fn bump(&mut self, insn: &Insn, up: bool) {
        let reads = &mut self.reads;
        let exact = BytecodeCompiler::insn_read_regs(insn, &mut |r| {
            let r = r as usize;
            if r >= reads.len() {
                reads.resize(r + 1, 0);
            }
            if up {
                reads[r] += 1;
            } else {
                reads[r] -= 1;
            }
        });
        if !exact {
            if up {
                self.fallbacks += 1;
            } else {
                self.fallbacks -= 1;
            }
        }
    }

    fn add(&mut self, insn: &Insn) {
        self.bump(insn, true);
    }

    fn remove(&mut self, insn: &Insn) {
        self.bump(insn, false);
    }

    /// `insns.iter().enumerate().any(|(k, x)| !skip.contains(&k) &&
    /// BytecodeCompiler::insn_reads_reg(x, r))` for distinct `skip` slots.
    fn read_outside(&self, insns: &[Insn], r: RegId, skip: &[usize]) -> bool {
        let mut own = 0u32;
        let mut own_fallbacks = 0u32;
        for &k in skip {
            if let Some(x) = insns.get(k) {
                if !BytecodeCompiler::insn_read_regs(x, &mut |q| own += (q == r) as u32) {
                    own_fallbacks += 1;
                }
            }
        }
        self.fallbacks > own_fallbacks || self.reads.get(r as usize).copied().unwrap_or(0) > own
    }
}

/// Reachable reads of one register definition, stopping at a replacement
/// definition on each path. Register IDs are recycled across case arms (§12.5),
/// so whole-stream read counts alone cannot distinguish their lifetimes.
struct RegLiveness {
    visited: Vec<u32>,
    epoch: u32,
    pending: Vec<usize>,
}

impl RegLiveness {
    fn new(len: usize) -> Self {
        Self {
            visited: vec![0; len],
            epoch: 0,
            pending: Vec::new(),
        }
    }

    fn read_before_write(&mut self, insns: &[Insn], starts: &[usize], reg: RegId) -> bool {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.visited.fill(0);
            self.epoch = 1;
        }
        self.pending.clear();
        self.pending.extend_from_slice(starts);
        // A conservative bound keeps repeated queries linear in stream size,
        // even when many recycled definitions have long reachable suffixes.
        let mut budget = 1024;
        while let Some(mut pc) = self.pending.pop() {
            while pc < insns.len() && self.visited[pc] != self.epoch {
                if budget == 0 {
                    return true;
                }
                budget -= 1;
                self.visited[pc] = self.epoch;
                let insn = &insns[pc];
                // An in-place update reads the old definition before replacing
                // it; opaque AST fallbacks conservatively read every register.
                if BytecodeCompiler::insn_reads_reg(insn, reg) {
                    return true;
                }
                if BytecodeCompiler::dest_reg(insn) == Some(reg) {
                    break;
                }
                match insn {
                    Insn::Jump(target) => {
                        pc = *target as usize;
                        continue;
                    }
                    Insn::BranchIfFalse(_, target)
                    | Insn::BranchUnlessZero(_, target)
                    | Insn::BranchIfSignalFalse(_, target, _)
                    | Insn::CmpBranch(_, _, _, _, target)
                    | Insn::CmpBranchConst(_, _, _, _, target) => {
                        self.pending.push(*target as usize);
                    }
                    Insn::CaseJump(_, jump) => {
                        self.pending.extend(jump.table.iter().map(|&t| t as usize));
                        pc = jump.default as usize;
                        continue;
                    }
                    Insn::CaseMaskJump(_, jump) => {
                        self.pending.extend(jump.table.iter().map(|&t| t as usize));
                        pc = jump.xz_path as usize;
                        continue;
                    }
                    _ => {}
                }
                pc += 1;
            }
        }
        false
    }
}

thread_local! {
    static SCOPED_KEY_BUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Run `f` on `scope.name` without allocating it: the key is built in a
/// reused buffer (a nested use falls back to a fresh string).
fn with_dotted<R>(scope: &str, name: &str, f: impl FnOnce(&str) -> R) -> R {
    SCOPED_KEY_BUF.with(|b| match b.try_borrow_mut() {
        Ok(mut buf) => {
            buf.clear();
            buf.push_str(scope);
            buf.push('.');
            buf.push_str(name);
            f(&buf)
        }
        Err(_) => f(&cat_dot(scope, name)),
    })
}

pub struct BytecodeCompiler<'a> {
    insns: Vec<Insn>,
    next_reg: u32,
    register_overflow: bool,
    signal_name_to_id: &'a NameMap,
    signal_signed: &'a [bool],
    signal_widths: &'a [u32],
    /// Per-signal `is_real`. Optional because only the simulator has it;
    /// absent means "assume possibly-real", which only costs missed
    /// `Resize` elisions, never correctness.
    signal_real: Option<&'a [bool]>,
    /// The runtime's per-signal 2-state marks (`Simulator::signal_two_state`),
    /// which its writes use to drop x/z (§6.11.2). `None` until set.
    signal_two_state: Option<&'a [bool]>,
    arrays: &'a HashMap<String, (i64, i64, u32)>,
    array_first_id: Option<&'a HashMap<Arc<str>, (usize, i64, i64)>>,
    widths: &'a dyn SignalWidthLookup,
    pub bail_reason: Option<&'static str>,
    /// When true, unsupported statements emit `StmtFallback` instead of
    /// failing compilation. Safe for edge blocks where the AST interpreter's
    /// statement path is the same one used by the non-compiled fallback.
    pub allow_ast_fallback: bool,
    /// Named blocks enclosing the statement being compiled, outermost first.
    /// A fallback that can print its scope runs inside them (`emit_fallback`).
    m_labels: Vec<crate::ast::Identifier>,
    /// Hierarchical scope for resolving unqualified identifiers. An Ident
    /// with a bare local name (`mem_valid`) is first tried verbatim, then
    /// with this prefix applied (`testbench.mem_valid`).
    pub scope_hint: Option<String>,
    /// §26.3: `pkg::member` -> (value, width) of every package enum member,
    /// shared by the module-code compilers (`pkg_enum_consts` is the
    /// method-mode snapshot). A member two packages both declare reaches
    /// the compiler as the two-segment `pkg.member` the inliner spells it
    /// with; resolving that by name landed on the bare signal twin.
    pkg_enum_consts_ref: Option<&'a HashMap<String, (u64, u32)>>,
    /// Enum literals visible in the method's lexical class (before global names).
    pub(crate) method_enum_constants: HashMap<String, Value>,
    /// `Class::MEMBER` enum literals of every class (method mode).
    pub(crate) class_scoped_enums: Option<std::rc::Rc<HashMap<String, Value>>>,
    /// Process-FSM mode (roadmap 11-12): statement-level `#delay` and
    /// `@(event)` compile to WaitDelayReg / WaitEdge instead of bailing.
    /// Never set for edge or comb blocks.
    pub allow_waits: bool,
    /// A timed process body that runs once per wakeup without waits of its
    /// own (the retained `always #d x = f(x);` assignment). Like a process
    /// FSM body it may run a call of a function with side effects on the
    /// interpreter (§13.4). Never set for edge or comb blocks.
    pub delegate_calls: bool,
    /// Event-control specs referenced by WaitEdge, in emission order.
    pub wait_specs: Vec<crate::ast::stmt::EventControl>,
    /// Per-for-loop leaf-name → signal_id override. Set by `compile_stmt`'s
    /// For arm before compiling condition/step expressions, cleared after.
    /// Re-routes bare-ident lookups for the loop variable so that the step
    /// `i = i+1` writes to the same signal as the init `i = 0`, even when
    /// the elaborator only scope-qualified init's lvalue (see compile_for
    /// for the full c910 hang context).
    pub for_loop_var_ids: std::collections::HashMap<String, usize>,
    /// Block-LOCAL variables held in bytecode registers rather than signals —
    /// currently a `for (int i = ...)` loop variable, which has no signal at
    /// all (§12.7.1 makes it automatic and local to the loop). Without this the
    /// whole loop fell back to the AST interpreter.
    pub local_var_regs: std::collections::HashMap<String, (RegId, u32)>,
    /// class-perf method-body mode (§X): when true, the compiler is
    /// lowering the BODY of a class method rather than an always block. In
    /// this mode `this.<member>` and `<class-formal/local>.<member>` lower
    /// to LoadClassMember/StoreClassMember instead of hierarchical-signal
    /// reads, and formals/locals live in VM registers. Any statement that
    /// cannot lower this way fails the WHOLE method (all-or-nothing) so the
    /// runtime falls back to the proven AST interpreter — byte-identical by
    /// construction.
    method_mode: bool,
    /// VM register holding the receiving object's heap handle (`this`).
    /// Seeded by the method-mode prologue before the body runs.
    method_this_reg: Option<RegId>,
    /// Method-local/formal names that hold CLASS HANDLES (not scalars), so a
    /// `.member` on them lowers to a heap access. Scalar formals/locals are
    /// already in `local_var_regs`; only class-typed ones need this set.
    method_handle_names: std::collections::HashSet<String>,
    /// class-perf P2 (typed handle roots): local/formal name -> declared
    /// class; resolves dispatch chains rooted at handle locals statically.
    method_handle_local_types: HashMap<String, String>,
    /// §6.21: the method body's STATIC locals lowered to register-backed
    /// slots whose value persists across calls. The dispatch shell seeds
    /// each register from `static_local_vars` before the block runs and
    /// copies the final register back into the locals frame afterwards so
    /// the exit sync persists it. The block itself NEVER seeds the register:
    /// a compile-time default would let `fold_const_regs` fold e.g.
    /// `m_inst == null` to true, silently constructing a fresh singleton on
    /// every call. `(name, reg, default)`: `default` is the first-call
    /// seed, exactly the AST declaration's.
    method_static_locals: Vec<(String, RegId, Value)>,
    /// class-perf P2: method names returning `string` in every declaring
    /// class — string-concat operand typing for method-call operands.
    method_string_ret_names: HashSet<String>,
    /// class-perf P2: package-scope enum constants `pkg::NAME` -> value,
    /// for `uvm_pkg::UVM_NONE`-style report-macro arguments. Width>64
    /// entries are omitted (those decline to the AST, which reads the
    /// full Value from the parameter table).
    pkg_enum_consts: HashMap<String, (u64, u32)>,
    /// Names declared as members/statics by the method's class OR any
    /// ENCLOSING class (so a nested method can lexically reach an outer
    /// static). In method mode a BARE Ident matching one of these names must
    /// resolve to class scope, NOT to a module-level signal of the same name;
    /// the compiled path does not model class-static storage, so it bails
    /// (falls to the AST interpreter) rather than read the wrong thing.
    class_shadow_names: HashSet<String>,
    /// BARE idents (single segment, no selects) that resolve to
    /// `this.<member>` in method mode: directly-declared or extends-inherited
    /// INSTANCE properties of the method's class that are NOT shadowed along
    /// the chain. A bare name here compiles to LoadClassMember(this_reg,
    /// name) instead of bailing.
    bare_member_names: HashSet<String>,
    /// Instance properties of the extends chain whose declared type IS a
    /// class (through typedefs/forward typedefs). Only these may be
    /// method-DISPATCH bases in method mode: `value.rand_mode()` on an
    /// integral member (`rand bit [63:0] value`) is a BUILTIN variable
    /// method, not a class-method dispatch — lowering it to CallMethod over
    /// the raw member bits silently returned 0 (uvm_reg_field::get_rand_mode
    /// regressed exactly this way). Integral members stay in
    /// `bare_member_names` (plain loads are fine); this set is the
    /// intersection the dispatch gate requires.
    member_class_names: HashSet<String>,
    /// Step 6: instance properties of the extends chain whose DOTTED access
    /// (`base.m`) is faithful by runtime bare key — see the simulator-side
    /// doc. A member in `class_shadow_names` but ALSO here lowers to a
    /// Load/StoreClassMember; in shadow but NOT here it bails.
    member_safe_names: HashSet<String>,
    /// Step 6: body VarDecl names the simulator identified as CLASS-typed
    /// (through typedefs). A declaration of one of these holds a heap HANDLE
    /// (32-bit register, default null), and the name joins
    /// `method_handle_names` so `.member` on it is a heap access.
    class_local_names: HashSet<String>,
    /// class-perf Step 9e: METHOD names of the enclosing class chain
    /// (plan-provided). A bare-name call (`helper(2)`) inside a method body
    /// is this-bounded (class scope shadows module functions per SV name
    /// resolution) and lowers to a CallMethod on the `this` register; the
    /// runtime re-enters the dispatcher, so virtual dispatch, defaults and
    /// ref write-backs keep their AST semantics (a function body cannot
    /// contain a task call per §13.3, so the callee is always a function).
    class_method_names: HashSet<String>,
    /// Step 9f: method names visible in the PARENT chain of the enclosing
    /// class (+ `new`) — the `super.m` admission set.
    super_method_names: HashSet<String>,
    /// Step 9f: the parent of the enclosing (defining) class — the baked
    /// start class for `super.m`.
    method_class_parent: Option<String>,
    /// Step 9f: BARE-call static targets — non-virtual method name -> the
    /// first defining class from the lexical chain (§8.20).
    static_bare_targets: HashMap<String, String>,
    /// LRM 11.6/11.8.2: declared width of every input port of every method
    /// visible in the enclosing class chain, keyed by method name.  A call
    /// ACTUAL is context-determined by the callee's declared formal — with
    /// `function bit wide9(logic [8:0] v)`, the call site `wide9(a + b)`
    /// must evaluate `a + b` at 9 bits (300), not at its 8-bit self width
    /// (44).  The inline path already gets this from `port_effective_width`;
    /// the CallMethod/CallScopedMethod lowerings compile their actuals
    /// against `ctx_width = 0` and so silently truncated.  Missing entry or
    /// width 0 = keep the old self-determined behavior.
    method_formal_widths: HashMap<String, Vec<u32>>,
    /// Method mode: the output / inout / ref formals. The call binds a ref
    /// as a redirection to the caller's actual while the compiled body keeps
    /// it in a register until its epilogue, so a statement fallback carries
    /// it with that redirection set aside (`via_binding`).
    method_writeback_names: HashSet<String>,
    /// Method mode: names whose declared type the activation records before
    /// the body runs — formals, top-level body declarations and the result
    /// cell. A carried class handle must be one of these, so a fallback's
    /// member access and dispatch see its declared class.
    method_bound_names: HashSet<String>,
    /// Registers of class-handle locals declared in a nested block over a
    /// bound name, which a statement fallback cannot carry.
    method_shadowing_regs: HashSet<RegId>,
    /// Step 9g: WRITABLE static-property names (bare) of the enclosing
    /// class chain — reads AND stores lower to Load/StoreClassStatic.
    static_member_names: HashSet<String>,
    /// Step 9g: read-only class-CONSTANT names (localparams; live in
    /// `param_defaults`). Reads lower to LoadClassStatic; stores decline.
    class_const_names: HashSet<String>,
    /// Step 9g: bare name -> the DECLARED class type of that lvalue
    /// (locals, statics, and instance members of the enclosing chain) —
    /// the target of a `lhs = new(...)` lowering. Only non-parameterized
    /// targets are admitted (compile-site filtered).
    new_type_map: HashMap<String, String>,
    scope_static_receivers: HashSet<String>,
    /// class-perf Step 9e: the static CLASS of the method being compiled —
    /// the `this` root's type for TYPED handle-chain admission.
    method_class: Option<String>,
    /// class-perf Step 9e: per-class map of member name -> its static class
    /// type (plan-provided), for TYPED handle-chain admission on DISPATCH
    /// receivers. See `method_handle_chain_class`.
    handle_member_types: Option<&'a HashMap<String, HashMap<String, String>>>,
    pub(crate) method_admission: Option<std::rc::Rc<MethodAdmission>>,
    /// Per-specialization instance field widths and signedness for `this` when
    /// the class chain has value parameters (where `MethodAdmission` cannot
    /// hold one global fixed width).
    pub(crate) this_field_widths: HashMap<String, u32>,
    pub(crate) this_field_signed: HashSet<String>,
    /// Formal declarations supply the same local select metadata as body locals.
    method_formal_types: Option<&'a [crate::ast::decl::FunctionPort]>,
    /// The implicit result is a local of the declared return type too.
    method_return_type: Option<&'a crate::ast::types::DataType>,
    /// Register holding the (implicit) function result cell, so `return` and
    /// `f = ...` writes land where the method-mode prologue reads the result.
    method_result_reg: Option<RegId>,
    /// Register holding a `return <expr>` VALUE on its way to the caller.
    /// Distinct from the function's implicit result cell only for explicit
    /// `return e`; the prologue returns this register's value.
    method_return_val_reg: Option<RegId>,
    /// Declared width of the method's (implicit) result, for re-sizing an
    /// explicit `return e`. Zero-width / void returns leave this unset.
    method_result_width: Option<u32>,
    /// True when the method returns a CLASS HANDLE: the result `Value` is a
    /// heap index passed through UNTOUCHED (no resize/stamp), exactly like the
    /// interpreter, which never resizes or stamps a non-`plainly_integral`
    /// return. Lets the Return handler tolerate an absent result width.
    method_result_is_class: bool,
    /// True when the method returns a `string`: like the class-handle case,
    /// the result Value (a byte vector) passes through untouched — the Return
    /// handler tolerates an absent width and skips the Resize.
    method_result_is_string: bool,
    /// class-perf collection-return: the method's return type is a queue /
    /// unsized dynamic array. `return '{…}` literals lower to `RetCollection`
    /// (side-channel hand-off); other return expression forms bail.
    method_ret_is_collection: bool,
    /// class-perf free-function tier: compiling a package/module-scope
    /// FUNCTION (no class scope). Bare names outside the frame (package
    /// variables, statics) are unresolvable storage for the compiled block
    /// — any read of one declines the whole body (see the Ident arm).
    free_function_mode: bool,
    /// §26.3: the package that declares the subroutine or class being
    /// compiled in method mode, and the table holding its `pkg::name`
    /// parameters.
    pub method_pkg: Option<(String, &'a HashMap<String, Value>)>,
    /// §26.3: every package's imports (`def_pkg_imports`), for the names a
    /// package's code reads through them.
    pub method_pkg_imports: Option<&'a HashMap<String, Vec<(String, Option<String>)>>>,
    /// The declaring package of each inlined callee on `inlining_stack`
    /// (`None`: a callee declared outside a package).
    inline_pkg_stack: Vec<Option<String>>,
    /// String-typed formal names (plan-provided). Any use of one outside the
    /// pass-through surface (read, compare, assign, concat, call arg, return)
    /// bails: indexing/string-methods/arith take interpreter-only paths.
    string_formal_names: HashSet<String>,
    /// class-perf Step 9a: string-typed INSTANCE members of the method's
    /// class chain (plan-provided). A bare/dotted read of one lowers to a
    /// LoadClassMember — the heap stores the string as a plain byte-vector
    /// Value and the interpreter's dotted read passes it through verbatim
    /// (fit_class_prop no-ops on `class_prop_width_impl == None`, which
    /// strings return). Stores stay AST for now (width-0 `Resize` to a
    /// string local is a no-op; a store without one risks a width-mutated
    /// value landing in the heap slot).
    string_member_names: HashSet<String>,
    /// class-perf Step 9b: member-collection names the runtime may resolve
    /// for the method's class chain. The BARE-receiver set is the union of
    /// `assoc_properties`/`queue_properties` keys over the extends chain
    /// (instance stores; statics are deliberately absent) — a bare
    /// `q.size()`/`m.exists(k)` inside a method body lowers to a
    /// CallCollMethod whose runtime resolution re-runs the interpreter's own
    /// §8.10 rewrite (`instance_assoc_member`) on the live `this`, so the
    /// store answer is byte-identical even for param-bound collections the
    /// plan set cannot see. The DOTTED-receiver set is the same union PLUS
    /// `static_collections` — dotted statics (`Cls::m`-flattened forms
    /// aside) are excluded at the call site instead, where only a bare
    /// receiver is admitted for a static name (a dotted static otherwise
    /// reaches `exec_method_call`'s broken zero path).
    coll_member_names: HashSet<String>,
    /// class-perf Step 9b: `static_collections` names of the class chain.
    /// Bare-receiver admission only (see `PreboundCompiledMethod::
    /// static_coll_members`): their stores resolve through the interpreter's
    /// §8.10 rewrite (per-spec key, collision-aware); a dotted static
    /// receiver must stay AST (it reaches `exec_method_call`'s broken zero
    /// path).
    static_coll_member_names: HashSet<String>,
    /// class-perf Step 9b-ii: BARE element-access admission — the union of
    /// the two sets above minus module-scope collection names (see
    /// `PreboundCompiledMethod::coll_elem_members`). The DOTTED element arm
    /// keeps using `coll_member_names` (a MemberAccess base cannot collide
    /// with a module table name).
    coll_elem_member_names: HashSet<String>,
    /// Mirrored plan fact (`PreboundCompiledMethod::coll_elem_packed_dims`):
    /// coll-element members whose ELEMENT is a multi-dimensional packed
    /// array — `arr[j][2]` must select the packed ELEMENT (ew-bit slice),
    /// not one bit of the loaded element.
    pub coll_elem_packed_dims: HashMap<String, Vec<(i64, i64)>>,
    /// Mirrored plan fact (`PreboundCompiledMethod::coll_elem_widths`):
    /// declared element widths of the element-access plan members.
    pub coll_elem_widths: HashMap<String, u32>,
    /// Unpacked dims of the typedefs this body's locals are declared with
    /// (`typedef int q_t[$]; q_t q;`), keyed by the type's source spelling
    /// (`p::q_t`). The declarator itself carries no dims, so without this a
    /// typedef'd collection local lowered as a scalar register.
    pub typedef_coll_dims: HashMap<String, Vec<crate::ast::types::UnpackedDimension>>,
    /// The `typedef_coll_dims` keys whose ELEMENT type is `string`
    /// (`typedef string q_t[$]`): such locals join `local_coll_strings`.
    pub typedef_coll_strings: HashSet<String>,
    /// Method mode: instance-member collections whose ELEMENT type is
    /// `string` (`Simulator::class_string_coll_member_names`).
    pub member_coll_strings: HashSet<String>,
    /// class-perf Step 9c: names whose `$cast` destination may take the
    /// register route — formals, the result cell, and TOP-LEVEL body
    /// locals. A nested-block local is register-backed too, but its
    /// declared type is never recorded in the runtime overlay (the
    /// interpreter records a decl's type when the VarDecl executes, and
    /// nested blocks are not pre-recorded at method entry), so
    /// `cast_type_ok` could consult a stale cross-frame entry and disagree
    /// with the AST path. Such dests decline.
    cast_reg_ok_locals: HashSet<String>,
    /// Indices of the placeholder `Jump(0)` emitted by method-mode `return`;
    /// back-patched to the method's common exit at the end of the body.
    method_ret_jumps: Vec<usize>,
    /// Names introduced by a `VarDecl` INSIDE the compiled block that live only
    /// in a VM register — the interpreter never ran the declaration, so it has
    /// no storage for them at all.
    ///
    /// A per-statement `StmtFallback` re-runs one statement in the interpreter.
    /// If that statement touches one of these names it reads a signal that does
    /// not exist (x), while the compiled statements around it read the register
    /// — one block, two different variables. `always_ff begin item_t item; …
    /// q.push_back(item); end` pushed an all-x item on every cycle because the
    /// push could not be compiled and fell back. Same failure mode as
    /// `reg_var_loop_depth`, which guards the loop-counter case; this guards
    /// the block-local-declaration case.
    ///
    /// Distinct from `local_var_regs`, which ALSO holds `set_process_locals`
    /// bindings — those do have an interpreter home and must not block a
    /// fallback.
    decl_local_regs: std::collections::HashSet<String>,
    /// Names imported from an already-active process frame. These registers
    /// are read-only for the compiled statement; writes leave compilation so
    /// the process interpreter can preserve full local-lifetime semantics.
    process_local_names: HashSet<String>,
    /// Declared type of each register-backed local that an AST fallback may
    /// carry (see `FbLocal`), keyed by its register. A local bound without an
    /// entry (an unsupported type, a process-frame import) makes any fallback
    /// that names it refuse, so the enclosing unit is interpreted instead.
    local_kinds: HashMap<RegId, LocalKind>,
    /// Enclosing inlined task bodies declared in a DIFFERENT scope than the
    /// block being compiled. A fallback inside one would print the block's
    /// `%m` and use its timescale, so none is emitted there.
    inline_foreign: u32,
    /// Depth of enclosing loops whose counter lives in a VM REGISTER
    /// (`for (int i = ...)`). While > 0, StmtFallback emission is FORBIDDEN:
    /// the AST interpreter cannot see VM registers, so a fallback statement
    /// inside such a loop silently reads the loop var as X. Any unsupported
    /// construct must instead fail the whole loop back to the AST path.
    reg_var_loop_depth: u32,
    /// Ranges of active register-backed loop counters proven unmodified in the body.
    loop_index_bounds: HashMap<RegId, (i64, i64)>,
    /// class-perf Step 9d: number of runtime key slots this compiled block
    /// needs — one per `foreach` over a member collection. The arena itself
    /// lives on the Simulator (swapped per method run so nested compiled
    /// calls never alias an outer loop's keys); the compiler only records
    /// how many distinct slots the block addresses.
    foreach_slot_count: u32,
    /// Expression-level fallback is only sound where surrounding analysis
    /// doesn't need to SEE the expression's reads: edge blocks, whose
    /// sensitivity is the explicit clock list. Comb entries build their
    /// wake-up graph from LoadSignal scans, so a read hidden inside an
    /// interpreted fragment would stop the entry re-firing. Off by default;
    /// enabled only at the edge-block compile site.
    allow_expr_fallback: bool,
    /// User-task table for inlining zero-arg, non-blocking task bodies.
    /// Task-enable (`task_name;`) statements that resolve here get their
    /// bodies compiled in place instead of emitting a single StmtFallback
    /// for the whole call — lets the inner simple assigns compile cleanly
    /// and narrows the fallback to just the inner $write/$display.
    tasks: Option<&'a HashMap<String, TaskDeclaration>>,
    functions: Option<&'a HashMap<String, FunctionDeclaration>>,
    function_scope_index: Option<&'a FunctionScopeIndex<'a>>,
    /// Deferred per-instance callable rewrite contexts (#193), materialized
    /// on demand when bytecode inspects or inlines an instance subroutine.
    deferred_functions: Option<&'a HashMap<String, std::rc::Rc<super::elaborate::CallableRewriteCtx>>>,
    deferred_tasks: Option<&'a HashMap<String, std::rc::Rc<super::elaborate::CallableRewriteCtx>>>,
    deferred_packages: Option<&'a HashSet<String>>,
    /// A scalarized stand-in for the next `compile_pure_call`'s callee (see
    /// `compile_struct_member_call`); taken at the function-table lookup.
    struct_call_fd: Option<FunctionDeclaration>,
    /// §7.2: unpacked-struct variables, by container signal id, with their
    /// member leaves as (member path, leaf signal name); an empty list marks
    /// one whose leaves are not plain signals (see `UnpackedStructLeaves`).
    unpacked_structs: Option<&'a UnpackedStructLeaves>,
    dpi_import_fds: Option<std::rc::Rc<HashMap<String, FunctionDeclaration>>>,
    inlining_stack: Vec<String>,
    pub tasks_inlined: u32,
    /// Elaborated module parameters — used by `eval_const_expr` so that
    /// bytecode compilation can fold module params (e.g. `CARRY_CHAIN`) into
    /// the compile-time widths of `+:` / `-:` range selects.
    params: Option<&'a HashMap<String, Value>>,
    /// Leaf-segment index over `params` (last dotted segment -> full keys).
    /// The suffix-match fallback in `lookup_param_value` otherwise scans the
    /// WHOLE param map with memcmp per entry on every miss — 94% of the
    /// C910 SoC's 410s compile phase.
    param_leaf_idx: Option<&'a HashMap<String, Vec<String>>>,
    /// Typedef name -> total width (module + package scope), for local
    /// declarations of typedef'd packed types inside inlined functions.
    typedefs: Option<&'a HashMap<String, u32>>,
    /// Typedef name -> packed ELEMENT width (`typedef u8_t [15:0] v_t` -> 8).
    typedef_elems: Option<&'a HashMap<String, u32>>,
    /// Concrete typedef targets retain signedness and default state as well as width.
    typedef_types: Option<&'a HashMap<String, crate::ast::types::DataType>>,
    /// Lexical class aliases are not all hoisted into the module table.
    local_typedef_types: HashMap<String, crate::ast::types::DataType>,
    /// Runtime-bound class type parameters must not capture module typedefs.
    unresolved_type_names: HashSet<String>,
    /// Register-backed local -> packed element width, for `local[i]`
    /// splice/extract compilation.
    pub local_var_elem: std::collections::HashMap<String, u32>,
    /// Packed-struct formals of an inlined pure function: name -> (the
    /// formal's register, its flattened members). Keyed like
    /// `local_var_regs`, and honored only while the name still resolves to
    /// that register.
    local_var_struct: HashMap<String, (RegId, Vec<(String, FormalMember)>)>,
    /// Packed geometry follows a local's register through inlined frames.
    local_packed_types: HashMap<RegId, crate::ast::types::DataType>,
    /// Loop variables bound to COMPILE-TIME constants by the unroller. Read
    /// before every other name source, so `state[col*4]` folds to a constant
    /// index while `col` is unrolled.
    local_const_vars: HashMap<String, Value>,
    /// Declared type of each unrolled loop variable, for a fallback that
    /// carries it (`fb_carry`); absent when the type is not carryable.
    local_const_kinds: HashMap<String, LocalKind>,
    /// Register-bank locals: a small fixed-size local unpacked array whose
    /// elements live in consecutive VM registers. Only CONSTANT indexes address a
    /// bank — the registers have no runtime indexing — so any dynamic access
    /// fails the enclosing compile, which rolls back to the AST path.
    local_array_regs: HashMap<String, LocalArrayBank>,
    /// class-perf row 3: bare names of LOCAL queue / dynamic-array
    /// declarations compiled in this method body (delegated to the
    /// interpreter via a StmtFallback of the declaration, so the per-call
    /// `@name#id` storage and queue bookkeeping live in the interpreter's
    /// own tables). Each use routes through `local_coll_key`'s `\x01`
    /// marker so the runtime resolves the store via `dyn_name_lookup` —
    /// the same precedence as the AST path for a bare local receiver.
    local_coll_names: HashSet<String>,
    /// The subset of `local_coll_names` whose ELEMENTS are `string`s
    /// (`string q[$]`). Their element reads are string operands: §6.16 gives
    /// a string no fixed width, so they are never resized to a context and
    /// compare by text.
    local_coll_strings: HashSet<String>,
    /// §7.5, §7.10: the declared ELEMENT width and signedness of each
    /// integral `local_coll_names` entry. An element read lowers to
    /// `LoadCollElem`, which no other width table knows, so a
    /// self-determined compare (`a[0] == a[1]`) sized both reads at one bit.
    local_coll_elems: HashMap<String, (u32, Option<bool>)>,
    /// §10.8: the type an untyped assignment pattern takes, installed right
    /// before compiling an assignment-like source (an assignment RHS, an
    /// inlined formal's actual, a return value). `compile_expr_typed` takes
    /// it, and passes it on only to a parenthesised or conditional operand.
    pattern_ctx: Option<crate::ast::types::DataType>,
    /// The return type of the function being inlined, for `return '{...}`.
    inline_ret_ty: Option<crate::ast::types::DataType>,
    /// Named-cast targets known at compile time: name -> (width, signed).
    /// See the simulator's `cast_widths`.
    cast_widths: Option<&'a HashMap<String, (u32, bool)>>,
    /// `expr_signedness` of a dotted identifier (a hierarchical, interface,
    /// package or member name), by name: an operator re-types its operand
    /// subtrees, so a long expression asks for the same leaf many times.
    /// Cleared with the scope hint, which name resolution depends on.
    dotted_ident_sign: HashMap<String, Option<bool>>,
    /// `packed_named_select_signed`'s declaration lookup, by variable name:
    /// the select depth that reaches a named signed element, if any.
    named_elem_depth: HashMap<String, Option<usize>>,
    /// Top-module name (e.g. "tb"). When a hierarchical identifier reads a
    /// signal whose absolute path is `<top>.<rest>` (e.g. xezim's
    /// port-rewriting baked the top name into a cross-hierarchical
    /// reference) the signal table actually stores the leaf as `<rest>`,
    /// because top-level instances have no prefix in the elaborated map.
    /// `lookup_signal_id` strips this prefix before re-trying the lookup
    /// to recover from those baked-in absolute paths.
    pub top_module_name: Option<String>,
    /// Per-signal packed-element width for multi-D packed vectors
    /// (e.g. `logic [3:0][7:0] x` → elem_w=8). Used by `compile_blocking_target`
    /// so that `x[i] = v` emits a 8-bit slice write at `i*8 +: 8` instead of
    /// the default bit-select-write (`BlockingAssignBitDyn`) which only sets
    /// bit `i` and silently drops the upper bits. Set via
    /// `set_packed_elem_widths`.
    packed_elem_widths: Option<&'a xezim_core::shared_names::SharedNameMap<u32>>,
    /// Declared element width of each associative array (§10.7). Without it an
    /// assoc lvalue fell through `infer_lhs_width` to the 1-bit "bit-select on
    /// a plain packed signal" default, so a compiled `aa[k] <= v` truncated the
    /// value to a single bit.
    assoc_elem_widths: Option<&'a HashMap<String, u32>>,
    /// Names of ASSOCIATIVE arrays. Their keys are not dense indices and their
    /// elements have no signal ids, so none of the bytecode store paths can
    /// address them: `lookup_array_name` misses (they are not in `arrays`), and
    /// the fall-through treated the base as a scalar and wrote a BIT of a
    /// phantom signal — an `aa[k] = v` inside an `always_ff` was silently lost
    /// (`exists()` stayed 0) while the same write from an `initial` block, which
    /// runs on the AST path, worked. Detected here so the statement bails to
    /// that AST path instead.
    assoc_arrays: Option<&'a HashMap<String, bool>>,
    /// Declared packed dimensions (outermost first) per signal, from the
    /// elaborated model. Needed because a packed element's LSB offset is
    /// `(idx - low_bound) * elem_w` for a DESCENDING range — the plain
    /// `idx * elem_w` is only correct for a normalized `[N-1:0]` range and
    /// mis-places (or drops) elements of e.g. `[2:1]` or an ascending `[0:1]`.
    packed_full_dims: Option<&'a xezim_core::shared_names::SharedSliceMap<(i64, i64)>>,
    /// Declared variable types, for the type a pattern assigned to a
    /// variable takes (§10.8).
    var_decl_types: Option<&'a xezim_core::decl_types::DeclTypeMap>,
    /// Unpacked dimensions of typedefs (`typedef int arr_t[3]`), by the
    /// `typedef_types` key: such a type is not packed.
    typedef_unpacked_dims: Option<&'a HashMap<String, Vec<crate::ast::types::UnpackedDimension>>>,
    /// Declared types of explicitly typed value parameters.
    param_decl_types: Option<&'a xezim_core::decl_types::ParamDeclTypeMap>,
    /// Ports collapsed onto the nets they connect, formal to actual.
    port_aliases: Option<&'a xezim_core::shared_names::SharedNameMap<xezim_core::shared_names::SharedName>>,
    /// Stack of pending `break` jump-target patches, one entry per enclosing
    /// loop. When the loop's end address is known we rewrite each `Jump(0)`
    /// at these insn-indices to the loop-exit address. LRM §12.7.
    loop_break_patches: Vec<Vec<usize>>,
    /// Same stack-of-Vecs shape, but for `continue` — patched to the loop's
    /// step (or condition-recheck) address.
    loop_continue_patches: Vec<Vec<usize>>,
    /// Stack of pending `disable <named_block>` jump-target patches, by block name.
    /// LRM §9.6.2: disabling an active named block terminates that block.
    named_block_patches: HashMap<String, Vec<usize>>,
    /// Set of signal names declared as `string` (LRM §6.16). When a
    /// concatenation involves any of these, the bytecode bails to the AST
    /// interpreter, which has byte-level (not bit-level) concat semantics.
    /// Set via `set_string_signals`. None = no string info available, in
    /// which case the compiler can only catch the literal-operand case.
    string_signals: Option<&'a HashSet<String>>,
    /// Recursion guard for `fn_is_pure_in`: its `Call` arm recurses into the
    /// CALLEE's purity, so a self- or mutually-recursive function walked the
    /// call graph forever and overflowed the stack (an `assign w = fact(6);`
    /// with a recursive `fact` aborted the process; the same call from an
    /// initial block was fine because only the compiled path asks about
    /// purity). Past the cap the answer is "not pure", which merely keeps the
    /// call on the AST interpreter — where recursion already works.
    purity_depth: std::cell::Cell<u32>,
    /// Set when a purity check hit `MAX_PURITY_DEPTH` (its `false` is then
    /// depth-dependent and not memoized).
    purity_cut: std::cell::Cell<bool>,
    /// Local/formal names (in the current inline scope) bound to STRING
    /// values — drives `%s` semantics and resize suppression. Name-based on
    /// purpose: register ids are recycled across arms, names are not.
    local_var_is_string: HashSet<String>,
    /// Local/formal names (in the current inline scope) bound to REAL values
    /// (§13.3.1). Assignments into these coerce numerically via
    /// `emit_to_real` — a plain register bind would leave an integral actual
    /// as integer bits and `t / 2` style body arithmetic would truncate.
    local_var_is_real: HashSet<String>,
    /// SMALL fixed-shape local arrays of an inlined pure body (`real row
    /// [0:3]`): one register per element, values kept WHOLE (real elements
    /// stay real — no bit packing). A dynamic index compiles to a
    /// compare/branch chain over the elements.
    local_var_array: HashMap<String, LocalArrayBind>,
    /// Loop variables currently bound to a compile-time CONSTANT (an
    /// unrolled `foreach` iteration). Consulted by `eval_const_expr`, so an
    /// index like `drivers[i]` folds to a direct element register instead of
    /// a compare/branch chain.
    const_var_binds: HashMap<String, u64>,
    /// Active inline return context: (result register — None for a void
    /// function or task —, resize width; 0 = none, e.g. strings). `return`
    /// compiles to result-move + jump; the jump indexes collect in
    /// `inline_ret_jumps` and the inliner patches them to its body end.
    inline_ret: Option<(Option<RegId>, u32)>,
    /// The inlined function's declared return type is real (§13.4.1: an
    /// integral `return` value converts to it).
    inline_ret_real: bool,
    inline_ret_jumps: Vec<usize>,
    /// Base names of 2D/ND UNPACKED arrays. When a continuous-assign LHS
    /// `m[0][j]` targets one of these, the flattening short-circuit
    /// (`flattened_outer_const_signal_id`) must NOT fire — the
    /// bogus scalar signal `m` would otherwise catch a bit-select write and
    /// silently drop the element. None = no info (older callers); the guard
    /// then only excludes 1D/packed bases as before. Set via
    /// `set_multi_dim_arrays`.
    multi_dim_arrays: Option<&'a HashSet<String>>,
    /// 2-D unpacked array shapes (`module.arrays_2d`): base ->
    /// ((lo1,hi1),(lo2,hi2),elem_w). Elements are materialized contiguously
    /// row-major from `base[lo1][lo2]`, so a DYNAMIC `a[i][j]` read compiles
    /// to a bounds-checked flat index over one Dense operand.
    arrays_2d: Option<&'a HashMap<String, ((i64, i64), (i64, i64), u32)>>,
    /// Names the elaborator recorded as DYNAMIC arrays / QUEUES. Used only to
    /// keep their element STORES off the dense array fast path (see
    /// `collection_store_denied`); their READS are unaffected.
    dynamic_arrays: Option<&'a HashSet<String>>,
    queue_vars: Option<&'a HashSet<String>>,
    /// Packed-struct field layout: container name → ordered
    /// `(member, lsb_offset, width)`. Lets a member-write LHS like
    /// `s.m0` (parsed as a 2-segment `Ident(["<scope>.s", "m0"])` after
    /// submodule inlining) compile to a constant bit-range write into the
    /// container signal, instead of bailing to the AST interpreter — where
    /// its read dependency would resolve bare-first to the wrong (top-scope)
    /// input and never re-trigger when the real scoped input changes. Set via
    /// `set_packed_struct_fields`.
    packed_struct_fields: Option<&'a xezim_core::shared_names::SharedSliceMap<(String, u32, u32)>>,
    /// Declared named-event variables (`module.events`), so bytecode bails on
    /// event operands/assignments rather than reading/writing their 1-bit
    /// backing signals directly.
    events: Option<&'a HashSet<String>>,
}

/// Where `packed_path_dyn_range` places a store in its root signal.
enum PackedPathTarget {
    Ignore,
    /// `width` bits from the bit in register `lo`.
    Range(RegId, u32),
    /// A part-select with a run-time base, which may run past either end of
    /// its dimension (see `emit_slot_run_store`).
    Slots(SlotRun),
}

/// `n` consecutive slots of `ew` bits, the lowest at run-time slot `first`,
/// of a dimension of `count` slots whose slot 0 starts at the bit in `base`.
/// Value bits `[k*ew +: ew]` go to slot `first + k`.
struct SlotRun {
    base: RegId,
    first: RegId,
    n: u64,
    ew: u32,
    count: u64,
}

/// The longest `SlotRun` whose elements are stored one at a time when it
/// leaves its dimension. Each takes a few registers and instructions, so a
/// longer run is stored as one range with run-time ends instead.
const MAX_SLOT_RUN: u64 = 16;

impl<'a> BytecodeCompiler<'a> {
    pub fn new(
        signal_name_to_id: &'a NameMap,
        signal_signed: &'a [bool],
        signal_widths: &'a [u32],
        arrays: &'a HashMap<String, (i64, i64, u32)>,
        widths: &'a dyn SignalWidthLookup,
    ) -> Self {
        Self {
            insns: Vec::with_capacity(64),
            next_reg: 0,
            register_overflow: false,
            signal_name_to_id,
            signal_signed,
            signal_widths,
            signal_real: None,
            signal_two_state: None,
            arrays,
            array_first_id: None,
            widths,
            bail_reason: None,
            allow_ast_fallback: false,
            m_labels: Vec::new(),
            scope_hint: None,
            pkg_enum_consts_ref: None,
            method_enum_constants: HashMap::default(),
            class_scoped_enums: None,
            allow_waits: false,
            delegate_calls: false,
            wait_specs: Vec::new(),
            for_loop_var_ids: std::collections::HashMap::default(),
            local_var_regs: std::collections::HashMap::default(),
            method_mode: false,
            method_this_reg: None,
            method_handle_names: std::collections::HashSet::default(),
            method_handle_local_types: HashMap::default(),
            method_static_locals: Vec::new(),
            method_string_ret_names: HashSet::default(),
            pkg_enum_consts: HashMap::default(),
            class_shadow_names: HashSet::default(),
            bare_member_names: HashSet::default(),
            member_class_names: HashSet::default(),
            member_safe_names: HashSet::default(),
            class_local_names: HashSet::default(),
            class_method_names: HashSet::default(),
            super_method_names: HashSet::default(),
            method_class_parent: None,
            static_bare_targets: HashMap::default(),
            method_formal_widths: HashMap::default(),
            method_writeback_names: HashSet::default(),
            method_bound_names: HashSet::default(),
            method_shadowing_regs: HashSet::default(),
            static_member_names: HashSet::default(),
            class_const_names: HashSet::default(),
            new_type_map: HashMap::default(),
            scope_static_receivers: HashSet::default(),
            method_class: None,
            handle_member_types: None,
            method_admission: None,
            this_field_widths: HashMap::default(),
            this_field_signed: HashSet::default(),
            method_formal_types: None,
            method_return_type: None,
            method_result_reg: None,
            method_return_val_reg: None,
            method_result_width: None,
            method_result_is_class: false,
            method_result_is_string: false,
            method_ret_is_collection: false,
            free_function_mode: false,
            method_pkg: None,
            method_pkg_imports: None,
            inline_pkg_stack: Vec::new(),
            string_formal_names: HashSet::default(),
            string_member_names: HashSet::default(),
            coll_member_names: HashSet::default(),
            static_coll_member_names: HashSet::default(),
            coll_elem_member_names: HashSet::default(),
            coll_elem_packed_dims: HashMap::default(),
            coll_elem_widths: HashMap::default(),
            typedef_coll_dims: HashMap::default(),
            typedef_coll_strings: HashSet::default(),
            member_coll_strings: HashSet::default(),
            cast_reg_ok_locals: HashSet::default(),
            method_ret_jumps: Vec::new(),
            decl_local_regs: std::collections::HashSet::default(),
            process_local_names: HashSet::default(),
            local_kinds: HashMap::default(),
            inline_foreign: 0,
            reg_var_loop_depth: 0,
            loop_index_bounds: HashMap::default(),
            allow_expr_fallback: false,
            tasks: None,
            functions: None,
            function_scope_index: None,
            deferred_functions: None,
            deferred_tasks: None,
            deferred_packages: None,
            struct_call_fd: None,
            unpacked_structs: None,
            dpi_import_fds: None,
            inlining_stack: Vec::new(),
            tasks_inlined: 0,
            params: None,
            param_leaf_idx: None,
            typedefs: None,
            typedef_elems: None,
            typedef_types: None,
            local_typedef_types: HashMap::default(),
            unresolved_type_names: HashSet::default(),
            local_var_elem: std::collections::HashMap::new(),
            local_var_struct: HashMap::default(),
            local_packed_types: HashMap::default(),
            cast_widths: None,
            dotted_ident_sign: HashMap::default(),
            named_elem_depth: HashMap::default(),
            pattern_ctx: None,
            inline_ret_ty: None,
            local_const_vars: HashMap::default(),
            local_const_kinds: HashMap::default(),
            local_array_regs: HashMap::default(),
            local_coll_names: HashSet::default(),
            local_coll_strings: HashSet::default(),
            local_coll_elems: HashMap::default(),
            top_module_name: None,
            packed_elem_widths: None,
            assoc_elem_widths: None,
            assoc_arrays: None,
            packed_full_dims: None,
            var_decl_types: None,
            typedef_unpacked_dims: None,
            param_decl_types: None,
            port_aliases: None,
            loop_break_patches: Vec::new(),
            loop_continue_patches: Vec::new(),
            named_block_patches: HashMap::default(),
            foreach_slot_count: 0,
            string_signals: None,
            local_var_is_string: HashSet::default(),
            local_var_is_real: HashSet::default(),
            local_var_array: HashMap::default(),
            const_var_binds: HashMap::default(),
            purity_depth: std::cell::Cell::new(0),
            purity_cut: std::cell::Cell::new(false),
            inline_ret: None,
            inline_ret_real: false,
            inline_ret_jumps: Vec::new(),
            multi_dim_arrays: None,
            arrays_2d: None,
            dynamic_arrays: None,
            queue_vars: None,
            packed_struct_fields: None,
            events: None,
        }
    }

    /// Make the active process's innermost frame visible to a compiled,
    /// non-suspending statement. Each local is loaded once on block entry and
    /// then behaves like any other register-backed operand.
    pub fn set_process_locals(&mut self, locals: &HashMap<String, Value>) {
        for (name, value) in locals {
            let reg = self.alloc_reg();
            self.emit(Insn::LoadProcessLocal(reg, name.clone().into_boxed_str()));
            self.local_var_regs.insert(name.clone(), (reg, value.width));
            self.process_local_names.insert(name.clone());
        }
    }

    pub fn set_packed_struct_fields(
        &mut self,
        f: &'a xezim_core::shared_names::SharedSliceMap<(String, u32, u32)>,
    ) {
        self.packed_struct_fields = Some(f);
    }

    pub fn set_string_signals(&mut self, s: &'a HashSet<String>) {
        self.string_signals = Some(s);
    }

    /// Supply per-signal `is_real` so `elide_redundant_resizes` can prove a
    /// loaded signal is not real. `Value::add` and friends special-case a real
    /// operand (returning a 64-bit `from_f64`), so without this every
    /// arithmetic result on a signal is "possibly real" and its `Resize`
    /// survives — measured at ~337K of the ~553K resizes still executing.
    pub fn set_signal_real(&mut self, r: &'a [bool]) {
        self.signal_real = Some(r);
    }

    /// The runtime's 2-state marks, by signal id. A compiled range store
    /// into a container declines when its signal may be 2-state.
    pub fn set_signal_two_state(&mut self, t: &'a [bool]) {
        self.signal_two_state = Some(t);
    }

    pub fn set_multi_dim_arrays(&mut self, s: &'a HashSet<String>) {
        self.multi_dim_arrays = Some(s);
    }

    pub fn set_arrays_2d(&mut self, m: &'a HashMap<String, ((i64, i64), (i64, i64), u32)>) {
        self.arrays_2d = Some(m);
    }

    pub fn set_collection_denies(
        &mut self,
        dyn_arrays: &'a HashSet<String>,
        queues: &'a HashSet<String>,
    ) {
        self.dynamic_arrays = Some(dyn_arrays);
        self.queue_vars = Some(queues);
    }

    pub fn set_array_first_id(&mut self, arrays: &'a HashMap<Arc<str>, (usize, i64, i64)>) {
        self.array_first_id = Some(arrays);
    }

    pub fn set_params(&mut self, params: &'a HashMap<String, Value>) {
        self.params = Some(params);
    }

    pub fn set_param_leaf_idx(&mut self, idx: &'a HashMap<String, Vec<String>>) {
        self.param_leaf_idx = Some(idx);
    }

    pub fn set_packed_elem_widths(
        &mut self,
        w: &'a xezim_core::shared_names::SharedNameMap<u32>,
    ) {
        self.packed_elem_widths = Some(w);
    }

    pub fn set_typedefs(
        &mut self,
        widths: &'a HashMap<String, u32>,
        elems: &'a HashMap<String, u32>,
    ) {
        self.typedefs = Some(widths);
        self.typedef_elems = Some(elems);
    }

    pub fn set_unpacked_structs(&mut self, m: &'a UnpackedStructLeaves) {
        self.unpacked_structs = Some(m);
    }

    pub fn set_typedef_types(&mut self, types: &'a HashMap<String, crate::ast::types::DataType>) {
        self.typedef_types = Some(types);
    }

    pub fn set_events(&mut self, events: &'a HashSet<String>) {
        self.events = Some(events);
    }

    pub(super) fn is_event_dt(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::{DataType, SimpleType};
        let mut cur = dt;
        for _ in 0..16 {
            match cur {
                DataType::Simple {
                    kind: SimpleType::Event,
                    ..
                } => return true,
                DataType::TypeReference { name, .. } => {
                    let next = if name.scopes.is_empty() {
                        self.local_typedef_types
                            .get(&name.name.name)
                            .or_else(|| self.typedef_types.and_then(|t| t.get(&name.name.name)))
                    } else {
                        let q = name.qualified();
                        self.typedef_types
                            .and_then(|t| t.get(&q).or_else(|| t.get(&name.name.name)))
                    };
                    match next {
                        Some(n) => cur = n,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    pub(super) fn is_event_ident(&self, hier: &HierarchicalIdentifier) -> bool {
        let Some(events) = self.events else {
            return false;
        };
        if hier.path.is_empty() {
            return false;
        }
        if hier.path.len() == 1 {
            let bare = hier.path[0].name.name.as_str();
            if events.contains(bare) {
                return true;
            }
            if let Some(scope) = self.scope_hint.as_deref()
                && !scope.is_empty()
            {
                return with_dotted(scope, bare, |k| events.contains(k));
            }
            return false;
        }
        let raw = Self::hier_raw_cow(hier);
        if events.contains(raw.as_ref()) {
            return true;
        }
        if let Some(top) = self.top_module_name.as_deref()
            && let Some(rest) = raw
                .strip_prefix(top)
                .and_then(|r| r.strip_prefix('.'))
            && events.contains(rest)
        {
            return true;
        }
        if hier.path.len() >= 2 && hier.path[0].selects.is_empty() {
            let pkg_qual = format!("{}::{}", hier.path[0].name.name, hier.path[1].name.name);
            if events.contains(&pkg_qual) {
                return true;
            }
        }
        false
    }

    pub(super) fn is_event_expr(&self, e: &Expression) -> bool {
        let mut cur = e;
        while let ExprKind::Paren(inner) = &cur.kind {
            cur = inner;
        }
        match &cur.kind {
            ExprKind::Ident(h) => self.is_event_ident(h),
            ExprKind::Index { expr, .. } => self.is_event_expr(expr),
            _ => false,
        }
    }

    pub fn set_local_typedef_types(&mut self, types: HashMap<String, crate::ast::types::DataType>) {
        self.local_typedef_types = types;
    }

    pub fn set_method_formal_types(&mut self, ports: &'a [crate::ast::decl::FunctionPort]) {
        self.method_formal_types = Some(ports);
    }

    pub fn set_method_return_type(&mut self, dt: &'a crate::ast::types::DataType) {
        self.method_return_type = Some(dt);
    }

    pub fn set_unresolved_type_names(&mut self, names: HashSet<String>) {
        self.unresolved_type_names = names;
    }

    pub fn set_assoc_elem_widths(&mut self, w: &'a HashMap<String, u32>) {
        self.assoc_elem_widths = Some(w);
    }

    pub fn set_assoc_arrays(&mut self, a: &'a HashMap<String, bool>) {
        self.assoc_arrays = Some(a);
    }

    pub fn set_packed_full_dims(
        &mut self,
        d: &'a xezim_core::shared_names::SharedSliceMap<(i64, i64)>,
    ) {
        self.packed_full_dims = Some(d);
    }

    pub fn set_var_decl_types(&mut self, t: &'a xezim_core::decl_types::DeclTypeMap) {
        self.var_decl_types = Some(t);
    }

    pub fn set_port_aliases(
        &mut self,
        a: &'a xezim_core::shared_names::SharedNameMap<xezim_core::shared_names::SharedName>,
    ) {
        self.port_aliases = Some(a);
    }

    pub fn set_param_decl_types(&mut self, t: &'a xezim_core::decl_types::ParamDeclTypeMap) {
        self.param_decl_types = Some(t);
    }

    pub fn set_typedef_unpacked_dims(
        &mut self,
        d: &'a HashMap<String, Vec<crate::ast::types::UnpackedDimension>>,
    ) {
        self.typedef_unpacked_dims = Some(d);
    }

    pub fn set_ast_fallback(&mut self, allow: bool) {
        self.allow_ast_fallback = allow;
    }

    pub fn set_expr_fallback(&mut self, allow: bool) {
        self.allow_expr_fallback = allow;
    }

    pub fn set_scope_hint(&mut self, scope: Option<String>) {
        self.dotted_ident_sign.clear();
        self.named_elem_depth.clear();
        self.scope_hint = scope;
    }

    pub fn set_pkg_enum_consts(&mut self, m: &'a HashMap<String, (u64, u32)>) {
        self.pkg_enum_consts_ref = Some(m);
    }

    pub fn set_cast_widths(&mut self, m: &'a HashMap<String, (u32, bool)>) {
        self.cast_widths = Some(m);
    }

    pub fn set_functions(&mut self, functions: &'a HashMap<String, FunctionDeclaration>) {
        self.functions = Some(functions);
        self.function_scope_index = None;
    }

    /// Bind the index and its declaration table together. Replacing the
    /// table through `set_functions` clears this borrowed index.
    pub(crate) fn set_function_scope_index(&mut self, index: &'a FunctionScopeIndex<'a>) {
        self.set_functions(index.functions);
        self.function_scope_index = Some(index);
    }

    /// DPI import prototypes by SV-visible name. A compiled body may call an
    /// import directly (e.g. `uvm_re_compexecfree` under `uvm_re_match`);
    /// imports are not in `functions`, so the pure-call planner needs their
    /// ports to shape `CallFreeFunction` emission.
    pub fn set_dpi_import_fds(
        &mut self,
        m: std::rc::Rc<HashMap<String, crate::ast::decl::FunctionDeclaration>>,
    ) {
        self.dpi_import_fds = Some(m);
    }

    pub fn set_tasks(&mut self, tasks: &'a HashMap<String, TaskDeclaration>) {
        self.tasks = Some(tasks);
    }

    pub fn set_deferred_callables(
        &mut self,
        functions: &'a HashMap<String, std::rc::Rc<super::elaborate::CallableRewriteCtx>>,
        tasks: &'a HashMap<String, std::rc::Rc<super::elaborate::CallableRewriteCtx>>,
        packages: &'a HashSet<String>,
    ) {
        self.deferred_functions = Some(functions);
        self.deferred_tasks = Some(tasks);
        self.deferred_packages = Some(packages);
    }

    fn flatten_deferred_items(
        &self,
        mut items: Vec<Statement>,
    ) -> crate::ast::shared_vec::SharedVec<Statement> {
        if let Some(packages) = self.deferred_packages
            && !packages.is_empty()
        {
            let pkgs: std::collections::HashSet<String> = packages.iter().cloned().collect();
            items = items
                .iter()
                .map(|s| super::elaborate::flatten_pkg_member_refs(s, &pkgs))
                .collect();
        }
        items.into()
    }

    fn materialize_deferred_fn(
        &self,
        name: &str,
        fd: &FunctionDeclaration,
    ) -> Option<crate::ast::shared_vec::SharedVec<Statement>> {
        let dctx = self.deferred_functions?.get(name)?;
        Some(crate::ast::cache::without_scope_caches(|| {
            let items = dctx.materialize_body(&fd.ports, &fd.items);
            self.flatten_deferred_items(items)
        }))
    }

    fn materialize_deferred_task(
        &self,
        name: &str,
        td: &TaskDeclaration,
    ) -> Option<crate::ast::shared_vec::SharedVec<Statement>> {
        let dctx = self.deferred_tasks?.get(name)?;
        Some(crate::ast::cache::without_scope_caches(|| {
            let items = dctx.materialize_body(&td.ports, &td.items);
            self.flatten_deferred_items(items)
        }))
    }

    fn bail(&mut self, reason: &'static str) {
        if self.bail_reason.is_none() {
            self.bail_reason = Some(reason);
        }
    }

    fn alloc_reg(&mut self) -> RegId {
        let Ok(r) = RegId::try_from(self.next_reg) else {
            self.register_overflow = true;
            return 0;
        };
        self.next_reg += 1;
        // A rolled-back compile rewinds `next_reg`, so this id may have held
        // a typed local before: whoever binds a local here re-types it.
        if !self.local_kinds.is_empty() {
            self.local_kinds.remove(&r);
        }
        r
    }

    fn emit(&mut self, insn: Insn) {
        self.insns.push(insn);
    }

    /// Discard speculative instructions together with their pending jump
    /// fixups. A return, break or continue emitted by a failed lowering must
    /// not patch discarded code when an enclosing function or loop finishes.
    fn truncate_insns(&mut self, len: usize) {
        self.insns.truncate(len);
        self.inline_ret_jumps.retain(|&pc| pc < len);
        self.method_ret_jumps.retain(|&pc| pc < len);
        for patches in &mut self.loop_break_patches {
            patches.retain(|&pc| pc < len);
        }
        for patches in &mut self.loop_continue_patches {
            patches.retain(|&pc| pc < len);
        }
        for patches in self.named_block_patches.values_mut() {
            patches.retain(|&pc| pc < len);
        }
    }

    /// `hier_raw_name`, borrowed when the path has a single segment.
    fn hier_raw_cow(hier: &HierarchicalIdentifier) -> std::borrow::Cow<'_, str> {
        match hier.path.as_slice() {
            [one] => std::borrow::Cow::Borrowed(one.name.name.as_str()),
            _ => std::borrow::Cow::Owned(Self::hier_raw_name(hier)),
        }
    }

    fn hier_raw_name(hier: &HierarchicalIdentifier) -> String {
        match hier.path.as_slice() {
            [one] => one.name.name.clone(),
            path => {
                let len = path.iter().map(|s| s.name.name.len() + 1).sum::<usize>();
                let mut out = String::with_capacity(len.saturating_sub(1));
                for (i, seg) in path.iter().enumerate() {
                    if i > 0 {
                        out.push('.');
                    }
                    out.push_str(&seg.name.name);
                }
                out
            }
        }
    }

    /// §7.4.1 physical bit position of a declared label: a descending range
    /// counts up from its low bound, an ascending one mirrors. Selects on
    /// such bases used to go to the AST interpreter for want of this map.
    fn label_to_phys(dim: (i64, i64), label: i64) -> i64 {
        let (lo_b, hi_b) = (dim.0.min(dim.1), dim.0.max(dim.1));
        if dim.0 >= dim.1 {
            label - lo_b
        } else {
            hi_b - label
        }
    }

    /// Compile a standalone expression and return the register containing its
    /// result. Used by scheduler fast paths that repeatedly evaluate the same
    /// delay expression outside an always-block body.
    pub fn compile_root_expr(&mut self, expr: &Expression) -> Option<RegId> {
        let result = self.compile_expr(expr, 0);
        if self.register_overflow {
            self.bail("bytecode_register_limit");
            None
        } else {
            // OBSERVABLE RESULT MARKER (see `compile_class_method`): the
            // runtime reads the result register after `exec_insns`, so
            // nothing in the block reads it and `finish`'s peepholes took
            // it for dead. A constant `#(1*2.0)` — the shape every bare
            // delay takes under a `timescale whose unit is not the tick —
            // folded to a `LoadConst` that was then dropped, and the
            // delay read a stale register (0: a zero-delay loop).
            if let Some(r) = result {
                self.emit(Insn::Move(r, r));
            }
            result
        }
    }

    pub fn finish(&mut self) -> CompiledBlock {
        debug_assert!(!self.register_overflow);
        Self::fuse_load_selects(&mut self.insns);
        // After fusion (so the fused `LoadSignalRange`/`LoadSignalBit` count as
        // width-defining) and before `compact_nops` (which removes the `Nop`s
        // this pass leaves behind).
        let signal_widths = self.signal_widths;
        let num_regs = (self.next_reg as usize).min(u16::MAX as usize + 1);
        Self::elide_redundant_resizes(&mut self.insns, signal_widths, self.signal_real, num_regs);
        Self::fold_fill_const_resize(&mut self.insns);
        Self::forward_copies(&mut self.insns);
        Self::propagate_copies(&mut self.insns);
        // AFTER resize elision: an about-to-be-deleted `Resize` sitting between
        // the array read and the NBA would otherwise hide the triple. Still
        // before `compact_nops`, which removes the `Nop`s it leaves behind.
        Self::fuse_array_read_nba(&mut self.insns);
        // Also after `elide_redundant_resizes`: a `Resize` that pass deletes
        // is what most often separates a `LoadConst` from its consumer. Its
        // pattern is disjoint from `fuse_array_read_nba`'s, so the order
        // between the two does not matter.
        Self::fuse_binop_const(&mut self.insns);
        {
            use std::sync::OnceLock;
            static FOLD_STORES: OnceLock<u32> = OnceLock::new();
            let m = *FOLD_STORES.get_or_init(|| {
                std::env::var("XEZIM_FOLD_STORES")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(FOLD_STORES_PERSISTED)
            });
            Self::fold_const_regs_masked(&mut self.insns, signal_widths, m);
        }
        // Folding turns `Mul(t, K0, K8)` into a constant the pass above had
        // already walked past; a second pass fuses those too.
        Self::fuse_binop_const(&mut self.insns);
        // After fuse_binop_const, before fuse_cmp_branch_move_resize: its
        // Move;Resize pattern is disjoint from the Move;Assign one here.
        Self::forward_move_into_assign(&mut self.insns);
        Self::fuse_cmp_branch_move_resize(&mut self.insns);
        // Last, so it sees the final insn stream (fusions above both create
        // and absorb sign scrubs).
        Self::elide_provably_unsigned_scrubs(&mut self.insns, self.signal_signed);
        Self::fuse_addc2(&mut self.insns);
        Self::compact_nops(&mut self.insns);
        // Trim unused capacity. `Vec::push` doubles the backing buffer
        // when it overflows, so a freshly compiled block can sit on
        // up to ~50% slack capacity. With ~100K CompiledBlocks on
        // c910, that slack stacks into double-digit MB; one
        // `shrink_to_fit` per finish reclaims it.
        self.insns.shrink_to_fit();
        let has_fallback = self
            .insns
            .iter()
            .any(|i| matches!(i, Insn::StmtFallback(..)));
        // Any signal written nonblockingly twice — counting the partial forms,
        // since `v[3:0] <= ..; v <= ..` is the same hazard as two whole writes.
        let mut nba_targets: Vec<u32> = Vec::new();
        for i in &self.insns {
            let id = match i {
                Insn::NbaAssign(id, _, _)
                | Insn::NbaAssignConst(id, _, _)
                | Insn::NbaAssignRange(id, _, _, _)
                | Insn::NbaAssignRangeDyn(id, _, _, _)
                | Insn::NbaAssignArrayRead(id, _, _, _)
                | Insn::NbaAssignBitDyn(id, _, _) => Some(*id as u32),
                _ => None,
            };
            if let Some(id) = id {
                nba_targets.push(id);
            }
        }
        nba_targets.sort_unstable();
        // Array-element NBAs resolve their target id at RUN time (dynamic
        // index), so two of them — or one plus a scalar NBA that a constant
        // index folded to an element id — can collide without appearing in
        // `nba_targets`. Any array NBA alongside another NBA marks the block
        // conservatively; the scan it enables is a short rposition over the
        // block's own queue, and single-NBA blocks (the overwhelming
        // majority) still take the plain push path.
        let array_nbas = self
            .insns
            .iter()
            .filter(|i| matches!(i, Insn::NbaAssignArray(..) | Insn::NbaAssignArrayRange(..)))
            .count();
        let total_nbas = nba_targets.len() + array_nbas;
        let nba_dup_targets =
            nba_targets.windows(2).any(|w| w[0] == w[1]) || (array_nbas >= 1 && total_nbas >= 2);
        CompiledBlock {
            num_regs: self.next_reg,
            instructions: std::mem::take(&mut self.insns),
            has_fallback,
            nba_dup_targets,
            foreach_slots: self.foreach_slot_count,
            static_locals: std::mem::take(&mut self.method_static_locals).into_boxed_slice(),
        }
    }

    /// Fallback bail reset for the all-or-nothing method compile path.
    fn bail_reset(&mut self) {
        self.bail_reason = None;
        self.register_overflow = false;
    }
}

/// §20.5 / §20.8: the system functions whose result is REAL — the
/// conversions to real, `$realtime`, and the real math functions.
pub(crate) fn system_function_is_real(name: &str) -> bool {
    matches!(
        name,
        "$itor"
            | "$bitstoreal"
            | "$bitstoshortreal"
            | "$realtime"
            | "$__xz_realtime_at"
    ) || real_math_function(name)
}

fn stepped_math_function(name: &str) -> bool {
    name == "$rtoi" || real_math_function(name)
}

fn real_math_function(name: &str) -> bool {
    matches!(name, "$ln"
            | "$log10"
            | "$exp"
            | "$sqrt"
            | "$pow"
            | "$floor"
            | "$ceil"
            | "$sin"
            | "$cos"
            | "$tan"
            | "$asin"
            | "$acos"
            | "$atan"
            | "$atan2"
            | "$hypot"
            | "$sinh"
            | "$cosh"
            | "$tanh"
            | "$asinh"
            | "$acosh"
            | "$atanh"
    )
}

/// Whether a member/select chain is rooted at a call (`mk().f0[1]`).
fn chain_has_call_base(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => {
            matches!(expr.kind, ExprKind::Call { .. }) || chain_has_call_base(expr)
        }
        _ => false,
    }
}

/// IEEE 1800-2017 §20/§21 result type of a system FUNCTION as (width,
/// signed): `int`/`integer` → (32, true); unsigned int → (32, false);
/// `bit` → (1, false); `time` / `$realtobits` → (64, false). Real- and
/// string-valued functions, and the ones that carry their argument's type
/// (`system_function_carries_arg`), are None. Shared by the compiler's width
/// and signedness inference and by the interpreter, so every path agrees.
/// The declared type of variable `name`, or of member `m` of a structure
/// variable `v` for `name` = `v.m` (§7.2: a member keeps its declared type).
pub(crate) fn declared_type_of<'a>(
    name: &str,
    decls: &'a xezim_core::decl_types::DeclTypeMap,
    types: &'a HashMap<String, crate::ast::types::DataType>,
) -> Option<&'a crate::ast::types::DataType> {
    if let Some(dt) = decls.get(name) {
        return Some(dt);
    }
    let (var, member) = name.rsplit_once('.')?;
    let crate::ast::types::DataType::Struct(su) =
        crate::compiler::elaborate::resolve_typedef_chain(decls.get(var)?, types)
    else {
        return None;
    };
    su.members
        .iter()
        .find(|m| m.declarators.iter().any(|d| d.name.name == member))
        .map(|m| &m.data_type)
}

/// §7.4.1: the number of selects at which a packed array declared as
/// `T [d1]...[dn] x` (directly, or through typedefs) reaches its NAMED
/// element type `T`, when `T` is a signed integral type. Such an element
/// select is signed; every other select of a packed array is unsigned
/// ("the individual elements of the array are unsigned unless they are of a
/// named type declared as signed"). None for any other declaration.
pub(crate) fn signed_named_packed_elem_depth(
    dt: &crate::ast::types::DataType,
    types: &HashMap<String, crate::ast::types::DataType>,
) -> Option<usize> {
    use crate::ast::types::DataType as D;
    /// Through plain aliases (`typedef T U;`) to the defining type.
    fn follow<'a>(mut cur: &'a D, types: &'a HashMap<String, D>) -> Option<&'a D> {
        for _ in 0..64 {
            match cur {
                D::TypeReference {
                    name, dimensions, ..
                } if dimensions.is_empty() => {
                    cur = crate::compiler::elaborate::lookup_scoped_type(name, types)?;
                }
                _ => return Some(cur),
            }
        }
        None
    }
    let mut cur = follow(dt, types)?;
    let mut depth = 0usize;
    for _ in 0..64 {
        let D::TypeReference {
            name, dimensions, ..
        } = cur
        else {
            return None;
        };
        depth += dimensions.len();
        let elem = follow(
            crate::compiler::elaborate::lookup_scoped_type(name, types)?,
            types,
        )?;
        match elem {
            // The element is itself a packed array of a named type: an
            // element of THAT array is one select deeper.
            D::TypeReference { .. } => cur = elem,
            D::IntegerAtom { .. } | D::IntegerVector { .. } | D::Enum(_) => {
                return crate::compiler::elaborate::is_type_signed_resolved(elem, types)
                    .then_some(depth);
            }
            D::Struct(su) if su.packed => {
                return crate::compiler::elaborate::is_type_signed_resolved(elem, types)
                    .then_some(depth);
            }
            _ => return None,
        }
    }
    None
}

pub(crate) fn system_function_result(name: &str) -> Option<(u32, bool)> {
    Some(match name {
        "$countones"
        | "$countbits"
        | "$clog2"
        | "$bits"
        | "$size"
        | "$dimensions"
        | "$unpacked_dimensions"
        | "$left"
        | "$right"
        | "$low"
        | "$high"
        | "$increment"
        | "$rtoi"
        | "$random"
        | "$cast"
        | "$fopen"
        | "$fgetc"
        | "$fgets"
        | "$fscanf"
        | "$sscanf"
        | "$fread"
        | "$ftell"
        | "$feof"
        | "$ferror"
        | "$ungetc"
        | "$fseek"
        | "$rewind"
        | "$test$plusargs"
        | "$value$plusargs"
        | "$dist_uniform"
        | "$dist_normal"
        | "$dist_exponential"
        | "$dist_poisson"
        | "$dist_chi_square"
        | "$dist_t"
        | "$dist_erlang"
        | "$coverage_control"
        | "$coverage_get_max"
        | "$coverage_get"
        | "$coverage_merge"
        | "$coverage_save" => (32, true),
        "$urandom" | "$urandom_range" | "$stime" | "$__xz_stime_at" | "$shortrealtobits" => {
            (32, false)
        }
        "$onehot" | "$onehot0" | "$isunknown" | "$isunbounded" | "$rose" | "$fell" | "$stable"
        | "$changed" => (1, false),
        "$time" | "$__xz_time_at" | "$realtobits" => (64, false),
        _ => return None,
    })
}

/// System functions whose result has the TYPE of their first argument.
pub(crate) fn system_function_carries_arg(name: &str) -> bool {
    matches!(name, "$signed" | "$unsigned" | "$past" | "$sampled")
}

/// Source spelling of a type name, `::`-joined (`p::q_t`), the key of
/// `BytecodeCompiler::typedef_coll_dims`.
pub(crate) fn type_name_key(tn: &crate::ast::types::TypeName) -> String {
    let mut k = String::new();
    for sc in &tn.scopes {
        k.push_str(&sc.name.name);
        k.push_str("::");
    }
    k.push_str(&tn.name.name);
    k
}

/// A shift whose type can be changed by a surrounding integral expression.
/// Self-determined operands (casts, concatenations, calls and reductions)
/// stop propagation; a conditional's condition does not determine its type.
pub(crate) fn has_context_shift(e: &Expression) -> bool {
    match &e.kind {
        ExprKind::Binary {
            op:
                BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight,
            ..
        } => true,
        ExprKind::Binary {
            op:
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor,
            left,
            right,
        } => has_context_shift(left) || has_context_shift(right),
        ExprKind::Binary {
            op: BinaryOp::Power,
            left,
            ..
        } => has_context_shift(left),
        ExprKind::Unary {
            op: UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot,
            operand,
        }
        | ExprKind::Paren(operand) => has_context_shift(operand),
        ExprKind::Conditional {
            then_expr,
            else_expr,
            ..
        } => has_context_shift(then_expr) || has_context_shift(else_expr),
        _ => false,
    }
}

/// The compiler's type tables, as a pattern plan resolves them.
struct BcPatternTypeResolver<'c, 'a>(&'c mut BytecodeCompiler<'a>);

impl crate::compiler::packed_assignment_pattern_plan::PatternTypeResolver
    for BcPatternTypeResolver<'_, '_>
{
    fn typedef_target(
        &mut self,
        name: &crate::ast::types::TypeName,
    ) -> Option<crate::ast::types::DataType> {
        // A typedef with unpacked dimensions is not packed (§7.4).
        let (key, t) = self.0.typedef_type_entry(name, None)?;
        if self
            .0
            .typedef_unpacked_dims
            .is_some_and(|d| d.get(key).is_some_and(|v| !v.is_empty()))
        {
            return None;
        }
        Some(t.clone())
    }

    fn const_i64(&mut self, e: &Expression) -> Option<i64> {
        self.0.fold_const_i64(e)
    }

    fn pattern_index(&mut self, e: &Expression, bounds: (i64, i64)) -> Option<i64> {
        xezim_core::elaborate::const_eval_typed_pattern_index(e, self.0.params, bounds)
            .or_else(|| self.0.fold_const_i64(e))
    }

    fn is_type_name(&mut self, name: &str) -> bool {
        if self.0.params.is_some_and(|p| p.contains_key(name)) {
            return false;
        }
        let tn = crate::compiler::packed_assignment_pattern_plan::parse_type_name_str(name);
        self.0.typedef_type_entry(&tn, None).is_some()
    }

    fn expr_type(&mut self, e: &Expression) -> Option<crate::ast::types::DataType> {
        use crate::compiler::packed_assignment_pattern_plan as pp;
        if let Some(t) = pp::named_type_of(e) {
            return Some(t);
        }
        if let Some(dt) =
            pp::scoped_name_key(e).and_then(|k| self.0.param_decl_types?.get(&k).cloned())
        {
            return Some(dt);
        }
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let raw = BytecodeCompiler::hier_raw_name(h);
        let scoped = self.0.scope_hint.as_deref().map(|sc| cat_dot(sc, &raw));
        let found = self
            .0
            .var_decl_types
            .and_then(|types| {
                [scoped.as_deref(), Some(raw.as_str())]
                    .into_iter()
                    .flatten()
                    .find_map(|n| types.get(n).cloned())
            })
            .or_else(|| {
                self.0.param_decl_types.and_then(|types| {
                    [scoped.as_deref(), Some(raw.as_str())]
                        .into_iter()
                        .flatten()
                        .find_map(|n| types.get(n).cloned())
                })
            });
        if found.is_some() {
            return found;
        }
        // A package parameter imported bare is keyed `pkg::P` (§26.3).
        if h.path.len() == 1 && self.0.params.is_some_and(|p| p.contains_key(raw.as_str())) {
            let mut hit = None;
            for (k, dt) in self.0.param_decl_types? {
                if k.rsplit_once("::").is_some_and(|(_, n)| n == raw) {
                    if hit.is_some() {
                        return None;
                    }
                    hit = Some(dt.clone());
                }
            }
            return hit;
        }
        None
    }
}
