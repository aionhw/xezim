//! Random number generator state, randomized collections and constraint helpers.

use super::*;

/// A random stream (IEEE 1800-2023 §18.14 "Random stability").
///
/// §18.14 requires every *object* and every *process* to own an independent,
/// seedable stream whose state can be captured (`get_randstate`) and restored
/// (`set_randstate`) verbatim — a restored stream must replay the identical
/// sequence. `StdRng` (ChaCha) has no stable, round-trippable textual state, so
/// the streams use an explicit SplitMix64 generator: the entire state is one
/// `u64`, which serialises to 16 hex digits and restores exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SvRng {
    state: u64,
}

impl SvRng {
    /// The seed a run uses when it does not ask for another one. Fixed, not
    /// drawn from entropy, so that two runs of the same design are identical
    /// and a failing random test can be re-run and debugged. `+seed=<n>`
    /// picks a different stream; `+seed=random` opts into entropy.
    pub(super) const DEFAULT_SEED: u64 = 1;

    /// §18.14: seeding is deterministic — equal seeds yield equal sequences.
    pub(super) fn from_seed(seed: u64) -> Self {
        SvRng {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// §18.14.2 `get_randstate`: the full state as an implementation-defined
    /// string (here: 16 hex digits).
    pub(super) fn to_state_string(&self) -> String {
        format!("{:016x}", self.state)
    }

    /// §18.14.2 `set_randstate`: restore a state produced by `to_state_string`.
    /// A foreign string is implementation-defined; hash it into a seed rather
    /// than ignore it.
    pub(super) fn from_state_string(s: &str) -> Self {
        let t = s.trim();
        if let Ok(v) = u64::from_str_radix(t, 16) {
            SvRng { state: v }
        } else {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in t.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            SvRng { state: h }
        }
    }

    /// SplitMix64 step.
    fn next_state(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl rand::RngCore for SvRng {
    fn next_u32(&mut self) -> u32 {
        (self.next_state() >> 32) as u32
    }
    fn next_u64(&mut self) -> u64 {
        self.next_state()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut i = 0;
        while i < dest.len() {
            let chunk = self.next_state().to_le_bytes();
            let n = (dest.len() - i).min(8);
            dest[i..i + n].copy_from_slice(&chunk[..n]);
            i += n;
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

/// The consequent of a class-constraint implication: a constraint item
/// (`ConstraintItem::Implication`) or the right operand of a `->` expression.
pub(super) enum ImpCons<'a> {
    Item(&'a ConstraintItem),
    Expr(&'a Expression),
}

// =========================================================================
// IEEE 1800-2017 §18.4 / §18.5.9 — rand COLLECTION members.
// =========================================================================

/// The three storage shapes a rand collection member can have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CollKind {
    /// `bit [7:0] a[4]` — fixed unpacked array, bounds known at elaboration.
    Fixed,
    /// `bit [7:0] a[]` / `a[$]` — dynamic array or queue (length is solved).
    Dyn,
    /// `bit [7:0] a[int]` — associative array (sparse; keys are pre-seeded).
    Assoc,
}

/// One rand collection member of the object being randomized.
#[derive(Clone, Debug)]
pub(super) struct RandColl {
    /// Bare member name (`dyn_arr`).
    pub(super) prop: String,
    /// Instance-scoped storage prefix (`<handle>#dyn_arr`).
    pub(super) scoped: String,
    pub(super) kind: CollKind,
    /// Element width in bits.
    pub(super) width: u32,
    /// Fixed-array bounds (unused for Dyn/Assoc).
    pub(super) lo: i64,
    pub(super) hi: i64,
    /// Dynamic array of dynamic arrays (`bit [7:0] m[][]`).
    pub(super) nested: bool,
    /// Element type is a CLASS (`rand obj arr[]`). Such elements are object
    /// handles randomized RECURSIVELY (§18.4.1), never drawn as scalars —
    /// a fresh random draw would overwrite the handle with junk, destroying
    /// the object reference and any aliasing (`arr[1]==arr[2]`).
    pub(super) is_object_elem: bool,
    /// §18.4 (#519): a ROW of a multi-dimensional unpacked array (`m[0]`
    /// of `rand int m[2][2]`): the base member and its declared bounds,
    /// outermost first. The solver keys a row like a 1-D array and walks
    /// the leading dimensions through the bounds (`Csp::nd_dims`).
    pub(super) nd_base: Option<(String, Vec<(i64, i64)>)>,
    /// §7.4.5: a dynamic outer dimension whose elements are unpacked arrays
    /// (`T a[][N]`, `T a[][]`): the shape of one row. The member's own
    /// elements are the rows; the joint solver keys each row like a row of
    /// a multi-dimensional array (`nd_base`) once the outer size is known.
    pub(super) row_tail: Option<RowTail>,
    /// §18.4: one rand member of an array of unpacked structs, solved as a
    /// collection of its own (`prop` is `arr.m`, elements `arr[i].m`).
    pub(super) member: Option<StructElemMember>,
}

/// The rows of a dynamic outer dimension (`RandColl::row_tail`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowTail {
    /// `T a[][N][M]`: every row has these fixed dimensions.
    Fixed(Vec<(i64, i64)>),
    /// `T a[][]`: every row is a dynamic array with its own size.
    Dyn,
}

#[derive(Clone, Debug)]
pub(super) struct StructElemMember {
    pub(super) name: String,
    pub(super) signed: bool,
    pub(super) enum_type: Option<String>,
}

impl RandColl {
    /// Storage key of element `i` (`<h>#arr[i]`, or `<h>#arr[i].m`).
    pub(super) fn elem_key(&self, i: impl std::fmt::Display) -> String {
        match &self.member {
            Some(m) => format!("{}[{}].{}", self.scoped, i, m.name),
            None => format!("{}[{}]", self.scoped, i),
        }
    }
}

/// A `.size()` constraint on a rand dynamic array (§18.4): the array must be
/// SIZED before its elements can be solved.
pub(super) enum SizeCon {
    /// `a.size() == E` — E already evaluated.
    Eq(u64),
    /// `a.size() == E` with a negative signed target or one above the trial cap.
    Impossible,
    /// `a.size() inside {…}` — the ranges already evaluated.
    In(Vec<(u64, u64)>),
    /// `a.size() < E`, `>= E`, … — E already evaluated.
    Rel(crate::ast::expr::BinaryOp, u64),
    /// `a.size() dist {…}` — each item's bounds and its weight over them.
    Dist(Vec<(u64, u64, u128)>),
}
