//! Class instances, property maps, class aggregate references and specialization helpers.

use super::*;

/// A class object's property map. Reads deref to the plain map; every
/// mutating entry point notes the property name in `STORE_WRITE_MASK`, so no
/// write site can be missed by the condition-waiter gate.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct PropMap(pub(super) HashMap<String, Value>);

impl PropMap {
    #[inline]
    pub(super) fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        note_store_write(name_bit(&k));
        self.0.insert(k, v)
    }
    #[inline]
    pub(super) fn get_mut(&mut self, k: &str) -> Option<&mut Value> {
        note_store_write(name_bit(k));
        self.0.get_mut(k)
    }
    #[inline]
    pub(super) fn remove(&mut self, k: &str) -> Option<Value> {
        note_store_write(name_bit(k));
        self.0.remove(k)
    }
}

/// Class static properties (`Class::prop` keys), noted like `PropMap`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct StaticMap(HashMap<String, Value>);

impl StaticMap {
    #[inline]
    pub(super) fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        note_store_write(name_bit(k.rsplit("::").next().unwrap_or(&k)));
        self.0.insert(k, v)
    }
}

/// §8.25/§6.20.2 (issue #423): the materialized elements of one class-scope
/// ARRAY constant of one specialization — a header VALUE parameter declared
/// with unpacked dimensions (`class C #(int unsigned TBL [4] = '{...})`,
/// read `C#(..)::TBL[i]`) or a class-body `localparam` array (`LP[i]`).
/// Lazily built on first element read by
/// `Simulator::materialize_class_param_array`; `lo`/`hi` preserve bounds;
/// `default` carries the declared element type for invalid/unknown indices.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ClassParamArray {
    pub(super) lo: i64,
    pub(super) hi: i64,
    pub(super) descending: bool,
    pub(super) default: Value,
    pub(super) elems: Vec<Value>,
}

/// §8.3.2/§6.20.2 (issue #388): a class-scope ARRAY constant resolved for a
/// WHOLE-COLLECTION context — `$size`, `inside`, `foreach`, whole-array copy —
/// spelled `pkg::Cls::T`, `Cls::T` from a foreign class, or bare `T` inside a
/// method of the declaring class. Holds the bounds plus the element source:
/// `param_cell` constants (class-body `localparam` arrays, header value
/// parameters with unpacked dims) read the materialized `class_param_arrays`
/// cell named by `key`; a static fixed-array property reads its per-element
/// signals `Cls::name[i]` (also named by `key`).
pub(super) struct ClassScopeArrayConst {
    pub(super) key: String,
    /// Normalized bounds (min, max) — the indexing domain.
    pub(super) lo: i64,
    pub(super) hi: i64,
    /// Declared bounds in source order (`[5:4]` gives (5, 4)) — drives
    /// `$left`/`$right`/`$increment` and the `foreach` visit order. Falls
    /// back to the normalized pair when no declared-dimension row exists.
    pub(super) left: i64,
    pub(super) right: i64,
    pub(super) param_cell: bool,
}

