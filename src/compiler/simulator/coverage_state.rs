//! Covergroup instances, bins and coverpoint shapes.

use super::*;

/// Covergroup handles live above this tag; class handles are plain heap indices.
pub(super) const CG_HANDLE_TAG: usize = 1 << 30;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CovergroupInstance {
    /// Coverpoints whose declared expression or sample formal is real (§19.5).
    pub(super) real_points: HashSet<String>,
    pub(super) cg_name: String,
    /// The object whose constructor ran `cg = new` (a class-body
    /// covergroup): coverpoint expressions read ITS properties, so sampling
    /// from outside the class (`e.cg.sample()`) evaluates with this `this`.
    pub(super) owner: Option<usize>,
    /// §19.3 constructor formals bound at `new(...)`: visible to coverpoint
    /// expressions, bins and options of this instance.
    pub(super) ctor_args: Vec<(String, Value)>,
    /// §19.3 `ref` constructor formals and their actual expressions: read
    /// again at every sample rather than captured at `new`.
    pub(super) ctor_refs: Vec<(String, Expression)>,
    /// §8.25/§19.5: the value parameters of the owning object's class
    /// specialization (`N` of `group_n #(2)`), for bins and options of a
    /// class-body covergroup. A constructor formal of the same name hides one.
    pub(super) class_params: Vec<(String, Value)>,
    /// Hits: coverpoint_name -> observed value -> times sampled
    pub(super) point_hits: HashMap<String, HashMap<Value, u64>>,
    /// Cross hits: cross_name -> observed value tuple -> times sampled
    pub(super) cross_hits: HashMap<String, HashMap<Vec<Value>, u64>>,
    /// §19.6.1 hit counts of the bins of a cross body. Key:
    /// `cross_name.bin_name`. A sample counts once for each bin selecting a
    /// product of the bins its values fall in.
    pub(super) cross_bin_hits: HashMap<String, u64>,
    /// Per-explicit-bin hit counts (LRM §19.5). Key: `coverpoint.bin`. Only
    /// populated for coverpoints that declared `bins`. Sampling increments
    /// the counter when the value falls in any of the bin's ranges; an
    /// `illegal_bins` hit also fires an immediate error.
    #[allow(dead_code)]
    pub(super) bin_hits: HashMap<String, u64>,
    /// §19.5.2 in-flight attempts of each transition sequence, by bin key
    /// (`cp.bin#<n>` for the n-th sequence of a scalar bin): (step, count).
    pub(super) trans_pos: HashMap<String, HashSet<(u32, u32)>>,
    /// Total `sample()` invocations on this instance.
    pub(super) sample_count: u64,
    /// §19.8 `stop()`: sampling is off until `start()`.
    pub(super) stopped: bool,
    /// Coverpoints and crosses stopped with `cg.item.stop()`.
    pub(super) stopped_items: HashSet<String>,
    /// §19.7 `cg.option.<name> = v` written at run time.
    pub(super) options: HashMap<String, Value>,
}

/// One bin of a coverpoint as coverage counts it (§19.5).
pub(super) struct CgBin {
    /// Key of its hit counter in `bin_hits`; None for an automatic bin,
    /// whose hits come from the sampled values.
    pub(super) key: Option<String>,
    /// The declared bin name (`auto[k]` for an automatic bin).
    pub(super) name: String,
    /// Constant value ranges; empty for a transition bin or non-constant
    /// values.
    pub(super) ranges: Vec<(i64, i64)>,
}

/// Which bins own one product of a cross's bins (§19.6.1).
pub(super) enum CrossOwner {
    /// Removed by the ignore_bins or illegal_bins at this index of the
    /// cross body.
    Excluded(usize),
    /// An automatic cross bin of its own.
    Auto,
    /// Held by these user bins (indices among the body's `bins`).
    User(Vec<usize>),
}

/// One resolved step of a §19.5.2 transition: its values and repetition
/// count range (`max` None = unbounded).
#[derive(Clone)]
pub(super) struct TransAtom {
    pub(super) ranges: Vec<(i64, i64)>,
    pub(super) kind: crate::ast::decl::TransRepeatKind,
    pub(super) min: u32,
    pub(super) max: Option<u32>,
}

/// The bins of a coverpoint, or of a crossed variable.
pub(super) struct CpShape {
    pub(super) bins: Vec<CgBin>,
    /// Automatic bins: (value space, values per bin, bin count).
    pub(super) auto: Option<(u64, u64, u64)>,
    /// Values removed by `ignore_bins` / `illegal_bins`.
    pub(super) excl: Vec<(i64, i64)>,
}