impl std::ops::Deref for StaticMap {
    type Target = HashMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::Deref for PropMap {
    type Target = HashMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a PropMap {
    type Item = (&'a String, &'a Value);
    type IntoIter = <&'a HashMap<String, Value> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Property -> (class-table key, definition) of the first class on a class's
/// `extends` chain that declares it (see `Simulator::prop_owners`).
pub(super) type PropOwners = HashMap<String, (String, Arc<super::elaborate::ElaboratedClass>)>;

/// Property names some class declares with a given property kind; a name in
/// none of them skips the per-class checks for that kind.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ClassMemberNames {
    pub(super) statics: HashSet<String>,
    /// Cheap prefilter for selected static-property accesses.
    #[serde(with = "serde_big_array")]
    pub(super) static_first: [bool; 256],
    pub(super) vif_props: HashSet<String>,
    /// First bytes of the `vif_props` names: a cheap prefilter for the
    /// member-chain checks that ask whether a name can be a vif property.
    #[serde(with = "serde_big_array")]
    pub(super) vif_first: [bool; 256],
    /// Every declared method name, of any kind.
    pub(super) methods: HashSet<String>,
    /// Every declared task name.
    pub(super) tasks: HashSet<String>,
    /// Every property declared as a packed vector (`bit [3:0][7:0] p`).
    pub(super) packed_vec_props: HashSet<String>,
    /// Every method name some class declares as a function returning
    /// `string`.
    pub(super) string_methods: HashSet<String>,
    pub(super) static_methods: HashSet<String>,
    /// Every function/task key and each of its suffixes after a `.`.
    pub(super) subroutine_suffixes: HashSet<String>,
    /// Typedef names (module or class scope) whose target a specialization
    /// key spells by type (`typedef_alias_frag`): a virtual interface, a
    /// one-keyword built-in, or a plain alias of another name.
    pub(super) alias_typedefs: HashSet<String>,
    /// Typedef names (module or class scope) whose target is a class-scoped
    /// type (`typedef R::elem_t et_t;`).
    pub(super) scoped_typedefs: HashSet<String>,
    /// Class typedef names whose target is a virtual interface (§25.9).
    pub(super) vif_typedef_leaves: HashSet<String>,
    /// Names that may prefix a class-scope reference without being a
    /// class: class parameters and typedef names.
    pub(super) scope_prefixes: HashSet<String>,
    /// First bytes of the `params` names: a cheap prefilter.
    #[serde(with = "serde_big_array")]
    pub(super) param_first: [bool; 256],
    /// Every class type/value parameter name.
    pub(super) params: HashSet<String>,
    /// Every associative-array property name.
    pub(super) assoc_props: HashSet<String>,
    /// Every property some class records a multidimensional collection
    /// shape for (`prop_coll_shapes`): a prefilter for the declared-shape
    /// lookup, which a one-dimensional property never needs.
    pub(super) shaped_props: HashSet<String>,
    /// Every `string` property name.
    pub(super) string_props: HashSet<String>,
    /// Every static property and class `localparam` name: the only names
    /// `static_prop_key` can resolve.
    pub(super) static_or_param: HashSet<String>,
    /// Names of class constant arrays and fixed static arrays.
    pub(super) constant_arrays: HashSet<String>,
    /// Names `plain_ident_read` leaves to the full path: static, vif and
    /// struct-capable properties, and the UVM activity constants.
    pub(super) ident_slow: HashSet<String>,
    /// Derived from `ident_slow` on first use, including after checkpoint restore.
    #[serde(skip)]
    pub(super) ident_slow_first: std::cell::OnceCell<[bool; 256]>,
}

/// A foreach/for index variable's outer state, restored when the loop exits:
/// (name, value, lived in a frame, typedef binding, trusted mark, packed
/// struct layout, (width, signed), shadowed vif frame alias).
pub(super) type LoopVarSnapshot = (
    String,
    Option<Value>,
    bool,
    Option<String>,
    bool,
    Option<Vec<(String, u32, u32)>>,
    (Option<u32>, bool),
    Option<String>,
);

/// `instance_assoc_member`'s class-only verdict for a bare name inside a
/// method of a given runtime class (class tables are fixed at run time).
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) enum MemberCollKind {
    /// A collection member: per-instance (`None`) or a static fixed array of
    /// the named owner class.
    Coll(Option<String>),
    /// A static collection somewhere in the chain.
    StaticColl,
    /// Neither: a per-instance collection only through a declared type that
    /// carries a collection dimension. `raws` are the raw declared type names
    /// met on the chain walk (a type-parameter binding for one of them
    /// changes the answer); `unbound` is the verdict with no binding applied.
    Plain { raws: Vec<String>, unbound: bool },
}

pub(super) type PackedAggregateLayout = (Vec<(String, u32, u32)>, u32);

#[derive(Debug)]
pub(super) struct ClassPackedLayouts {
    // Keep the source alive for the lifetime of the pointer-keyed cache.
    pub(super) _declaration: Rc<crate::ast::types::StructUnionType>,
    pub(super) variants: Vec<(Vec<(String, Value)>, Rc<PackedAggregateLayout>)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ClassInstance {
    pub(super) class_name: String,
    pub(super) properties: PropMap,
    /// Maps a class TYPE-parameter name (e.g. `T` in
    /// `class Mk #(type T=Base)`) to the concrete class name it was
    /// specialized with (e.g. `Base`). Populated by
    /// `instantiate_class_with_type_args`. Used so an unqualified `obj = new()`
    /// unqualified `obj = new()` whose declared type is a type parameter
    /// constructs the bound concrete class (running its real `new`). Empty
    /// for classes without type parameters / non-specialized instances.
    pub(super) type_bindings: HashMap<String, String>,
    /// The active specialization (`(base_class, sig)`) the instance was
    /// constructed under, captured so a later VIRTUAL method call on the
    /// instance can restore it (current_spec is otherwise lost once the
    /// constructing static call returns). Used by the dispatch override in
    /// `exec_method_in_class_hierarchy`. `None` for placeholder/stub
    /// instances and unspecialized constructions.
    pub(super) spec: Option<(String, String)>,
    /// §23.8 / §23.10.1: the INSTANCE scope whose process ran `new()`.
    /// Hierarchical names in this object's methods resolve upward from
    /// here, not from the caller's scope — a checker class constructed
    /// inside a bound module must see its binder's host even when the
    /// method is invoked from the top level (e.g. through a handle held
    /// in an associative array). Empty for objects built at top scope.
    pub(super) creation_scope: String,
}

/// A memoized `method_defining_class` answer: the defining class, the
/// method, its inherited string property names, and its call plan once built
/// (see `MethodCallPlan`).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct MethodDef {
    pub(super) class: std::rc::Rc<String>,
    pub(super) method: Arc<crate::ast::decl::ClassMethod>,
    pub(super) string_props: std::rc::Rc<Vec<String>>,
    pub(super) plan: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<MethodCallPlan>>>>,
}

/// Per leaf class: the facts of its `extends` chain that instantiation used
/// to re-derive on every `new` (see `Simulator::inst_template`). The class
/// tables are fixed at run time; the template also records the chain it was
/// built from and is only used for that same chain (compared by pointer).
/// `InstTemplate::seed` flag: seed the property with x (`prop_seeds_x`).
pub(super) const SEED_X: u8 = 4;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct InstTemplate {
    pub(super) chain: Vec<Arc<crate::compiler::elaborate::ElaboratedClass>>,
    /// Pre-seeded scalar/handle property map for the chain (cloned in one
    /// allocation per `new()` instead of inserting property-by-property).
    pub(super) base_props: HashMap<String, Value>,
    /// Per-class keys into `base_props` when a descendant property hides an
    /// ancestor parameter. Such chains need property/parameter interleaving.
    pub(super) property_seed_keys: Option<Vec<Vec<String>>>,
    /// Property names redeclared along the chain (`seed & 3 == 2`).
    pub(super) shadowed_props: Vec<String>,
    /// Whether any class in `chain` has `param_defaults` (value parameters).
    pub(super) has_value_params: bool,
    /// The `(chain index, property)` pairs, in the registration loop's
    /// order, that the type-parameter-bound collection registration can
    /// claim: a declared type name and no queue, associative, fixed,
    /// multi-dimensional or static collection property of the class.
    pub(super) coll_candidates: Vec<(usize, String)>,
    /// §7.2.2 member defaults of the chain's UNPACKED-struct properties
    /// (`dflt_t d;` with `typedef struct { int a = 3; } dflt_t`): the cell
    /// key `<prop>.<member>`, the default expression and the member's type,
    /// root class first so a derived class's property wins.
    pub(super) struct_defaults: Vec<(String, Expression, DataType)>,
    /// §6.8 / §7.2: the initial member cells of the chain's UNPACKED-struct
    /// properties (`st_t st;`), keyed `<prop>.<member>`: x for a 4-state
    /// member, 0 for a 2-state one. `struct_defaults` applies on top.
    pub(super) struct_cells: Vec<(String, Value)>,
}

/// Width of a spec fragment naming a DIMENSIONED vector type —
/// `bit[7:0]`, `logic signed[3:0][7:0]` — as `data_type_to_spec_fragment`
/// renders it for a `P#(bit [7:0])` specialization. Keyword-only fragments
/// are `atom_type_keyword_width`'s job; anything else declines.
pub(super) fn vector_fragment_width(frag: &str) -> Option<u32> {
    let rest = frag
        .strip_prefix("bit")
        .or_else(|| frag.strip_prefix("logic"))
        .or_else(|| frag.strip_prefix("reg"))?;
    let rest = rest
        .trim_start()
        .strip_prefix("unsigned")
        .or_else(|| rest.trim_start().strip_prefix("signed"))
        .unwrap_or(rest)
        .trim();
    if !rest.starts_with('[') {
        return None;
    }
    let mut width: u64 = 1;
    for dim in rest.split(']').filter(|d| !d.trim().is_empty()) {
        let inner = dim.trim().strip_prefix('[')?;
        let (l, r) = inner.split_once(':')?;
        let (l, r) = (l.trim().parse::<i64>().ok()?, r.trim().parse::<i64>().ok()?);
        width = width.checked_mul((l - r).unsigned_abs() + 1)?;
    }
    u32::try_from(width).ok()
}

/// Parse `<base>[<idx>]` and resolve via the compact 1D-array map.
/// Free function so it can be shared between the `&self` resolver on
/// Simulator and the `&[Value]`-borrowed `exec_insns_isolated` path that
/// runs on separate threads.
/// Self-determined bit width of a built-in type keyword, for `$bits(<type>)`
/// where the type parses as a bare identifier (§6.11, §20.6.2). Returns None
/// for non-type names so a same-named signal/typedef path is used instead.
pub(super) fn atom_type_keyword_width(name: &str) -> Option<u32> {
    let name = name
        .trim()
        .strip_suffix(" unsigned")
        .or_else(|| name.trim().strip_suffix(" signed"))
        .unwrap_or(name.trim())
        .trim_end();
    Some(match name {
        "integer" | "int" => 32,
        "shortint" => 16,
        "longint" | "time" | "real" | "realtime" => 64,
        "shortreal" => 32,
        "byte" => 8,
        "bit" | "logic" | "reg" => 1,
        _ => return None,
    })
}

/// IEEE 1800-2017 §18.4: where a member of an aggregate (struct/union) CLASS
/// property actually lives. Class properties are cells in `heap[h].properties`,
/// not signals, so member access has to be routed here rather than through the
/// flat-signal struct paths.
#[derive(Debug, Clone)]
pub(super) enum ClassAggRef {
    /// Member of a PACKED struct/union — a bit slice `[off +: w]` of the
    /// property's raw `total`-bit integral value. Union members all sit at
    /// `off == 0` and therefore alias one another (§7.3.1).
    Packed {
        handle: usize,
        prop: String,
        off: u32,
        w: u32,
        total: u32,
        /// §7.2.1: a 2-state member of a 4-state struct — its reads and
        /// writes convert x/z to 0.
        two_state: bool,
        /// §7.2.1: a whole member declared signed reads signed.
        signed: bool,
    },
    /// Member of an UNPACKED struct — its own cell, keyed `<prop>.<member>`.
    Unpacked { handle: usize, key: String, w: u32 },
    /// A bit SLICE of one per-instance cell of an unpacked-struct property
    /// member: `[off +: w]` of the cell's `cell_w` bits. This is how a
    /// PACKED-ARRAY member of an unpacked struct is addressed (the whole
    /// array is one cell, §7.4.1 slot math picks the element) and how a
    /// part-select slices an element of an unpacked-array member.
    UnpackedSlice {
        handle: usize,
        key: String,
        off: u32,
        w: u32,
        cell_w: u32,
    },
}

/// Where a `class_packed_operand` value lives: the whole property, or an
/// element of its unpacked dimension(s) under the per-instance key
/// `<h>#<prop>[k]...` — the write counterpart splices back there.
#[derive(Clone, Debug)]
pub(super) enum ClassPackedStore {
    Prop,
    ElemSuffix(String),
}

/// One step of a member/select chain being resolved into class aggregate
/// storage: a named member, a constant index, or a part-select.
#[derive(Debug, Clone)]
pub(super) enum MemberSel {
    Member(String),
    Index(i64),
    Range { kind: RangeKind, l: i64, r: i64 },
}

/// One step of a member/select chain, UNEVALUATED — [`class_member_select_ref`]
/// resolves the receiver/property boundary before any index expression is
/// evaluated, so a chain rooted at no struct-valued class property never
/// pays for its selects.
#[derive(Clone, Copy)]
pub(super) enum PendingSel<'a> {
    Member(&'a str),
    Index(&'a Expression),
    Range {
        kind: RangeKind,
        left: &'a Expression,
        right: &'a Expression,
    },
}

/// How one select of a chain addresses the packed value it is applied to
/// (see [`Simulator::packed_hops`]), before any index is evaluated.
#[derive(Debug, Clone, Copy)]
pub(super) enum PackedHop {
    /// A packed dimension declared `[l:r]` of elements `ew` bits wide.
    /// §7.4.1: the left bound labels the most significant element.
    Dim { l: i64, r: i64, ew: u32 },
    /// A member of a packed struct or union, `off` bits above its LSB and
    /// `w` bits wide.
    Field { off: u32, w: u32 },
}

/// The §7.8 collection kind of an unpacked-struct member whose declarator
/// carries an unsized `[]`, queue `[$]` or associative `[k]` dimension.
/// (Distinct from the rand-collection [`CollKind`]: this one separates the
/// queue and dynamic registrations.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StructCollKind {
    Dynamic,
    Queue,
    Assoc,
}

/// IEEE 1800-2017 §18.4: a constraint target that is not a bare rand property
/// of the object being solved.
#[derive(Debug, Clone)]
pub(super) enum RandMemberTarget {
    /// A member of an aggregate (struct/union) rand property.
    Agg(ClassAggRef),
    /// A property of a `rand` OBJECT HANDLE — solved concurrently with the
    /// enclosing object (the handle itself is never written).
    Sub(usize, String),
}

// Per-thread path of value-parameter names currently being resolved by
// `Simulator::resolve_value_param_from_spec`. Used to break cyclic value-param
// references (a specialization argument that names a value param of the same
// class, which would otherwise recurse infinitely and overflow the stack).
thread_local! {
    pub(super) static VALPARAM_RESOLVING: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::new());
}

/// Drop guard that pops the last entry from `VALPARAM_RESOLVING` when it goes
/// out of scope, ensuring the path is unwound on every return path (including
/// `?` early returns) from `resolve_value_param_from_spec`.
pub(super) struct ValparamResolveGuard;

impl Drop for ValparamResolveGuard {
    fn drop(&mut self) {
        VALPARAM_RESOLVING.with(|r| r.borrow_mut().pop());
    }
}
