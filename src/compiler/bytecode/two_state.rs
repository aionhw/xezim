//! Two-state instruction representation and lowering from four-state bytecode.

use super::*;

// ---------------------------------------------------------------------------
// Two-state lowering (P5). A comb block whose reads are all X-free at eval
// time can run on plain u64 words: no 4-state masks, no `Value` clones, no
// storage dispatch. `lower_two_state` translates a compiled block's insn
// stream 1:1 into `TsInsn`s with STATICALLY-known widths and masks, bailing
// (returning None) on anything whose 2-state semantics are not proven here:
//
// - signed loads/consts (2-state Resize only zero-extends; every lowered reg
//   is unsigned by construction, so a Resize that widens is a no-op),
// - out-of-STATIC-range selects (4-state reads produce X bits there),
// - fill/real/wide(>64)/X-carrying constants,
// - width-changing or real/wide writebacks,
// - any opcode outside the proven set (branches, fallbacks, arrays, …).
//
// The eval site must additionally check, per evaluation: every signal in
// `read_sigs` currently holds an X-free value (width ≤ 64 was proven at
// lower time, so storage is Inline and `raw_bits` is exact), no forces are
// active, and `warn_x` is off (the 2-state path skips its bookkeeping).
// On any miss it falls back to the 4-state stream, which is always correct.
// ---------------------------------------------------------------------------

/// Payload of the dynamic array-element ops, boxed to keep `TsInsn` at 24
/// bytes (the `i64` bounds pair alone would push every instruction to 40
/// bytes and cost measurable dispatch bandwidth on long comb streams).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TsElemStoreFromSig {
    pub sig: u32,
    pub dl: u16,
    pub op: TsElemOp,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TsElemOp {
    pub first: u32,
    pub lo: i64,
    pub hi: i64,
    pub idx: u16,
    pub s: u16,
    pub w: u32,
    pub mask: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TsNbaFromElem {
    pub dst: u32,
    pub first: u32,
    pub lo: i64,
    pub hi: i64,
    pub idx_sig: u32,
    pub w: u32,
}

/// One two-state instruction. Register file is `u64`; every value is kept
/// masked to its static width by construction. Branch targets are indices
/// into the LOWERED stream (remapped from the 4-state stream's indices).
/// Variant name of a two-state instruction (census / diagnostics only).
#[cfg(feature = "opcode-census")]
pub fn ts_insn_name(i: &TsInsn) -> &'static str {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static NAMES: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let dbg = format!("{:?}", i);
    let key: String = dbg
        .split(|c: char| c == ' ' || c == '{' || c == '(')
        .next()
        .unwrap_or("?")
        .to_string();
    let mut g = NAMES.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(s) = m.get(&key) {
        return s;
    }
    let leaked: &'static str = Box::leak(key.clone().into_boxed_str());
    m.insert(key, leaked);
    leaked
}

/// Boxed payload of `RangeStoreX`/`RangeStoreXW` (rare: folded x-constant
/// range stores), kept off the hot enum size.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TsRangeStoreX {
    pub sig: u32,
    pub hi: u32,
    pub lo: u32,
    pub v: u64,
    pub x: u64,
}

/// Boxed payload of `CaseMaskJmp`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TsCaseMaskJmp {
    pub mask: u64,
    pub lo: u32,
    pub wmask: u64,
    pub mj: Box<CaseMaskJumpData>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TsInsn {
    /// regs[d] = signal_table[sig] (raw value bits; proven X-free by the
    /// eval-site prefilter).
    LoadSig {
        d: u16,
        sig: u32,
    },
    Const {
        d: u16,
        v: u64,
    },
    /// regs[d] = bit `bit` of a ≤64-bit signal (inline raw_bits path).
    SigBit {
        d: u16,
        sig: u32,
        bit: u16,
    },
    /// regs[d] = (signal >> lo) & mask, ≤64-bit signal.
    SigRange {
        d: u16,
        sig: u32,
        lo: u16,
        mask: u64,
    },
    /// Wide-signal (>64) variants — read the addressed slice only.
    SigBitW {
        d: u16,
        sig: u32,
        bit: u16,
    },
    SigRangeW {
        d: u16,
        sig: u32,
        lo: u16,
        w: u16,
        mask: u64,
    },
    /// regs[d] = bit `bit` of regs[s].
    Bit {
        d: u16,
        s: u16,
        bit: u8,
    },
    /// regs[d] = (regs[s] >> lo) & mask.
    Range {
        d: u16,
        s: u16,
        lo: u8,
        mask: u64,
    },
    Xor {
        d: u16,
        a: u16,
        b: u16,
    },
    And {
        d: u16,
        a: u16,
        b: u16,
    },
    Or {
        d: u16,
        a: u16,
        b: u16,
    },
    /// regs[d] = if regs[c] != 0 { regs[a] } else { regs[b] } — §11.4.11
    /// conditional. Two-state values are X-free by the eval-site prefilter,
    /// so the 4-state arm's unknown-condition bit-merge cannot arise and the
    /// selector reduces to plain SV truthiness (non-zero).
    /// `m`: the result width mask, used only by the x-plane executor's
    /// merge of the two arms under an x selector.
    Sel {
        d: u16,
        c: u16,
        a: u16,
        b: u16,
        m: u64,
    },
    /// regs[d] = !regs[s] & mask (mask = source width).
    Not {
        d: u16,
        s: u16,
        mask: u64,
    },
    XorC {
        d: u16,
        s: u16,
        k: u64,
    },
    /// regs[d] = (regs[s] == k) — 4-state Eq/CaseEq agree on clean values.
    EqC {
        d: u16,
        s: u16,
        k: u64,
    },
    /// Wrapping two's-complement at max operand width (`mask`); operands are
    /// zero-extended (all lowered registers are unsigned).
    Add {
        d: u16,
        a: u16,
        b: u16,
        mask: u64,
    },
    Sub {
        d: u16,
        a: u16,
        b: u16,
        mask: u64,
    },
    /// 1-bit comparison results; unsigned compare per §5.5.1 (either operand
    /// unsigned ⇒ unsigned, and every lowered register is unsigned).
    Eq {
        d: u16,
        a: u16,
        b: u16,
    },
    /// regs[d] = ((regs[s] & mask) == v) — a `casez`/`casex` compare against
    /// a constant pattern: `mask` clears the pattern's wildcard bits (z, or
    /// x too for casex) and the bits above the compare width; `v` is the
    /// pattern value under that mask. The selector is X-free on this path.
    MaskEq {
        d: u16,
        s: u16,
        mask: u64,
        v: u64,
    },
    Neq {
        d: u16,
        a: u16,
        b: u16,
    },
    /// Fused adjacent pairs (see `fuse_ts_pairs`). Each one is exactly its
    /// two component instructions run back to back — same register writes,
    /// same order — minus one dispatch through the executor's jump table.
    LoadSigNot {
        dl: u16,
        d: u16,
        sig: u32,
    },
    /// `lo`/`w` (width) instead of `lo`/`mask`: keeps the payload ≤ 22 bytes
    /// so `TsInsn` is 24 bytes (was 32); the executor rebuilds the mask.
    SigRangeEqC {
        dr: u16,
        d: u16,
        sig: u32,
        lo: u8,
        w: u8,
        k: u64,
    },
    LoadSigLogAnd {
        dl: u16,
        sig: u32,
        d: u16,
        a: u16,
        b: u16,
    },
    LogNotAnd {
        dn: u16,
        s: u16,
        d: u16,
        a: u16,
        b: u16,
    },
    LogNotLogAnd {
        dn: u16,
        s: u16,
        d: u16,
        a: u16,
        b: u16,
    },
    LogAndStore {
        d: u16,
        a: u16,
        b: u16,
        sig: u32,
        mask: u64,
    },
    AndRangeStore {
        d: u16,
        a: u16,
        b: u16,
        sig: u32,
        hi: u32,
        lo: u32,
    },
    SigBitNot {
        db: u16,
        d: u16,
        sig: u32,
        bit: u16,
    },
    /// Second fusion batch (same rules).
    LoadSig2 {
        d1: u16,
        sig1: u32,
        d2: u16,
        sig2: u32,
    },
    SigBit2 {
        d1: u16,
        sig1: u32,
        bit1: u16,
        d2: u16,
        sig2: u32,
        bit2: u16,
    },
    LoadSigBrNz {
        dl: u16,
        sig: u32,
        t: u32,
    },
    BrFalseLoadSig {
        s: u16,
        t: u32,
        dl: u16,
        sig: u32,
    },
    EqBrFalse {
        d: u16,
        a: u16,
        b: u16,
        t: u32,
    },
    /// Constant equality followed by a false branch. The comparison still
    /// writes `d`, including when it aliases `s` or the branch is taken.
    EqCBrFalse {
        d: u16,
        s: u16,
        k: u64,
        t: u32,
    },
    LoadSigLogOr {
        dl: u16,
        sig: u32,
        d: u16,
        a: u16,
        b: u16,
    },
    LoadSigAnd {
        dl: u16,
        sig: u32,
        d: u16,
        a: u16,
        b: u16,
    },
    LoadSigRepl {
        dl: u16,
        sig: u32,
        d: u16,
        w: u8,
        count: u8,
    },
    LoadSigSigRange {
        dl: u16,
        sig: u32,
        d: u16,
        sig2: u32,
        lo: u16,
        mask: u64,
    },
    SigRangeAnd {
        dr: u16,
        sig: u32,
        lo: u16,
        mask: u64,
        d: u16,
        a: u16,
        b: u16,
    },
    SigRangeEq {
        dr: u16,
        sig: u32,
        lo: u16,
        mask: u64,
        d: u16,
        a: u16,
        b: u16,
    },
    ConstEq {
        dc: u16,
        v: u64,
        d: u16,
        a: u16,
        b: u16,
    },
    LogOrStore {
        d: u16,
        a: u16,
        b: u16,
        sig: u32,
        mask: u64,
    },
    AndOr {
        d1: u16,
        a1: u16,
        b1: u16,
        d: u16,
        a: u16,
        b: u16,
    },
    OrRangeStore {
        d: u16,
        a: u16,
        b: u16,
        sig: u32,
        hi: u32,
        lo: u32,
    },
    Lt {
        d: u16,
        a: u16,
        b: u16,
    },
    /// Signed relational compare (§11.8.1: both operands signed). `sa`/`sb`
    /// are the shifts that sign-extend each operand from its static width;
    /// `kind`: 0 `<`, 1 `<=`, 2 `>`, 3 `>=`.
    CmpS {
        d: u16,
        a: u16,
        b: u16,
        kind: u8,
        sa: u8,
        sb: u8,
    },
    /// `sig[regs[i]] <= regs[s]` (§11.5.1: an out-of-range index writes
    /// nothing); merges into the pending entry like `RangeStoreNba`.
    BitStoreNbaDyn {
        sig: u32,
        i: u16,
        s: u16,
        w: u32,
    },
    /// `d = regs[s][regs[i]]` on a narrow register. An out-of-range index
    /// reads x in four-state (§11.5.1), which the executor cannot produce:
    /// it ABORTS and the four-state re-run supplies the x.
    BitDyn {
        d: u16,
        s: u16,
        i: u16,
        w: u32,
    },
    Leq {
        d: u16,
        a: u16,
        b: u16,
    },
    Gt {
        d: u16,
        a: u16,
        b: u16,
    },
    Geq {
        d: u16,
        a: u16,
        b: u16,
    },
    LogNot {
        d: u16,
        s: u16,
    },
    LogAnd {
        d: u16,
        a: u16,
        b: u16,
    },
    LogOr {
        d: u16,
        a: u16,
        b: u16,
    },
    /// MSB-first parts, mirroring `Value::concat_refs`.
    Concat {
        d: u16,
        parts: Box<[(u16, u8)]>,
    },
    /// Two- and three-part `Concat` with the operands in the instruction
    /// itself: `d = (a << wb) | b` and `d = ((a << wb | b) << wc) | c`.
    /// The boxed-parts form chases a heap pointer per evaluation; on the
    /// C910 `Concat` was 8% of executor time and most concats have two or
    /// three parts.
    Concat2 {
        d: u16,
        a: u16,
        wa: u8,
        b: u16,
        wb: u8,
    },
    Concat3 {
        d: u16,
        a: u16,
        wa: u8,
        b: u16,
        wb: u8,
        c: u16,
        wc: u8,
    },
    /// In-place truncation (a `Resize` that narrows; widening is free).
    Mask {
        d: u16,
        mask: u64,
    },
    /// Jump to `t` when the signal (or its `bit`, when != u32::MAX) is zero.
    /// The prefilter proved the signal X-free, so `!is_true` = `== 0`.
    BrSigFalse {
        sig: u32,
        bit: u32,
        t: u32,
    },
    /// Jump to `t` when regs[s] == 0 (4-state `BranchIfFalse` on a clean reg).
    BrFalse {
        s: u16,
        t: u32,
    },
    /// Jump to `t` when regs[s] != 0 (4-state `BranchUnlessZero`).
    BrNz {
        s: u16,
        t: u32,
    },
    Jmp {
        t: u32,
    },
    /// A backward `Jmp` (a loop back-edge; rewritten once the stream is
    /// final): Ctrl-C bails the block to the four-state VM, whose back-edge
    /// poll stops the run. A long loop inside one time slot never reaches the
    /// event loop's own poll. A separate instruction rather than a test in
    /// `Jmp`, so forward jumps pay nothing.
    JmpPoll {
        t: u32,
    },
    /// Ahead of a do-while back-edge (`BrNz`), same purpose as `JmpPoll`.
    LoopPoll,
    /// Jump-table dispatch (§12.5 `case`). Two-state registers are X-free by
    /// the eval-site prefilter, so the 4-state arm's "any x/z bit matches no
    /// pattern -> default" branch cannot arise and this reduces to a bounds-
    /// checked table index. Targets are LOWERED indices (remapped in the same
    /// fixup pass as the other branches).
    CaseJmp {
        s: u16,
        cj: Box<CaseJumpData>,
    },
    /// Bucket-window jump table (§12.5 `case` with wildcard-free windows).
    /// X-free registers make the 4-state `xz_path` (wildcard selector can
    /// match several buckets) unreachable, leaving a plain window index.
    CaseMaskJmp {
        s: u16,
        mj: Box<TsCaseMaskJmp>,
    },
    /// regs[d] = (regs[s] != 0) — §11.4.9 reduction OR on an X-free operand.
    RedOr {
        d: u16,
        s: u16,
    },
    /// Wide (65..512-bit) reduction OR: reads the WIDE register file. The
    /// narrow `RedOr` on a wide source read `regs[s]` — a slot the wide load
    /// never wrote, i.e. whatever the PREVIOUS block's evaluation left there.
    WRedOr {
        d: u16,
        s: u16,
    },
    /// Wide reduction AND: value == all-ones over its width `w`.
    WRedAnd {
        d: u16,
        s: u16,
        w: u16,
    },
    /// regs[d] = (regs[s] == mask) — §11.4.9 reduction AND; `mask` is the
    /// source width, so "all ones" is an equality against it.
    RedAnd {
        d: u16,
        s: u16,
        mask: u64,
    },
    /// `sig[regs[i]] = regs[s] & 1` (§11.5.1 dynamic bit-select target).
    /// Out-of-range indices are dropped, matching `Value::set_bit`. Splices
    /// through the same plane-level merge as `RangeStore`, so X elsewhere in
    /// the destination survives. `w > 64` (wide destination) sets the bit in
    /// the wide planes directly.
    BitStoreDyn {
        sig: u32,
        i: u16,
        s: u16,
        w: u32,
    },
    /// `d = sig[i +: w]` at a run-time offset, on a narrow or a wide
    /// signal; a slice running past the signal aborts (the four-state read
    /// yields x there), an x inside it bails like any signal read.
    SigRangeDyn {
        d: u16,
        sig: u32,
        i: u16,
        w: u16,
        sw: u32,
        mask: u64,
    },
    /// `sig[i +: w] = s` at a run-time offset; a slice past the signal
    /// aborts to the four-state re-run.
    RangeStoreDyn {
        sig: u32,
        i: u16,
        s: u16,
        w: u16,
        sw: u32,
        mask: u64,
    },
    /// `sig[i +: w] <= s` at a run-time offset.
    RangeStoreNbaDyn {
        sig: u32,
        i: u16,
        s: u16,
        w: u16,
        sw: u32,
        mask: u64,
    },
    /// Save a WIDE (65..=512-bit) signal the block reads before writing,
    /// so a later bail can restore it; the narrow form is `SaveSig`.
    SaveSigW {
        sig: u32,
    },
    /// Wide mux: `d = c ? a : b` on the wide register file (`c` narrow).
    WSel {
        d: u16,
        c: u16,
        a: u16,
        b: u16,
    },
    /// `sig[hi:lo] = {N{bit}}` where the bit is x or z: fill both planes
    /// (`v` is the value bit, x is always set). A reset default of `'x` on
    /// a wide bus takes this form.
    RangeFillXW {
        sig: u32,
        hi: u32,
        lo: u32,
        v: u8,
    },
    /// Blocking store of a folded 4-state constant (`y = 'x;`). `v`/`x` are
    /// the raw value/xz planes, already masked to the assigned width.
    ConstStoreX {
        sig: u32,
        v: u64,
        x: u64,
    },
    /// Partial-range counterpart (`y[hi:lo] = 'x;`).
    RangeStoreX(Box<TsRangeStoreX>),
    /// Write back `regs[s] & mask` to `sig` with change-detect + dirty
    /// marking (the eval site mirrors the 4-state fast-path bookkeeping).
    Store {
        sig: u32,
        s: u16,
        mask: u64,
    },
    /// Nonblocking write: queue `regs[s] & mask` (width `w`) with §10.4.2
    /// last-write-wins and the eval-time elision, exactly as `NbaAssign`.
    StoreNba {
        sig: u32,
        s: u16,
        w: u32,
        mask: u64,
    },
    /// NBA of a compile-time CONSTANT (`q <= 8'h3f;`) — the single largest
    /// two-state bail on c906 (1,189 edge blocks). Non-abortable: the value
    /// was validated 2-state at lowering.
    ConstStoreNba {
        sig: u32,
        v: u64,
        w: u32,
    },
    /// Partial-bit NBA (`q[hi:lo] <= v`). Mirrors the 4-state executor's
    /// `compose_inline_range_bits` merge against the pending-or-current
    /// value — the composition works on raw bit PLANES, so an X base flows
    /// through instead of aborting; only the SOURCE must be 2-state, and it
    /// is (it lives in a ts register). Non-abortable.
    RangeStoreNba {
        sig: u32,
        hi: u32,
        lo: u32,
        s: u16,
        mask: u64,
    },
    /// Blocking counterpart of `RangeStoreNba`: splice regs[s] into
    /// signal[hi:lo] immediately (§10.4.1), preserving the bits outside the
    /// window. The TS bail census measured `BlockingAssignRange` gating
    /// 133.2M interpreter evaluations on the C906 SoC — 42.9% of them, the
    /// single largest reason two-state lowering gave up.
    RangeStore {
        sig: u32,
        hi: u32,
        lo: u32,
        s: u16,
        mask: u64,
    },
    /// `RangeStore` into a >64-bit destination: the ≤64-bit slice regs[s]
    /// replaces bits [hi:lo] of the wide planes; everything else (X
    /// included) is preserved. Routed through the same splice the JIT bridge
    /// uses for wide destinations.
    RangeStoreW {
        sig: u32,
        hi: u32,
        lo: u32,
        s: u16,
        mask: u64,
    },
    /// `RangeStoreNba` for a destination wider than 64 bits: the window is
    /// spliced into the pending NBA value (seeded from the signal).
    RangeStoreNbaW {
        sig: u32,
        hi: u32,
        lo: u32,
        s: u16,
        mask: u64,
    },
    /// `sig[hi:lo] = {N{bit}}` for any width (the `bus = {265{1'b0}}` default
    /// assignment that opens most c906 decode blocks): a constant fill of
    /// the window, blocking and non-blocking.
    RangeFillW {
        sig: u32,
        hi: u32,
        lo: u32,
        bit: u8,
    },
    RangeFillNbaW {
        sig: u32,
        hi: u32,
        lo: u32,
        bit: u8,
    },
    /// `RangeStoreX` into a >64-bit destination (folded 4-state constant).
    RangeStoreXW(Box<TsRangeStoreX>),
    /// Dynamic array-element read: eid = first + (regs[idx] - lo). ABORTS
    /// the two-state run (caller re-runs 4-state) when the index is out of
    /// range or the element holds X — both produce X in 4-state. Lowering
    /// only admits this before any side-effecting op, so an abort is clean.
    ElemLoad(Box<TsElemOp>),
    /// `ElemLoad` into the wide bank: elements wider than 64 bits (a
    /// vector unit's register file, a cache line array). Out-of-range
    /// aborts like `ElemLoad`; an element holding X is an x-read bail, so a
    /// persistently undefined memory sleeps like any other x source.
    WElemLoad(Box<TsElemOp>),
    /// Dynamic array-element NBA (`mem[waddr] <= v`): out-of-range DROPS
    /// silently (4-state behavior); otherwise mirrors the NbaAssignArray
    /// arm (elide-if-equal, then index+push).
    ElemStoreNba(Box<TsElemOp>),
    /// `LoadSig { d } ; ElemStoreNba { s: d }` fused: the data comes straight
    /// from the source signal's planes, and x/z bits are QUEUED as a 4-state
    /// value instead of bailing the block (a RAM written with x data — the
    /// image-load phase, uninitialised words — fired that bail on every
    /// clock edge: c906 1.7M x-read bails per 100 iterations from the
    /// `if (we) mem[a] <= d` blocks). `d` is still written for later readers.
    ElemStoreNbaFromSig(Box<TsElemStoreFromSig>),
    /// Dynamic array-element blocking write; out-of-range drops silently.
    ElemStore(Box<TsElemOp>),
    /// Fused array-read-to-NBA (`q <= mem[raddr]`, NbaAssignArrayRead):
    /// index comes from a SIGNAL; aborts on out-of-range or X element
    /// (4-state queues an X value there).
    NbaFromElem(Box<TsNbaFromElem>),
    /// `NbaFromElem` for elements wider than 64 bits: the element Value is
    /// queued whole (the NBA queue holds Values), so it needs no wide
    /// register and runs in every executor through the out-of-line arm.
    WNbaFromElem(Box<TsNbaFromElem>),
    // ---- WIDE (65..=512-bit) bank: little-endian [u64; N] registers, N
    // = 2 or 8 words per block (`TwoStateBlock::wide_words`); widths give
    // the executor the top word and its mask. ----
    /// wregs[d] = signal words; bails when the signal holds X/Z.
    WLoadSig {
        d: u16,
        sig: u32,
    },
    /// Little-endian words of the constant (≤ N of them).
    WConst {
        d: u16,
        v: Box<[u64]>,
    },
    WXor {
        d: u16,
        a: u16,
        b: u16,
    },
    WAnd {
        d: u16,
        a: u16,
        b: u16,
    },
    WOr {
        d: u16,
        a: u16,
        b: u16,
    },
    /// wregs[d] = !wregs[s] masked to width `w`.
    WNot {
        d: u16,
        s: u16,
        w: u16,
    },
    /// Wide→wide slice: wregs[d] = (wregs[s] >> lo) & mask(w), w in 65..=512.
    WRange {
        d: u16,
        s: u16,
        lo: u16,
        w: u16,
    },
    /// Narrow (≤64) slice of a wide register.
    RangeFromW {
        d: u16,
        s: u16,
        lo: u16,
        mask: u64,
    },
    BitFromW {
        d: u16,
        s: u16,
        bit: u16,
    },
    /// MSB-first concat into a wide register; parts (reg, width, is_wide)
    /// may be narrow or wide.
    WConcat {
        d: u16,
        parts: Box<[(u16, u16, bool)]>,
    },
    /// Resize truncation within the wide bank to width `w`.
    WMask {
        d: u16,
        w: u16,
    },
    /// Bank moves for Resize crossings (same register index).
    WFromN {
        r: u16,
    },
    NFromW {
        r: u16,
        mask: u64,
    },
    /// Wide writeback with change-detect + dirty hooks (the register is
    /// masked to its width by construction; stored via Value::set_words).
    WStore {
        sig: u32,
        s: u16,
    },
    WStoreNba {
        sig: u32,
        s: u16,
        w: u32,
    },
    /// `sig[lo+w-1:lo] = wregs[s]` / `<=`: a whole wide (65..=512-bit)
    /// register written into a window of a wider destination (the
    /// `bus[127:0] = data` arms of the c906 load/store arbiters).
    WRangeStore {
        sig: u32,
        lo: u32,
        s: u16,
        w: u32,
    },
    WRangeStoreNba {
        sig: u32,
        lo: u32,
        s: u16,
        w: u32,
    },
    /// Wrapping multiply at max operand width (both operands unsigned).
    Mul {
        d: u16,
        a: u16,
        b: u16,
        mask: u64,
    },
    /// Wide (65..=512-bit) slice of a SIGNAL into the wide bank; bails
    /// when any bit of the slice holds X/Z.
    WSigRange {
        d: u16,
        sig: u32,
        lo: u16,
        w: u16,
    },
    /// Logical shifts. `w` is the LEFT operand's width, which is also the
    /// result width (`Value::shift_left`/`shift_right` keep `self.width`);
    /// an amount ≥ w yields 0, matching those helpers exactly. The amount
    /// register is X-free by construction, so the 4-state all-X result for
    /// an X amount cannot arise here.
    /// regs[d] = (regs[s] + k) & mask — the fused constant add. Counters
    /// and address increments make this the most common shape in clocked
    /// bodies, which the islands cover since the edge hook landed.
    AddC {
        d: u16,
        s: u16,
        k: u64,
        mask: u64,
    },
    Shl {
        d: u16,
        a: u16,
        b: u16,
        w: u32,
        mask: u64,
    },
    Shr {
        d: u16,
        a: u16,
        b: u16,
        w: u32,
    },
    /// Narrow replicate: dst = {count{src}} with count*w ≤ 64.
    Repl {
        d: u16,
        s: u16,
        w: u8,
        count: u8,
    },
    /// Wide replicate (result 65..=512 bits) of a NARROW part.
    WRepl {
        d: u16,
        s: u16,
        w: u8,
        count: u16,
    },
    /// One statement the bytecode compiler could not compile (a `$display`
    /// in a check's failing branch, typically), run by the AST interpreter
    /// in place. Before this, any such statement kept the WHOLE block on the
    /// four-state VM at ~150 host instructions per bytecode instruction,
    /// even when the fallback sat on a branch that never executes.
    Fallback(Arc<Statement>),
    /// Save the signal's current bits; the executor restores every saved
    /// signal when the block bails, so the four-state re-run is exact for
    /// a block that reads a signal it later overwrites.
    SaveSig {
        sig: u32,
    },
    /// Process-FSM wait points (only in FSM streams, run by the control
    /// executor from a start index). `resume` is the four-state pc after
    /// the wait, which the runner keeps as the process's pc; `vm_to_ts`
    /// maps it back to a stream index on the next resume.
    WaitEdge {
        ix: u32,
        resume: u32,
    },
    /// A folded constant delay: `raw` is the f64 bit pattern of the tick
    /// count before quantization (`fsm_delay_ticks` applies it at run time).
    WaitDelayRaw {
        raw: u64,
        resume: u32,
    },
    /// `Insn::CovHit`; undone on a bail like a hazard save.
    CovHit(u32),
}

#[derive(serde::Serialize, serde::Deserialize)]

pub struct TwoStateBlock {
    pub insns: Vec<TsInsn>,
    pub num_regs: u32,
    /// Deduped ≤64-bit signals read WHOLE — checked X-free with one
    /// raw_bits call each.
    pub reads_whole: Box<[u32]>,
    /// Deduped (signal, lo, width) slices of WIDE signals — checked via
    /// raw_bits_slice, so unrelated X bits don't force a bail.
    pub reads_slice: Box<[(u32, u16, u16)]>,
    /// Any branch/jump present. Straight-line blocks (the overwhelmingly
    /// common comb shape) run a compact loop without the pc bookkeeping.
    pub has_ctrl: bool,
    /// Any wide (65..=512-bit) op present — routed to the wide executor so
    /// the narrow hot loops keep their code size.
    pub has_wide: bool,
    /// Words per wide register for this block: 0 (none), 2 (widths up to
    /// 128) or 8 (up to 512); selects the wide executor's monomorph.
    pub wide_words: u8,
    /// Process-FSM streams only: the stream holds wait opcodes; `vm_to_ts`
    /// maps each four-state pc to its stream index; `wait_regs` lists, per
    /// resume pc, the registers the stream defines there as (reg, width,
    /// signed) — the set a VM re-run converts from and to.
    pub has_wait: bool,
    /// Every instruction has an x-plane form and the stream needs no
    /// wide bank: the x-plane executor can run it after an x-read bail.
    pub tsx: bool,
    pub vm_to_ts: Box<[u32]>,
    pub wait_regs: Box<[(u32, Box<[(u16, u32, bool)]>)]>,
    /// Wide (>64-bit) signals read WHOLE — X-checked via words_if_clean.
    pub reads_wide: Box<[u32]>,
    /// Signals this block WRITES (sorted, deduped). §9.3.1 force filtering
    /// is per DESTINATION — `write_sig!` drops writes to forced signals and
    /// the two-state stores bypass that check — so an active force only
    /// makes a block unsafe when the block writes a forced signal. Reads of
    /// a forced signal are exact (the forced value lives in the table).
    pub writes: Box<[u32]>,
    /// Array element-store spans `(first_id, count)`: a dynamic element
    /// write can land on any id in the span, so a forced id inside one
    /// disqualifies the block.
    pub writes_span: Box<[(u32, u32)]>,
}

/// Can the x-plane executor run this instruction? Everything except the
/// wide bank, process waits and AST fallbacks.
pub fn tsx_insn_ok(i: &TsInsn) -> bool {
    !matches!(
        i,
        TsInsn::Fallback(..)
            | TsInsn::WaitEdge { .. }
            | TsInsn::WaitDelayRaw { .. }
            | TsInsn::WRedOr { .. }
            | TsInsn::WRedAnd { .. }
            | TsInsn::WSel { .. }
            | TsInsn::WLoadSig { .. }
            | TsInsn::WConst { .. }
            | TsInsn::WXor { .. }
            | TsInsn::WAnd { .. }
            | TsInsn::WOr { .. }
            | TsInsn::WNot { .. }
            | TsInsn::WRange { .. }
            | TsInsn::RangeFromW { .. }
            | TsInsn::BitFromW { .. }
            | TsInsn::WConcat { .. }
            | TsInsn::WMask { .. }
            | TsInsn::WFromN { .. }
            | TsInsn::NFromW { .. }
            | TsInsn::WStore { .. }
            | TsInsn::WStoreNba { .. }
            | TsInsn::WRangeStore { .. }
            | TsInsn::WRangeStoreNba { .. }
            | TsInsn::WSigRange { .. }
            | TsInsn::WRepl { .. }
            | TsInsn::WElemLoad(..)
    )
}

fn ts_mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

/// Sensitivity mask of bits `lo .. lo + n` of a `width`-bit signal. A
/// signal of up to 64 bits maps bit for bit; a wider one maps mask bit `k`
/// to the `2^s`-bit chunk starting at `k << s`, `s` the smallest shift that
/// fits the signal in 64 chunks, so a span covers every chunk it touches.
/// (Bits past chunk 63 — only out-of-range spans — all map to chunk 63,
/// the same way for reads and writes.) An empty span claims the whole
/// signal.
#[inline(always)]
pub fn sig_span_mask(width: u32, lo: u32, n: u32) -> u64 {
    if n == 0 {
        return u64::MAX;
    }
    let sh = sig_chunk_shift(width);
    let first = (lo >> sh).min(63);
    let last = (lo.saturating_add(n - 1) >> sh).min(63);
    (u64::MAX >> (63 - last)) & (u64::MAX << first)
}

/// log2 of the chunk size `sig_span_mask` uses for a `width`-bit signal.
#[inline(always)]
pub fn sig_chunk_shift(width: u32) -> u32 {
    if width <= 64 {
        0
    } else {
        // ceil(log2(ceil(width / 64)))
        let c = width.div_ceil(64);
        32 - (c - 1).leading_zeros()
    }
}

/// Bits read by a two-state stream, per signal, for bit-granular comb
/// sensitivity: `(signal, mask)` with `sig_span_mask` chunks (`u64::MAX`
/// for a whole-signal read), plus every signal the stream stores, once per
/// store instruction. None when the stream holds anything whose reads are
/// not a fixed set of bits: array elements, AST fallbacks, waits, coverage
/// hits (an evaluation count is observable). The match is exhaustive on
/// purpose — a new instruction must be classified before it compiles.
pub fn ts_read_masks(insns: &[TsInsn], widths: &[u32]) -> Option<(Vec<(u32, u64)>, Vec<u32>)> {
    let mut reads: Vec<(u32, u64)> = Vec::new();
    let mut writes: Vec<u32> = Vec::new();
    let wid = |sig: u32| widths.get(sig as usize).copied().unwrap_or(0);
    let add = |reads: &mut Vec<(u32, u64)>, sig: u32, m: u64| {
        if let Some(e) = reads.iter_mut().find(|e| e.0 == sig) {
            e.1 |= m;
        } else {
            reads.push((sig, m));
        }
    };
    // Low contiguous `mask` → its bit count (0 claims the whole signal).
    let nbits = |mask: u64| 64 - mask.leading_zeros();
    for i in insns {
        match i {
            TsInsn::LoadSig { sig, .. }
            | TsInsn::LoadSigNot { sig, .. }
            | TsInsn::LoadSigLogAnd { sig, .. }
            | TsInsn::LoadSigBrNz { sig, .. }
            | TsInsn::BrFalseLoadSig { sig, .. }
            | TsInsn::LoadSigLogOr { sig, .. }
            | TsInsn::LoadSigAnd { sig, .. }
            | TsInsn::LoadSigRepl { sig, .. }
            | TsInsn::WLoadSig { sig, .. }
            | TsInsn::SigRangeDyn { sig, .. } => add(&mut reads, *sig, u64::MAX),
            TsInsn::LoadSig2 { sig1, sig2, .. } => {
                add(&mut reads, *sig1, u64::MAX);
                add(&mut reads, *sig2, u64::MAX);
            }
            TsInsn::SigBit { sig, bit, .. }
            | TsInsn::SigBitW { sig, bit, .. }
            | TsInsn::SigBitNot { sig, bit, .. } => {
                add(&mut reads, *sig, sig_span_mask(wid(*sig), *bit as u32, 1))
            }
            TsInsn::SigBit2 {
                sig1,
                bit1,
                sig2,
                bit2,
                ..
            } => {
                add(
                    &mut reads,
                    *sig1,
                    sig_span_mask(wid(*sig1), *bit1 as u32, 1),
                );
                add(
                    &mut reads,
                    *sig2,
                    sig_span_mask(wid(*sig2), *bit2 as u32, 1),
                );
            }
            TsInsn::SigRange { sig, lo, mask, .. }
            | TsInsn::SigRangeAnd { sig, lo, mask, .. }
            | TsInsn::SigRangeEq { sig, lo, mask, .. } => add(
                &mut reads,
                *sig,
                sig_span_mask(wid(*sig), *lo as u32, nbits(*mask)),
            ),
            TsInsn::SigRangeW { sig, lo, w, .. } | TsInsn::WSigRange { sig, lo, w, .. } => add(
                &mut reads,
                *sig,
                sig_span_mask(wid(*sig), *lo as u32, *w as u32),
            ),
            TsInsn::SigRangeEqC { sig, lo, w, .. } => add(
                &mut reads,
                *sig,
                sig_span_mask(wid(*sig), *lo as u32, *w as u32),
            ),
            TsInsn::LoadSigSigRange {
                sig,
                sig2,
                lo,
                mask,
                ..
            } => {
                add(&mut reads, *sig, u64::MAX);
                add(
                    &mut reads,
                    *sig2,
                    sig_span_mask(wid(*sig2), *lo as u32, nbits(*mask)),
                );
            }
            TsInsn::BrSigFalse { sig, bit, .. } => {
                let m = if *bit == u32::MAX {
                    u64::MAX
                } else {
                    sig_span_mask(wid(*sig), *bit, 1)
                };
                add(&mut reads, *sig, m);
            }
            TsInsn::LogAndStore { sig, .. }
            | TsInsn::AndRangeStore { sig, .. }
            | TsInsn::LogOrStore { sig, .. }
            | TsInsn::OrRangeStore { sig, .. }
            | TsInsn::BitStoreNbaDyn { sig, .. }
            | TsInsn::BitStoreDyn { sig, .. }
            | TsInsn::RangeStoreDyn { sig, .. }
            | TsInsn::RangeStoreNbaDyn { sig, .. }
            | TsInsn::RangeFillXW { sig, .. }
            | TsInsn::ConstStoreX { sig, .. }
            | TsInsn::Store { sig, .. }
            | TsInsn::StoreNba { sig, .. }
            | TsInsn::ConstStoreNba { sig, .. }
            | TsInsn::RangeStoreNba { sig, .. }
            | TsInsn::RangeStore { sig, .. }
            | TsInsn::RangeStoreW { sig, .. }
            | TsInsn::RangeStoreNbaW { sig, .. }
            | TsInsn::RangeFillW { sig, .. }
            | TsInsn::RangeFillNbaW { sig, .. }
            | TsInsn::WStore { sig, .. }
            | TsInsn::WStoreNba { sig, .. }
            | TsInsn::WRangeStore { sig, .. }
            | TsInsn::WRangeStoreNba { sig, .. } => writes.push(*sig),
            TsInsn::RangeStoreX(p) | TsInsn::RangeStoreXW(p) => writes.push(p.sig),
            // A save snapshots a signal the stream is about to store, for
            // the undo after a bail: not an input.
            TsInsn::SaveSig { .. } | TsInsn::SaveSigW { .. } => {}
            TsInsn::ElemLoad(..)
            | TsInsn::WElemLoad(..)
            | TsInsn::ElemStoreNba(..)
            | TsInsn::ElemStoreNbaFromSig(..)
            | TsInsn::ElemStore(..)
            | TsInsn::NbaFromElem(..)
            | TsInsn::WNbaFromElem(..)
            | TsInsn::Fallback(..)
            | TsInsn::WaitEdge { .. }
            | TsInsn::WaitDelayRaw { .. }
            | TsInsn::CovHit(..) => return None,
            TsInsn::Const { .. }
            | TsInsn::Bit { .. }
            | TsInsn::Range { .. }
            | TsInsn::Xor { .. }
            | TsInsn::And { .. }
            | TsInsn::Or { .. }
            | TsInsn::Sel { .. }
            | TsInsn::Not { .. }
            | TsInsn::XorC { .. }
            | TsInsn::EqC { .. }
            | TsInsn::Add { .. }
            | TsInsn::Sub { .. }
            | TsInsn::Eq { .. }
            | TsInsn::MaskEq { .. }
            | TsInsn::Neq { .. }
            | TsInsn::LogNotAnd { .. }
            | TsInsn::LogNotLogAnd { .. }
            | TsInsn::EqBrFalse { .. }
            | TsInsn::EqCBrFalse { .. }
            | TsInsn::ConstEq { .. }
            | TsInsn::AndOr { .. }
            | TsInsn::Lt { .. }
            | TsInsn::CmpS { .. }
            | TsInsn::BitDyn { .. }
            | TsInsn::Leq { .. }
            | TsInsn::Gt { .. }
            | TsInsn::Geq { .. }
            | TsInsn::LogNot { .. }
            | TsInsn::LogAnd { .. }
            | TsInsn::LogOr { .. }
            | TsInsn::Concat { .. }
            | TsInsn::Concat2 { .. }
            | TsInsn::Concat3 { .. }
            | TsInsn::Mask { .. }
            | TsInsn::BrFalse { .. }
            | TsInsn::BrNz { .. }
            | TsInsn::Jmp { .. }
            | TsInsn::JmpPoll { .. }
            | TsInsn::LoopPoll
            | TsInsn::CaseJmp { .. }
            | TsInsn::CaseMaskJmp { .. }
            | TsInsn::RedOr { .. }
            | TsInsn::WRedOr { .. }
            | TsInsn::WRedAnd { .. }
            | TsInsn::RedAnd { .. }
            | TsInsn::WSel { .. }
            | TsInsn::WConst { .. }
            | TsInsn::WXor { .. }
            | TsInsn::WAnd { .. }
            | TsInsn::WOr { .. }
            | TsInsn::WNot { .. }
            | TsInsn::WRange { .. }
            | TsInsn::RangeFromW { .. }
            | TsInsn::BitFromW { .. }
            | TsInsn::WConcat { .. }
            | TsInsn::WMask { .. }
            | TsInsn::WFromN { .. }
            | TsInsn::NFromW { .. }
            | TsInsn::Mul { .. }
            | TsInsn::AddC { .. }
            | TsInsn::Shl { .. }
            | TsInsn::Shr { .. }
            | TsInsn::Repl { .. }
            | TsInsn::WRepl { .. } => {}
        }
    }
    Some((reads, writes))
}

thread_local! {
    /// Opcode the lowering was on when it gave up — the only way to answer
    /// "why is this block not an island?" on a design you cannot read.
    /// Reported by the XZ_TS_DBG dump.
    static TS_BAIL_AT: std::cell::Cell<(usize, &'static str)> =
        const { std::cell::Cell::new((0, "-")) };
}

/// (stream index, opcode) where the last `lower_two_state` call stopped.
pub fn ts_last_bail() -> (usize, &'static str) {
    TS_BAIL_AT.with(|c| c.get())
}

thread_local! {
    /// Fine-grained reason for the last bail, for arms that carry several
    /// distinct gates. The opcode alone was not enough: the gate census
    /// showed `BlockingAssignRange` accounting for 58% of all interpreter
    /// evaluations on the C906 SoC while its DESTINATION passed every
    /// signal-level check, so the deciding condition had to be named from
    /// inside the arm.
    static TS_GATE_WHY: std::cell::Cell<&'static str> =
        const { std::cell::Cell::new("-") };
}

/// Fine-grained gate label for the last `lower_two_state` bail ("-" when the
/// arm does not carry one).
pub fn ts_last_gate() -> &'static str {
    TS_GATE_WHY.with(|c| c.get())
}

/// Does the block read a signal (or array) BEFORE writing it, on some path,
/// with no write of that signal earlier on that same path?
///
/// Every two-state bail — an x-read, an out-of-range element index — hands
/// the block to the four-state interpreter, which re-runs it from the top
/// against the signal table as the two-state stores left it. That re-run
/// reproduces the aborted run exactly iff no load observed a signal the
/// block itself overwrites later on the same path: `cnt = cnt + 1; ...`
/// followed by a bail would increment twice. Loop counters (`i = 0; ...
/// i = i + 1`) are fine — every read of `i` is dominated by a store.
///
/// Two small dataflow passes over the block's CFG: MUST-stored (intersection
/// over predecessors) protects a read; a read of an unprotected signal is
/// recorded (union), and a later blocking store of a recorded signal is the
/// hazard. Non-blocking assignments queue and never touch the table.
/// Returns the signals whose read-after-write makes a re-run inexact
/// (`Ok(empty)` = none), or `Err` when a hazard sits on an array or the
/// dataflow did not converge — those blocks cannot be saved and restored.
fn ts_raw_hazard(
    insns: &[Insn],
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Result<Vec<u32>, ()> {
    #[derive(PartialEq, Eq, Hash, Clone)]
    enum Base {
        Sig(u32),
        Arr(usize),
        ArrName(String),
    }
    // One access site class: base + bit range. `protects` is false for a
    // store whose bits are not static (dynamic bit / array element), which
    // therefore shields no later read.
    struct Acc {
        base: Base,
        lo: u32,
        hi: u32,
        protects: bool,
    }
    const FULL: (u32, u32) = (0, u32::MAX);
    let arr_base = |a: &ArrayOperand| -> Base {
        match a {
            ArrayOperand::Dense { first_id, .. } => Base::Arr(*first_id),
            ArrayOperand::Named(name) => match array_first_id.get(name.as_str()) {
                Some(&(first, _, _)) => Base::Arr(first),
                None => Base::ArrName(name.clone()),
            },
        }
    };
    let n = insns.len();
    let mut loads: Vec<Acc> = Vec::new();
    let mut stores: Vec<Acc> = Vec::new();
    let mut reads: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut writes: Vec<Option<usize>> = vec![None; n];
    type AccKey = (Base, u32, u32, bool);
    // Linear for the usual handful of access classes; past that through an
    // index (an unrolled reduction loads one signal per trip), which covers
    // a prefix of the append-only list.
    fn intern(v: &mut Vec<Acc>, ix: &mut HashMap<AccKey, usize>, a: Acc) -> usize {
        let found = if v.len() < 16 {
            v.iter().position(|x| {
                x.base == a.base && x.lo == a.lo && x.hi == a.hi && x.protects == a.protects
            })
        } else {
            for (i, x) in v.iter().enumerate().skip(ix.len()) {
                ix.entry((x.base.clone(), x.lo, x.hi, x.protects))
                    .or_insert(i);
            }
            ix.get(&(a.base.clone(), a.lo, a.hi, a.protects)).copied()
        };
        match found {
            Some(i) => i,
            None => {
                v.push(a);
                v.len() - 1
            }
        }
    }
    let mut loads_ix: HashMap<AccKey, usize> = HashMap::default();
    let mut stores_ix: HashMap<AccKey, usize> = HashMap::default();
    let rng = |a: u32, b: u32| -> (u32, u32) { (a.min(b), a.max(b)) };
    for (i, insn) in insns.iter().enumerate() {
        let mut ld = |base: Base, r: (u32, u32)| {
            reads[i].push(intern(
                &mut loads,
                &mut loads_ix,
                Acc {
                    base,
                    lo: r.0,
                    hi: r.1,
                    protects: false,
                },
            ));
        };
        match insn {
            Insn::LoadSignal(_, sig) | Insn::LoadSignalSigned(_, sig) => ld(Base::Sig(*sig), FULL),
            Insn::LoadSignalBit(_, sig, b) => ld(Base::Sig(*sig), (*b, *b)),
            Insn::LoadSignalRange(_, sig, l, r) => ld(Base::Sig(*sig), rng(*l, *r)),
            Insn::BranchIfSignalFalse(sig, _, b) => ld(
                Base::Sig(*sig),
                if *b == u32::MAX { FULL } else { (*b, *b) },
            ),
            Insn::LoadArrayElem(_, arr, _) => ld(arr_base(arr), FULL),
            Insn::NbaAssignArrayRead(_, arr, isig, _) => {
                ld(arr_base(arr), FULL);
                ld(Base::Sig(*isig), FULL);
            }
            _ => {}
        }
        let st = match insn {
            Insn::BlockingAssign(sig, _, _) | Insn::BlockingAssignString(sig, _) => Some(Acc {
                base: Base::Sig(*sig),
                lo: 0,
                hi: u32::MAX,
                protects: true,
            }),
            Insn::BlockingAssignRange(sig, hi, lo, _) => {
                let r = rng(*hi, *lo);
                Some(Acc {
                    base: Base::Sig(*sig),
                    lo: r.0,
                    hi: r.1,
                    protects: true,
                })
            }
            Insn::BlockingAssignBitDyn(sig, _, _) => Some(Acc {
                base: Base::Sig(*sig),
                lo: 0,
                hi: u32::MAX,
                protects: false,
            }),
            Insn::BlockingAssignArray(arr, _, _, _)
            | Insn::BlockingAssignArrayRange(arr, _, _, _, _) => Some(Acc {
                base: arr_base(arr),
                lo: 0,
                hi: u32::MAX,
                protects: false,
            }),
            _ => None,
        };
        if let Some(a) = st {
            writes[i] = Some(intern(&mut stores, &mut stores_ix, a));
        }
    }
    if stores.is_empty() || loads.is_empty() {
        return Ok(Vec::new());
    }
    let (nl, ns) = (loads.len(), stores.len());
    if nl > 4096 || ns > 4096 {
        return Err(());
    }
    // covers[L]: store classes whose static range contains load class L.
    // overlaps[S]: load classes whose range meets store class S.
    let covers: Vec<Vec<usize>> = loads
        .iter()
        .map(|l| {
            stores
                .iter()
                .enumerate()
                .filter(|(_, s)| s.protects && s.base == l.base && s.lo <= l.lo && s.hi >= l.hi)
                .map(|(j, _)| j)
                .collect()
        })
        .collect();
    let overlaps: Vec<Vec<usize>> = stores
        .iter()
        .map(|s| {
            loads
                .iter()
                .enumerate()
                .filter(|(_, l)| l.base == s.base && l.lo <= s.hi && s.lo <= l.hi)
                .map(|(j, _)| j)
                .collect()
        })
        .collect();
    let lw = nl.div_ceil(64);
    let sw = ns.div_ceil(64);
    let succs = |i: usize| -> Vec<usize> {
        match &insns[i] {
            Insn::Jump(t) => vec![*t as usize],
            Insn::BranchIfFalse(_, t)
            | Insn::BranchUnlessZero(_, t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::CmpBranchConst(_, _, _, _, t) => vec![i + 1, *t as usize],
            Insn::CaseJump(_, cj) => cj
                .table
                .iter()
                .chain(std::iter::once(&cj.default))
                .map(|&t| t as usize)
                .collect(),
            Insn::CaseMaskJump(_, mj) => mj
                .table
                .iter()
                .chain(std::iter::once(&mj.xz_path))
                .map(|&t| t as usize)
                .collect(),
            _ => vec![i + 1],
        }
    };
    let bit = |v: &[u64], k: usize| (v[k >> 6] >> (k & 63)) & 1 != 0;
    // must_in over store classes (intersection; top = all-ones except the
    // entry), unprot_in over load classes (union).
    let mut must_in: Vec<Vec<u64>> = vec![vec![u64::MAX; sw]; n + 1];
    must_in[0] = vec![0; sw];
    let mut unprot_in: Vec<Vec<u64>> = vec![vec![0; lw]; n + 1];
    let mut must_out: Vec<Vec<u64>> = vec![vec![u64::MAX; sw]; n];
    let mut unprot_out: Vec<Vec<u64>> = vec![vec![0; lw]; n];
    let mut hazards: Vec<u32> = Vec::new();
    let mut changed = true;
    let mut sweeps = 0;
    while changed {
        changed = false;
        sweeps += 1;
        if sweeps > 64 {
            return Err(());
        }
        for i in 0..n {
            let mut mo = must_in[i].clone();
            let mut uo = unprot_in[i].clone();
            for &l in &reads[i] {
                let protected = covers[l].iter().any(|&sidx| bit(&must_in[i], sidx));
                if !protected {
                    uo[l >> 6] |= 1u64 << (l & 63);
                }
            }
            if let Some(sidx) = writes[i] {
                if overlaps[sidx].iter().any(|&l| bit(&unprot_in[i], l)) {
                    match stores[sidx].base {
                        Base::Sig(sg) => {
                            if !hazards.contains(&sg) {
                                hazards.push(sg);
                            }
                        }
                        _ => return Err(()),
                    }
                }
                mo[sidx >> 6] |= 1u64 << (sidx & 63);
            }
            if mo != must_out[i] || uo != unprot_out[i] {
                changed = true;
                must_out[i] = mo;
                unprot_out[i] = uo;
            }
            for t in succs(i) {
                if t > n {
                    continue;
                }
                let mut ch = false;
                for w in 0..sw {
                    let nm = must_in[t][w] & must_out[i][w];
                    if nm != must_in[t][w] {
                        must_in[t][w] = nm;
                        ch = true;
                    }
                }
                for w in 0..lw {
                    let nu = unprot_in[t][w] | unprot_out[i][w];
                    if nu != unprot_in[t][w] {
                        unprot_in[t][w] = nu;
                        ch = true;
                    }
                }
                if ch {
                    changed = true;
                }
            }
        }
    }
    Ok(hazards)
}

/// Peephole over a lowered stream: the eight adjacent opcode pairs that a
/// c906 census put at 17% of all two-state instructions collapse into one
/// fused instruction each. A pair is only fused when its second half is not
/// a branch target; every branch target is then remapped.
pub(super) fn fuse_ts_pairs(out: &mut Vec<TsInsn>) {
    let n = out.len();
    if n < 2 {
        return;
    }
    let mut is_tgt = vec![false; n + 1];
    for i in out.iter() {
        match i {
            TsInsn::BrSigFalse { t, .. }
            | TsInsn::BrFalse { t, .. }
            | TsInsn::BrNz { t, .. }
            | TsInsn::EqCBrFalse { t, .. }
            | TsInsn::Jmp { t } => {
                if (*t as usize) <= n {
                    is_tgt[*t as usize] = true;
                }
            }
            TsInsn::CaseMaskJmp { mj, .. } => {
                let mj = &mj.mj;
                for t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                    if (*t as usize) <= n {
                        is_tgt[*t as usize] = true;
                    }
                }
            }
            TsInsn::CaseJmp { cj, .. } => {
                for t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                    if (*t as usize) <= n {
                        is_tgt[*t as usize] = true;
                    }
                }
            }
            _ => {}
        }
    }
    let mut fused: Vec<TsInsn> = Vec::with_capacity(n);
    let mut new_of: Vec<u32> = vec![0; n + 1];
    let mut i = 0;
    let mut any = false;
    while i < n {
        new_of[i] = fused.len() as u32;
        // Three-instruction form: `LoadSig(data) ; LoadSig(index) ;
        // ElemStoreNba { s: data, idx: index }` (the lowering emits the data
        // load first). The data load folds into the x-tolerant
        // `ElemStoreNbaFromSig`; the index load stays. Checked before the
        // pair rules so the two-load merge does not pre-empt it.
        if i + 2 < n && !is_tgt[i + 1] && !is_tgt[i + 2] {
            if let (
                TsInsn::LoadSig { d: d1, sig: sig1 },
                TsInsn::LoadSig { d: d2, sig: sig2 },
                TsInsn::ElemStoreNba(op),
            ) = (&out[i], &out[i + 1], &out[i + 2])
            {
                if op.s == *d1 && op.idx == *d2 {
                    new_of[i + 1] = fused.len() as u32;
                    fused.push(TsInsn::LoadSig { d: *d2, sig: *sig2 });
                    new_of[i + 2] = fused.len() as u32;
                    fused.push(TsInsn::ElemStoreNbaFromSig(Box::new(TsElemStoreFromSig {
                        sig: *sig1,
                        dl: *d1,
                        op: (**op).clone(),
                    })));
                    any = true;
                    i += 3;
                    continue;
                }
            }
        }
        let f = if i + 1 < n && !is_tgt[i + 1] {
            match (&out[i], &out[i + 1]) {
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::ElemStoreNba(op)) if op.s == *dl => {
                    Some(TsInsn::ElemStoreNbaFromSig(Box::new(TsElemStoreFromSig {
                        sig: *sig,
                        dl: *dl,
                        op: (**op).clone(),
                    })))
                }
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::LogNot { d, s }) if s == dl => {
                    Some(TsInsn::LoadSigNot {
                        dl: *dl,
                        d: *d,
                        sig: *sig,
                    })
                }
                (
                    TsInsn::SigRange {
                        d: dr,
                        sig,
                        lo,
                        mask,
                    },
                    TsInsn::EqC { d, s, k },
                ) if s == dr
                    && *lo < 64
                    && mask.count_ones() as u64 == 64 - mask.leading_zeros() as u64 =>
                {
                    Some(TsInsn::SigRangeEqC {
                        dr: *dr,
                        d: *d,
                        sig: *sig,
                        lo: *lo as u8,
                        w: (64 - mask.leading_zeros()) as u8,
                        k: *k,
                    })
                }
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::LogAnd { d, a, b })
                    if a == dl || b == dl =>
                {
                    Some(TsInsn::LoadSigLogAnd {
                        dl: *dl,
                        sig: *sig,
                        d: *d,
                        a: *a,
                        b: *b,
                    })
                }
                (TsInsn::LogNot { d: dn, s }, TsInsn::And { d, a, b }) if a == dn || b == dn => {
                    Some(TsInsn::LogNotAnd {
                        dn: *dn,
                        s: *s,
                        d: *d,
                        a: *a,
                        b: *b,
                    })
                }
                (TsInsn::LogNot { d: dn, s }, TsInsn::LogAnd { d, a, b }) if a == dn || b == dn => {
                    Some(TsInsn::LogNotLogAnd {
                        dn: *dn,
                        s: *s,
                        d: *d,
                        a: *a,
                        b: *b,
                    })
                }
                (TsInsn::LogAnd { d, a, b }, TsInsn::Store { sig, s, mask }) if s == d => {
                    Some(TsInsn::LogAndStore {
                        d: *d,
                        a: *a,
                        b: *b,
                        sig: *sig,
                        mask: *mask,
                    })
                }
                (
                    TsInsn::And { d, a, b },
                    TsInsn::RangeStore {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    },
                ) if s == d && *mask == ts_mask(*hi - *lo + 1) => Some(TsInsn::AndRangeStore {
                    d: *d,
                    a: *a,
                    b: *b,
                    sig: *sig,
                    hi: *hi,
                    lo: *lo,
                }),
                (TsInsn::SigBit { d: db, sig, bit }, TsInsn::LogNot { d, s }) if s == db => {
                    Some(TsInsn::SigBitNot {
                        db: *db,
                        d: *d,
                        sig: *sig,
                        bit: *bit,
                    })
                }
                (TsInsn::LoadSig { d: d1, sig: sig1 }, TsInsn::LoadSig { d: d2, sig: sig2 }) => {
                    Some(TsInsn::LoadSig2 {
                        d1: *d1,
                        sig1: *sig1,
                        d2: *d2,
                        sig2: *sig2,
                    })
                }
                (
                    TsInsn::SigBit {
                        d: d1,
                        sig: sig1,
                        bit: bit1,
                    },
                    TsInsn::SigBit {
                        d: d2,
                        sig: sig2,
                        bit: bit2,
                    },
                ) => Some(TsInsn::SigBit2 {
                    d1: *d1,
                    sig1: *sig1,
                    bit1: *bit1,
                    d2: *d2,
                    sig2: *sig2,
                    bit2: *bit2,
                }),
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::BrNz { s, t }) if s == dl => {
                    Some(TsInsn::LoadSigBrNz {
                        dl: *dl,
                        sig: *sig,
                        t: *t,
                    })
                }
                (TsInsn::BrFalse { s, t }, TsInsn::LoadSig { d: dl, sig }) => {
                    Some(TsInsn::BrFalseLoadSig {
                        s: *s,
                        t: *t,
                        dl: *dl,
                        sig: *sig,
                    })
                }
                (TsInsn::Eq { d, a, b }, TsInsn::BrFalse { s, t }) if s == d => {
                    Some(TsInsn::EqBrFalse {
                        d: *d,
                        a: *a,
                        b: *b,
                        t: *t,
                    })
                }
                (TsInsn::EqC { d, s, k }, TsInsn::BrFalse { s: condition, t })
                    if condition == d =>
                {
                    Some(TsInsn::EqCBrFalse {
                        d: *d,
                        s: *s,
                        k: *k,
                        t: *t,
                    })
                }
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::LogOr { d, a, b })
                    if a == dl || b == dl =>
                {
                    Some(TsInsn::LoadSigLogOr {
                        dl: *dl,
                        sig: *sig,
                        d: *d,
                        a: *a,
                        b: *b,
                    })
                }
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::And { d, a, b }) if a == dl || b == dl => {
                    Some(TsInsn::LoadSigAnd {
                        dl: *dl,
                        sig: *sig,
                        d: *d,
                        a: *a,
                        b: *b,
                    })
                }
                (TsInsn::LoadSig { d: dl, sig }, TsInsn::Repl { d, s, w, count }) if s == dl => {
                    Some(TsInsn::LoadSigRepl {
                        dl: *dl,
                        sig: *sig,
                        d: *d,
                        w: *w,
                        count: *count,
                    })
                }
                (
                    TsInsn::LoadSig { d: dl, sig },
                    TsInsn::SigRange {
                        d,
                        sig: sig2,
                        lo,
                        mask,
                    },
                ) => Some(TsInsn::LoadSigSigRange {
                    dl: *dl,
                    sig: *sig,
                    d: *d,
                    sig2: *sig2,
                    lo: *lo,
                    mask: *mask,
                }),
                (
                    TsInsn::SigRange {
                        d: dr,
                        sig,
                        lo,
                        mask,
                    },
                    TsInsn::And { d, a, b },
                ) if a == dr || b == dr => Some(TsInsn::SigRangeAnd {
                    dr: *dr,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                    d: *d,
                    a: *a,
                    b: *b,
                }),
                (
                    TsInsn::SigRange {
                        d: dr,
                        sig,
                        lo,
                        mask,
                    },
                    TsInsn::Eq { d, a, b },
                ) if a == dr || b == dr => Some(TsInsn::SigRangeEq {
                    dr: *dr,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                    d: *d,
                    a: *a,
                    b: *b,
                }),
                (TsInsn::Const { d: dc, v }, TsInsn::Eq { d, a, b }) if a == dc || b == dc => {
                    Some(TsInsn::ConstEq {
                        dc: *dc,
                        v: *v,
                        d: *d,
                        a: *a,
                        b: *b,
                    })
                }
                (TsInsn::LogOr { d, a, b }, TsInsn::Store { sig, s, mask }) if s == d => {
                    Some(TsInsn::LogOrStore {
                        d: *d,
                        a: *a,
                        b: *b,
                        sig: *sig,
                        mask: *mask,
                    })
                }
                (
                    TsInsn::And {
                        d: d1,
                        a: a1,
                        b: b1,
                    },
                    TsInsn::Or { d, a, b },
                ) if a == d1 || b == d1 => Some(TsInsn::AndOr {
                    d1: *d1,
                    a1: *a1,
                    b1: *b1,
                    d: *d,
                    a: *a,
                    b: *b,
                }),
                (
                    TsInsn::Or { d, a, b },
                    TsInsn::RangeStore {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    },
                ) if s == d && *mask == ts_mask(*hi - *lo + 1) => Some(TsInsn::OrRangeStore {
                    d: *d,
                    a: *a,
                    b: *b,
                    sig: *sig,
                    hi: *hi,
                    lo: *lo,
                }),
                _ => None,
            }
        } else {
            None
        };
        if let Some(f) = f {
            new_of[i + 1] = fused.len() as u32;
            fused.push(f);
            any = true;
            i += 2;
        } else {
            fused.push(out[i].clone());
            i += 1;
        }
    }
    if !any {
        return;
    }
    new_of[n] = fused.len() as u32;
    for insn in fused.iter_mut() {
        match insn {
            TsInsn::BrSigFalse { t, .. }
            | TsInsn::BrFalse { t, .. }
            | TsInsn::BrNz { t, .. }
            | TsInsn::LoadSigBrNz { t, .. }
            | TsInsn::BrFalseLoadSig { t, .. }
            | TsInsn::EqBrFalse { t, .. }
            | TsInsn::EqCBrFalse { t, .. }
            | TsInsn::Jmp { t } => *t = new_of[(*t as usize).min(n)],
            TsInsn::CaseMaskJmp { mj, .. } => {
                let mj = &mut mj.mj;
                for t in mj.table.iter_mut().chain(std::iter::once(&mut mj.xz_path)) {
                    *t = new_of[(*t as usize).min(n)];
                }
            }
            TsInsn::CaseJmp { cj, .. } => {
                for t in cj.table.iter_mut().chain(std::iter::once(&mut cj.default)) {
                    *t = new_of[(*t as usize).min(n)];
                }
            }
            _ => {}
        }
    }
    *out = fused;
}

const _: () = assert!(
    std::mem::size_of::<TsInsn>() <= 24,
    "TsInsn grew past 24 bytes"
);

/// Calls `f` with every register `ins` defines, in place or as a destination.
fn ts_for_each_reg_def(ins: &Insn, mut f: impl FnMut(RegId)) {
    if let Some(d) = BytecodeCompiler::dest_reg(ins) {
        f(d);
    }
    if let Some(a) = BytecodeCompiler::in_place_reg(ins) {
        f(a);
    }
    match ins {
        Insn::CmpBranchConst(_, _, _, tmp, _) => f(*tmp),
        Insn::BinOpConstAdd2(a) => {
            f(a.d1);
            f(a.d2);
        }
        Insn::EvalExprFallback(_, d, _)
        | Insn::LoadDynElem(d, _, _)
        | Insn::Format(d, _)
        | Insn::StrOp(d, _, _) => f(*d),
        _ => {}
    }
}

/// For each instruction index, the earliest forward branch that targets it.
fn ts_earliest_forward_branch(insns: &[Insn]) -> Vec<Option<usize>> {
    let mut earliest: Vec<Option<usize>> = vec![None; insns.len() + 1];
    for (i, insn) in insns.iter().enumerate() {
        let mut note = |t: u32| {
            if let Some(slot) = earliest.get_mut(t as usize)
                && t as usize > i
                && slot.is_none()
            {
                *slot = Some(i);
            }
        };
        match insn {
            Insn::BranchIfFalse(_, t)
            | Insn::BranchUnlessZero(_, t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::CmpBranchConst(_, _, _, _, t)
            | Insn::Jump(t) => note(*t),
            Insn::CaseJump(_, cj) => {
                cj.table.iter().for_each(|&t| note(t));
                note(cj.default);
            }
            Insn::CaseMaskJump(_, mj) => {
                mj.table.iter().for_each(|&t| note(t));
                note(mj.xz_path);
            }
            _ => {}
        }
    }
    earliest
}

pub fn lower_two_state(
    cb: &CompiledBlock,
    signal_widths: &[u32],
    signal_signed: &[bool],
    signal_real: &[bool],
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Option<TwoStateBlock> {
    // Constant-index array stores fold to stores on the element signal for
    // the lowering only (see `fold_const_regs_masked`); the copy is 1:1 with
    // the original, so every stream index below still names the same
    // four-state instruction.
    let folded: Option<Vec<Insn>> = if cb.instructions.iter().any(|i| {
        matches!(
            i,
            Insn::NbaAssignArrayRange(..)
                | Insn::BlockingAssignArrayRange(..)
                | Insn::NbaAssignArray(..)
                | Insn::BlockingAssignArray(..)
        )
    }) {
        let mut v = cb.instructions.clone();
        BytecodeCompiler::fold_const_regs_masked(&mut v, signal_widths, FOLD_STORES_LOWERING);
        debug_assert_eq!(v.len(), cb.instructions.len());
        Some(v)
    } else {
        None
    };
    let insns: &[Insn] = match &folded {
        Some(v) => v.as_slice(),
        None => cb.instructions.as_slice(),
    };
    let n = insns.len();
    let mut out: Vec<TsInsn> = Vec::with_capacity(n);
    // Static width per register. A register REDEFINED with a different width
    // (possible for loop-var slots, which are mutable) bails: stream-order
    // width tracking can't represent it.
    let mut rw: Vec<Option<u32>> = vec![None; cb.num_regs as usize];
    // Widest register defined in the block: picks the wide bank's word count.
    let mut max_wide: u32 = 0;
    // Process-FSM wait bookkeeping (see `TwoStateBlock::has_wait`).
    let mut has_wait = false;
    // Wildcard compares (`casez`/`casex`) lower to masked equalities that
    // are exact only on an x-free selector; the x-plane executor does not
    // get such streams (a z selector bit is a wildcard there, not an x).
    let mut has_wild_case = false;
    // `===` lowers to the same `Eq`/`EqC` as `==` (they agree on x-free
    // operands); only the x-plane executor would tell them apart, so a
    // stream with a case equality stays off it.
    let mut has_case_eq = false;
    let mut wait_regs: Vec<(u32, Box<[(u16, u32, bool)]>)> = Vec::new();
    let mut skip_next = false;
    // Resume points are moved back onto the hazard saves that follow the
    // wait (a branch landing there runs the saves too, which is harmless).
    let mut wait_resume_fix: Vec<(u32, u32)> = Vec::new();
    // Constant value per register (from LoadConst), for folding const-index
    // array writes into static element stores. Cleared on any redefinition.
    let mut rc: Vec<Option<u64>> = vec![None; cb.num_regs as usize];
    // §6.11.1 signedness per register. Registers stay unsigned bit patterns;
    // the flag only decides (a) which relational compares are signed and
    // (b) where an implicit widening would have to sign-extend — `mn!`
    // (maybe negative) gates those, so a signed register whose value is a
    // known non-negative constant keeps lowering exactly as before.
    let mut sg: Vec<bool> = vec![false; cb.num_regs as usize];
    macro_rules! mn {
        ($r:expr) => {{
            let r = $r as usize;
            sg[r]
                && !matches!((rc[r], rw[r]), (Some(v), Some(w)) if w == 0 || (v >> (w - 1)) & 1 == 0)
        }};
    }
    // Once a side-effecting op is emitted, ABORTABLE ops (dynamic element
    // reads) can no longer be admitted: an abort must leave no trace.
    let mut side_effects = false;
    // Staged with a skip flag: a read of a signal this block has ALREADY
    // STORED (blocking) this evaluation is clean by construction and needs
    // no prefilter entry — but only for STRAIGHT-LINE blocks (a store inside
    // a branch may not execute), so the skip is applied after `has_ctrl` is
    // known. Wide loads BEFORE any side effect are covered by the exec-time
    // abort instead of the prefilter.
    let mut reads_whole: Vec<(u32, bool)> = Vec::new();
    let mut reads_slice: Vec<(u32, u16, u16, bool)> = Vec::new();
    let mut reads_wide: Vec<(u32, bool)> = Vec::new();
    let mut stored: Vec<u32> = Vec::new();
    // idx_map[i] = lowered index of the first lowered insn at-or-after
    // 4-state index i (dropped insns map to their successor). Branch targets
    // are rewritten through it in a fixup pass.
    let mut idx_map: Vec<u32> = Vec::with_capacity(n + 1);
    // ≤64-bit signals are checked whole (one raw_bits each) regardless of
    // which part is read; only WIDE signals get slice entries.
    // A block reading many signals (an unrolled reduction reads one per
    // trip) looks them up through an index instead of a linear scan; the
    // list only ever grows, so the index covers a prefix of it.
    let mut rw_index: HashMap<u32, usize> = HashMap::default();
    let mut note_read = |sig: usize,
                         lo: u32,
                         w: u32,
                         narrow: bool,
                         skip: bool,
                         rw_: &mut Vec<(u32, bool)>,
                         rs_: &mut Vec<(u32, u16, u16, bool)>| {
        if narrow {
            let s32 = sig as u32;
            let hit = if rw_.len() < 16 {
                rw_.iter().position(|(x, _)| *x == s32)
            } else {
                for (i, &(x, _)) in rw_.iter().enumerate().skip(rw_index.len()) {
                    rw_index.entry(x).or_insert(i);
                }
                rw_index.get(&s32).copied()
            };
            match hit {
                Some(i) => rw_[i].1 &= skip,
                None => rw_.push((s32, skip)),
            }
        } else {
            let t = (sig as u32, lo as u16, w as u16);
            if let Some(e) = rs_.iter_mut().find(|(a, b, c, _)| (*a, *b, *c) == t) {
                e.3 &= skip;
            } else {
                rs_.push((t.0, t.1, t.2, skip));
            }
        }
    };
    // Whole-value loads need the value in one u64; selects only need the
    // addressed slice, so they accept ANY signal width (the exec reads the
    // slice via raw_bits_slice).
    let sig_ok = |sig: usize| -> bool {
        sig < signal_widths.len() && signal_widths[sig] <= 64 && !signal_real[sig]
    };
    let sig_ok_slice = |sig: usize| -> bool { sig < signal_widths.len() && !signal_real[sig] };
    let sig_ok_wide = |sig: usize| -> bool {
        sig < signal_widths.len()
            && signal_widths[sig] > 64
            && signal_widths[sig] <= 512
            && !signal_real[sig]
    };
    // Static array span: (first_id, lo, hi) with elements proven narrow,
    // unsigned, non-real (elements of one array share their declaration;
    // the first element's metadata stands for all).
    let array_span = |a: &ArrayOperand| -> Option<(usize, i64, i64)> {
        let (first, lo, hi) = match a {
            ArrayOperand::Dense {
                first_id, lo, hi, ..
            } => (*first_id, *lo, *hi),
            ArrayOperand::Named(name) => {
                let &(first, lo, hi) = array_first_id.get(name.as_str())?;
                (first, lo, hi)
            }
        };
        if hi < lo {
            return None;
        }
        if first >= signal_widths.len()
            || signal_widths[first] > 64
            || signal_signed[first]
            || signal_real[first]
        {
            return None;
        }
        Some((first, lo, hi))
    };
    let clean_const = |k: &Value| -> Option<u64> {
        if k.width > 64 || k.is_real || k.is_fill {
            return None;
        }
        let (v, x) = k.raw_bits();
        if x != 0 {
            return None;
        }
        let v = v & ts_mask(k.width);
        // Bare decimal literals are SIGNED (§5.7.1), so rejecting every
        // signed constant kept `x + 1`, `x >> 3` and `x == 5` — most of the
        // RTL ever written — out of the islands. A NON-NEGATIVE signed
        // constant is admissible: zero- and sign-extension agree on it, and
        // every lowered register is unsigned by construction, so no
        // operation's signedness can flip (§5.5.1). A negative one still
        // bails, since its widening differs.
        if k.is_signed && k.width > 0 && (v >> (k.width - 1)) & 1 == 1 {
            return None;
        }
        Some(v)
    };
    // Stream-order width tracking is EXACT wherever a register's read is
    // reached by exactly ONE of its definitions. Bailing on every
    // differing-width redefinition threw away the `case` shape, where each
    // arm reuses the same temp slot at its own width and every use stays
    // inside its own arm: on the C906 SoC that single rule was the first
    // blocker for 58% of the remaining interpreter-bound evaluations.
    // Instead, allow the redefinition and reject only a read that a BRANCH
    // TARGET separates from its stream-order definition -- the one place a
    // second definition could reach it. `tcount[i]` counts branch targets at
    // indices <= i, so "no target in between" is one integer compare.
    let n_ins = insns.len();
    let mut tgts: Vec<u32> = Vec::new();
    for insn in insns.iter() {
        match insn {
            Insn::BranchIfFalse(_, t)
            | Insn::BranchUnlessZero(_, t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::CmpBranchConst(_, _, _, _, t)
            | Insn::Jump(t) => tgts.push(*t),
            Insn::CaseJump(_, cj) => {
                tgts.extend(cj.table.iter().copied());
                tgts.push(cj.default);
            }
            Insn::CaseMaskJump(_, mj) => {
                tgts.extend(mj.table.iter().copied());
                tgts.push(mj.xz_path);
            }
            _ => {}
        }
    }
    let mut is_tgt = vec![false; n_ins + 1];
    for t in tgts {
        if (t as usize) <= n_ins {
            is_tgt[t as usize] = true;
        }
    }
    let mut tcount = vec![0u32; n_ins + 1];
    let mut tc_run = 0u32;
    for i in 0..=n_ins {
        if is_tgt[i] {
            tc_run += 1;
        }
        tcount[i] = tc_run;
    }
    // A back edge can re-enter an earlier read from a LATER definition, which
    // the forward-only reasoning above does not model.
    let back_branch = BytecodeCompiler::has_backward_branch(insns);
    // See `ts_raw_hazard`: a block that reads a signal it later overwrites
    // cannot be re-run by the interpreter after a partial two-state run, and
    // every bail is such a re-run.
    // A hazardous ≤64-bit signal is saved at the top of the stream and
    // restored by the executor on a bail, so the re-run starts from the
    // values the block saw (`failures++` in a check's failing branch, with
    // an x read later in the block, counted twice before this).
    let hazards = match ts_raw_hazard(insns, array_first_id) {
        Ok(h) => h,
        Err(()) => {
            if std::env::var_os("XEZIM_TS_DBG").is_some() {
                TS_BAIL_AT.with(|c| c.set((usize::MAX, "RawHazard")));
            }
            return None;
        }
    };
    for &sig in &hazards {
        let sg = sig as usize;
        if sg >= signal_widths.len()
            || signal_widths[sg] > 512
            || signal_widths[sg] == 0
            || signal_real[sg]
        {
            if std::env::var_os("XEZIM_TS_DBG").is_some() {
                TS_BAIL_AT.with(|c| c.set((usize::MAX, "RawHazard")));
                TS_GATE_WHY.with(|c| c.set("raw hazard on a >512b/real signal"));
            }
            return None;
        }
        // A wide read-modify-write (`bus = {bus[..], ..}` on a >64-bit bus)
        // saves the whole value: cheaper than the interpreter these blocks
        // ran on, and it was 32% of a C910 SoC's interpreted evaluations.
        out.push(if signal_widths[sg] > 64 {
            TsInsn::SaveSigW { sig }
        } else {
            TsInsn::SaveSig { sig }
        });
    }
    let mut wconf: Vec<bool> = vec![false; cb.num_regs as usize];
    let mut wconf_list: Vec<RegId> = Vec::new();
    let mut def_tc: Vec<u32> = vec![0; cb.num_regs as usize];
    // Mirror of the loop index, readable from `def!` (a `for` pattern binding
    // is not, due to macro hygiene).
    let mut cur_i: usize;
    // Registers holding a folded 4-state constant: (value plane, xz plane).
    // These registers are NEVER materialized in the u64 file, so any reader
    // outside the fold set would see an unwritten slot -- the guard at the
    // top of the loop rejects the block instead.
    let mut xc: Vec<Option<(u64, u64)>> = vec![None; cb.num_regs as usize];
    let mut xc_live: Vec<RegId> = Vec::new();
    // A `{N{bit}}` replicate wider than the register banks is not
    // materialised: the register only remembers the bit, `rw` stays None so
    // every ordinary consumer bails, and the store arms turn it into a fill.
    let mut wfill: Vec<Option<u8>> = vec![None; cb.num_regs as usize];
    // `{N{1'bx}}` wider than the banks, the x/z twin of `wfill`: the register
    // remembers (value bit, x bit); only a blocking store may consume it.
    let mut xfill: Vec<Option<(u8, u8)>> = vec![None; cb.num_regs as usize];
    macro_rules! def {
        ($rw:ident, $r:expr, $w:expr) => {{
            let r = $r as usize;
            let w = $w;
            if w > max_wide {
                max_wide = w;
            }
            sg[r] = false;
            wfill[r] = None;
            xfill[r] = None;
            match $rw[r] {
                Some(prev) if prev != w => {
                    if !wconf[r] {
                        wconf[r] = true;
                        wconf_list.push(r as RegId);
                    }
                    $rw[r] = Some(w);
                }
                _ => $rw[r] = Some(w),
            }
            def_tc[r] = tcount[cur_i];
            rc[r] = None;
            xc[r] = None;
        }};
    }
    // Tracking costs one thread-local store per instruction, so it is armed
    // only when the dump is requested.
    static TRACK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let track = *TRACK.get_or_init(|| std::env::var("XEZIM_TS_DBG").is_ok());
    // Runtime opcode bisection: XEZIM_TS_DENY=CaseJump,RedOr,... makes any
    // block containing a listed 4-state opcode bail back to the interpreter,
    // so a suspected two-state miscompile can be isolated without rebuilding.
    static DENY: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let deny = DENY.get_or_init(|| {
        std::env::var("XEZIM_TS_DENY")
            .map(|v| {
                v.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    });
    // Names the exact condition that rejected an instruction, for arms with
    // more than one gate. Costs nothing unless the dump is armed.
    // A register wider than 64 bits lives in the WIDE plane file, so any
    // TsInsn that reads `regs[r]` must prove the register is narrow first —
    // otherwise it reads whatever the previous evaluation left in the slot.
    macro_rules! narrow_reg {
        ($rw:ident, $r:expr, $why:expr) => {{
            let w = $rw[$r as usize]?;
            if w > 64 {
                gate!($why);
            }
            w
        }};
    }
    macro_rules! gate {
        ($why:expr) => {{
            if track {
                TS_GATE_WHY.with(|c| c.set($why));
            }
            return None;
        }};
    }
    let bt_map = BytecodeCompiler::branch_target_map(insns);
    // Index of each register's LAST definition (dest or in-place), for the
    // back-edge reset below; `usize::MAX` marks the fused pair form, which
    // the lowering handles but this table does not model.
    let mut last_def: Vec<usize> = vec![0; cb.num_regs as usize];
    for (j, ins) in insns.iter().enumerate() {
        ts_for_each_reg_def(ins, |r| {
            if let Some(slot) = last_def.get_mut(r as usize) {
                *slot = j;
            }
        });
    }
    let earliest_branch = ts_earliest_forward_branch(insns);
    // Index of each register's latest definition in the forward walk, for the
    // forward-join reset below.
    let mut walk_def: Vec<usize> = vec![0; cb.num_regs as usize];
    for (ins_i, insn) in insns.iter().enumerate() {
        if track {
            TS_BAIL_AT.with(|c| c.set((ins_i, insn_opcode_name(insn))));
            TS_GATE_WHY.with(|c| c.set("-"));
        }
        if !deny.is_empty() && deny.iter().any(|d| d == insn_opcode_name(insn)) {
            gate!("bare bail at bytecode.rs:15020");
        }
        cur_i = ins_i;
        // A back edge re-enters a branch target from a LATER definition, so
        // nothing known about a register's VALUE survives the merge: a loop
        // counter seeded by `LoadConst(i, 0)` is not 0 on the second trip,
        // and an element read indexed by it must not become a constant
        // element read. Widths are guarded by the phi rule below.
        // Only a register DEFINED at or after the target can arrive here
        // with a value the forward walk did not see; a constant set before
        // the loop and never touched inside it stays known (clearing every
        // register cost a c906 SoC 0.8% in lost constant-index folds).
        if back_branch && is_tgt[ins_i] {
            for r in 0..rc.len() {
                if last_def[r] >= ins_i {
                    rc[r] = None;
                    wfill[r] = None;
                }
            }
        }
        // A forward branch can skip a definition between it and its target,
        // so a value the walk learned there is only one of the values that
        // reach the target. The out-of-range arm of a 2-D element index
        // (`flat = count`, see `compile_2d_flat_index`) read as a constant
        // index here, and the store it guards was dropped as out of range.
        if ins_i > 0 {
            ts_for_each_reg_def(&insns[ins_i - 1], |r| {
                if let Some(slot) = walk_def.get_mut(r as usize) {
                    *slot = ins_i - 1;
                }
            });
        }
        if let Some(branch) = earliest_branch[ins_i] {
            for r in 0..rc.len() {
                if walk_def[r] > branch {
                    rc[r] = None;
                    wfill[r] = None;
                }
            }
        }
        if !wconf_list.is_empty() {
            if back_branch {
                gate!("reg width phi (back edge)");
            }
            let tc = tcount[cur_i];
            if wconf_list
                .iter()
                .any(|&r| tc != def_tc[r as usize] && BytecodeCompiler::insn_reads_reg(insn, r))
            {
                gate!("reg width phi");
            }
        }
        if !xc_live.is_empty()
            && !matches!(
                insn,
                Insn::Replicate(..)
                    | Insn::Move(..)
                    | Insn::BlockingAssign(..)
                    | Insn::BlockingAssignRange(..)
                    | Insn::CasezEq(..)
                    | Insn::CasexEq(..)
            )
            && xc_live
                .iter()
                .any(|&r| xc[r as usize].is_some() && BytecodeCompiler::insn_reads_reg(insn, r))
        {
            gate!("x-const consumed");
        }
        idx_map.push(out.len() as u32);
        if skip_next {
            // The `WaitDelayReg` consumed by the constant fold below.
            skip_next = false;
            continue;
        }
        match insn {
            Insn::Nop => {}
            Insn::WaitEdge(ix) => {
                has_wait = true;
                let resume = ins_i as u32 + 1;
                out.push(TsInsn::WaitEdge { ix: *ix, resume });
                wait_resume_fix.push((resume, out.len() as u32));
                let snap: Vec<(u16, u32, bool)> = rw
                    .iter()
                    .enumerate()
                    .filter_map(|(r, w)| w.map(|w| (r as u16, w, sg[r])))
                    .collect();
                wait_regs.push((resume, snap.into_boxed_slice()));
                for &sig in &hazards {
                    out.push(TsInsn::SaveSig { sig });
                }
            }
            Insn::WaitDelayReg(_) => {
                // Only the constant-folded form (below) lowers; a dynamic
                // delay keeps the process on the four-state VM.
                gate!("dynamic delay wait");
            }
            // No-op here: every lowered register is unsigned by construction
            // (signed sources bail below).
            Insn::ClearSigned(r) => sg[*r as usize] = false,
            Insn::LoadSignal(d, sig) | Insn::LoadSignalSigned(d, sig) => {
                let sig = *sig as usize;
                // A signed source (an `integer` loop counter, typically) is
                // admitted narrow only, as an unsigned bit pattern tagged
                // signed in `sg`; wide signed sources still bail.
                let signed = signal_signed.get(sig).copied().unwrap_or(true)
                    || matches!(insn, Insn::LoadSignalSigned(..));
                if signed && !sig_ok(sig) {
                    gate!("wide signed signal");
                }
                if sig_ok(sig) {
                    note_read(
                        sig,
                        0,
                        signal_widths[sig],
                        true,
                        stored.contains(&(sig as u32)),
                        &mut reads_whole,
                        &mut reads_slice,
                    );
                    def!(rw, *d, signal_widths[sig]);
                    sg[*d as usize] = signed;
                    out.push(TsInsn::LoadSig {
                        d: *d as u16,
                        sig: sig as u32,
                    });
                } else if sig_ok_wide(sig) {
                    let skip = !side_effects || stored.contains(&(sig as u32));
                    let s32 = sig as u32;
                    if let Some(e) = reads_wide.iter_mut().find(|(x, _)| *x == s32) {
                        e.1 &= skip;
                    } else {
                        reads_wide.push((s32, skip));
                    }
                    def!(rw, *d, signal_widths[sig]);
                    out.push(TsInsn::WLoadSig {
                        d: *d as u16,
                        sig: sig as u32,
                    });
                } else {
                    gate!("signal >512b");
                }
            }
            Insn::LoadConst(d, k) => {
                if k.is_real {
                    // `LoadConst(real ticks); WaitDelayReg(d)` — the
                    // registration's constant-delay fold — lowers to one
                    // wait; any other real constant bails.
                    if let Some(Insn::WaitDelayReg(r)) = insns.get(ins_i + 1) {
                        if r == d {
                            has_wait = true;
                            let resume = ins_i as u32 + 2;
                            out.push(TsInsn::WaitDelayRaw {
                                raw: k.to_f64().to_bits(),
                                resume,
                            });
                            wait_resume_fix.push((resume, out.len() as u32));
                            let snap: Vec<(u16, u32, bool)> = rw
                                .iter()
                                .enumerate()
                                .filter_map(|(r, w)| w.map(|w| (r as u16, w, sg[r])))
                                .collect();
                            wait_regs.push((resume, snap.into_boxed_slice()));
                            for &sig in &hazards {
                                out.push(TsInsn::SaveSig { sig });
                            }
                            skip_next = true;
                            continue;
                        }
                    }
                    gate!("real constant");
                }
                if k.width > 64 {
                    if k.width > 512 || k.is_signed {
                        gate!("bare bail at bytecode.rs:15157");
                    }
                    let mut wv = vec![0u64; (k.width as usize).div_ceil(64)];
                    if !k.words_if_clean(&mut wv) {
                        gate!("bare bail at bytecode.rs:15161");
                    }
                    def!(rw, *d, k.width);
                    out.push(TsInsn::WConst {
                        d: *d as u16,
                        v: wv.into_boxed_slice(),
                    });
                } else if k.is_fill && !k.has_xz() {
                    // `'0` / `'1`: a fill takes the width of its consumer, so
                    // it rides the same fill-register path as `{N{bit}}` —
                    // the stores emit a range fill, and any other use bails.
                    let (v, _) = k.raw_bits();
                    def!(rw, *d, 1);
                    rw[*d as usize] = None;
                    wfill[*d as usize] = Some((v & 1) as u8);
                } else if k.has_xz() {
                    // Fold rather than reject: admitted only while it stays
                    // a constant all the way to a store (see the loop guard).
                    if k.is_signed || k.is_real || k.is_fill {
                        gate!("x-const signed/real");
                    }
                    let Some((v, x)) = k.inline_bits() else {
                        gate!("x-const not inline");
                    };
                    def!(rw, *d, k.width);
                    let m = ts_mask(k.width);
                    xc[*d as usize] = Some((v & m, x & m));
                    xc_live.push(*d);
                } else {
                    let v = clean_const(k)?;
                    def!(rw, *d, k.width);
                    rc[*d as usize] = Some(v);
                    sg[*d as usize] = k.is_signed;
                    out.push(TsInsn::Const { d: *d as u16, v });
                }
            }
            Insn::LoadSignalBit(d, sig, idx) => {
                let sig = *sig as usize;
                if !sig_ok_slice(sig) || *idx >= signal_widths[sig] || *idx > u16::MAX as u32 {
                    gate!("bare bail at bytecode.rs:15197");
                }
                let narrow = signal_widths[sig] <= 64;
                note_read(
                    sig,
                    *idx,
                    1,
                    narrow,
                    stored.contains(&(sig as u32)),
                    &mut reads_whole,
                    &mut reads_slice,
                );
                def!(rw, *d, 1);
                out.push(if narrow {
                    TsInsn::SigBit {
                        d: *d as u16,
                        sig: sig as u32,
                        bit: *idx as u16,
                    }
                } else {
                    TsInsn::SigBitW {
                        d: *d as u16,
                        sig: sig as u32,
                        bit: *idx as u16,
                    }
                });
            }
            Insn::LoadSignalRange(d, sig, l, r) => {
                let sig = *sig as usize;
                let (hi, lo) = (*l.max(r), *l.min(r));
                if !sig_ok_slice(sig) || hi >= signal_widths[sig] || hi > u16::MAX as u32 {
                    gate!("bare bail at bytecode.rs:15212");
                }
                let w = hi - lo + 1;
                if w > 64 {
                    if w > 512 {
                        gate!("bare bail at bytecode.rs:15217");
                    }
                    // Wide slice of a signal, noted as ≤64-bit chunks.
                    let mut off = 0;
                    while off < w {
                        let cw = (w - off).min(64);
                        note_read(
                            sig,
                            lo + off,
                            cw,
                            false,
                            stored.contains(&(sig as u32)),
                            &mut reads_whole,
                            &mut reads_slice,
                        );
                        off += 64;
                    }
                    def!(rw, *d, w);
                    out.push(TsInsn::WSigRange {
                        d: *d as u16,
                        sig: sig as u32,
                        lo: lo as u16,
                        w: w as u16,
                    });
                    continue;
                }
                let narrow = signal_widths[sig] <= 64;
                note_read(
                    sig,
                    lo,
                    w,
                    narrow,
                    stored.contains(&(sig as u32)),
                    &mut reads_whole,
                    &mut reads_slice,
                );
                def!(rw, *d, w);
                out.push(if narrow {
                    TsInsn::SigRange {
                        d: *d as u16,
                        sig: sig as u32,
                        lo: lo as u16,
                        mask: ts_mask(w),
                    }
                } else {
                    TsInsn::SigRangeW {
                        d: *d as u16,
                        sig: sig as u32,
                        lo: lo as u16,
                        w: w as u16,
                        mask: ts_mask(w),
                    }
                });
            }
            Insn::BitSelect(d, base, idx) => {
                let wb = narrow_reg!(rw, *base, "wide operand (bitsel)");
                narrow_reg!(rw, *idx, "wide bit index");
                def!(rw, *d, 1);
                out.push(TsInsn::BitDyn {
                    d: *d as u16,
                    s: *base as u16,
                    i: *idx as u16,
                    w: wb,
                });
            }
            Insn::BitSelectConst(d, s, idx) => {
                let sw = rw[*s as usize]?;
                if *idx >= sw || *idx > 511 {
                    gate!("bare bail at bytecode.rs:15264");
                }
                def!(rw, *d, 1);
                out.push(if sw > 64 {
                    TsInsn::BitFromW {
                        d: *d as u16,
                        s: *s as u16,
                        bit: *idx as u16,
                    }
                } else {
                    TsInsn::Bit {
                        d: *d as u16,
                        s: *s as u16,
                        bit: *idx as u8,
                    }
                });
            }
            Insn::RangeSelectConst(d, s, l, r) => {
                let sw = rw[*s as usize]?;
                let (hi, lo) = (*l.max(r), *l.min(r));
                if hi >= sw || hi > 511 {
                    gate!("bare bail at bytecode.rs:15277");
                }
                let w = hi - lo + 1;
                def!(rw, *d, w);
                out.push(if sw > 64 {
                    if w > 64 {
                        TsInsn::WRange {
                            d: *d as u16,
                            s: *s as u16,
                            lo: lo as u16,
                            w: w as u16,
                        }
                    } else {
                        TsInsn::RangeFromW {
                            d: *d as u16,
                            s: *s as u16,
                            lo: lo as u16,
                            mask: ts_mask(w),
                        }
                    }
                } else {
                    TsInsn::Range {
                        d: *d as u16,
                        s: *s as u16,
                        lo: lo as u8,
                        mask: ts_mask(w),
                    }
                });
            }
            // A plain register copy. Lowered as `d = s | s`, which is exact
            // (`x | x == x`) and needs no new opcode or executor arm — the
            // wide bank has the same identity. Worth an arm at all because
            // the TS bail census measured `Move` gating 43.8M interpreter
            // evaluations on the C906 SoC (14.1% of all of them): the block
            // was not doing anything two-state could not express, it just
            // had no lowering.
            Insn::Move(d, s) => {
                if let Some(f) = xfill[*s as usize] {
                    def!(rw, *d, 1);
                    rw[*d as usize] = None;
                    xfill[*d as usize] = Some(f);
                    continue;
                }
                if let Some(bit) = wfill[*s as usize] {
                    def!(rw, *d, 1);
                    rw[*d as usize] = None;
                    wfill[*d as usize] = Some(bit);
                    continue;
                }
                let Some(w) = rw[*s as usize] else {
                    gate!("move source width unknown");
                };
                if let Some(k) = xc[*s as usize] {
                    def!(rw, *d, w);
                    xc[*d as usize] = Some(k);
                    xc_live.push(*d);
                    continue;
                }
                def!(rw, *d, w);
                sg[*d as usize] = sg[*s as usize];
                rc[*d as usize] = rc[*s as usize];
                let (d, s) = (*d as u16, *s as u16);
                out.push(if w > 64 {
                    TsInsn::WOr { d, a: s, b: s }
                } else {
                    TsInsn::Or { d, a: s, b: s }
                });
            }
            // §11.4.11 conditional. The TS bail census measured `Select`
            // gating 57.8M interpreter evaluations (18.6%) — muxes are
            // everywhere in RTL and nothing about them resists two-state.
            // Both branches must share a width: the 4-state arm yields the
            // CHOSEN branch's own width, and a downstream `Not`/`Range` masks
            // by the register width, so adopting the max would change those
            // results. Unequal branches bail (conservative, still covers the
            // ordinary same-width mux).
            Insn::Select(d, c, a, b) => {
                let (wc, wa, wb) = (rw[*c as usize]?, rw[*a as usize]?, rw[*b as usize]?);
                if wa != wb {
                    gate!("select branch widths differ");
                }
                if wc > 64 {
                    gate!("wide select condition");
                }
                if wa > 512 {
                    gate!("select >512b");
                }
                def!(rw, *d, wa);
                sg[*d as usize] = sg[*a as usize] && sg[*b as usize];
                let (d, c, a, b) = (*d as u16, *c as u16, *a as u16, *b as u16);
                out.push(if wa > 64 {
                    TsInsn::WSel { d, c, a, b }
                } else {
                    TsInsn::Sel {
                        d,
                        c,
                        a,
                        b,
                        m: ts_mask(wa),
                    }
                });
            }
            Insn::BitXor(d, a, b) | Insn::BitAnd(d, a, b) | Insn::BitOr(d, a, b) => {
                let (wa, wb) = (rw[*a as usize]?, rw[*b as usize]?);
                let (aw, bw) = (wa > 64, wb > 64);
                if aw != bw {
                    // Mixed-bank operands — bail rather than model the
                    // zero-extension across banks.
                    gate!("mixed-width bitop");
                }
                let both_sg = sg[*a as usize] && sg[*b as usize];
                if both_sg && wa != wb && (if wa < wb { mn!(*a) } else { mn!(*b) }) {
                    gate!("signed widening (bitop)");
                }
                def!(rw, *d, wa.max(wb));
                sg[*d as usize] = both_sg;
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if aw {
                    match insn {
                        Insn::BitXor(..) => TsInsn::WXor { d, a, b },
                        Insn::BitAnd(..) => TsInsn::WAnd { d, a, b },
                        _ => TsInsn::WOr { d, a, b },
                    }
                } else {
                    match insn {
                        Insn::BitXor(..) => TsInsn::Xor { d, a, b },
                        Insn::BitAnd(..) => TsInsn::And { d, a, b },
                        _ => TsInsn::Or { d, a, b },
                    }
                });
            }
            Insn::BitNot(d, s) => {
                let w = rw[*s as usize]?;
                let s_sg = sg[*s as usize];
                def!(rw, *d, w);
                sg[*d as usize] = s_sg;
                out.push(if w > 64 {
                    if w > 512 {
                        gate!("bare bail at bytecode.rs:15409");
                    }
                    TsInsn::WNot {
                        d: *d as u16,
                        s: *s as u16,
                        w: w as u16,
                    }
                } else {
                    TsInsn::Not {
                        d: *d as u16,
                        s: *s as u16,
                        mask: ts_mask(w),
                    }
                });
            }
            // Wrapping at max operand width; zero-extension is the correct
            // §11.8.1 widening because both registers are unsigned.
            Insn::Add(d, a, b) | Insn::Sub(d, a, b) => {
                let (wa, wb) = (
                    narrow_reg!(rw, *a, "wide operand (add/sub)"),
                    narrow_reg!(rw, *b, "wide operand (add/sub)"),
                );
                let w = wa.max(wb);
                let both_sg = sg[*a as usize] && sg[*b as usize];
                if both_sg && wa != wb && (if wa < wb { mn!(*a) } else { mn!(*b) }) {
                    gate!("signed widening (add/sub)");
                }
                def!(rw, *d, w);
                sg[*d as usize] = both_sg;
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if matches!(insn, Insn::Add(..)) {
                    TsInsn::Add {
                        d,
                        a,
                        b,
                        mask: ts_mask(w),
                    }
                } else {
                    TsInsn::Sub {
                        d,
                        a,
                        b,
                        mask: ts_mask(w),
                    }
                });
            }
            // §11.4.10 logical shifts. Result width = LEFT operand width.
            // AShr is excluded: every lowered register is unsigned, so an
            // arithmetic shift would need sign tracking the bank lacks.
            Insn::Shl(d, a, b) | Insn::Shr(d, a, b) => {
                let wa = narrow_reg!(rw, *a, "wide operand (shift)");
                narrow_reg!(rw, *b, "wide shift amount");
                let a_sg = sg[*a as usize];
                def!(rw, *d, wa);
                sg[*d as usize] = a_sg;
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if matches!(insn, Insn::Shl(..)) {
                    TsInsn::Shl {
                        d,
                        a,
                        b,
                        w: wa,
                        mask: ts_mask(wa),
                    }
                } else {
                    TsInsn::Shr { d, a, b, w: wa }
                });
            }
            // 4-state table semantics (x/z selector -> default) cannot lower
            // to the 2-state pipeline. `CaseJump` is the exception and is
            // handled below, as is `CaseMaskJump`: the eval-site prefilter
            // proves every signal the block reads is X-free, so their selectors
            // cannot carry x/z and the "no pattern matches -> default" /
            // wildcard-chain branches are unreachable, leaving a plain
            // bounds-checked table index.
            Insn::CaseLut(..) => return None,
            Insn::Format(..) => return None,
            Insn::StrOp(..) => return None,
            Insn::BlockingAssignString(..) => return None,
            // 1-bit results. CaseEq/CaseNeq equal Eq/Neq on X-free values.
            Insn::CasezEq(d, a, b) | Insn::CasexEq(d, a, b) => {
                has_wild_case = true;
                let sw = narrow_reg!(rw, *a, "wide operand (casez)");
                let Some(pw) = rw[*b as usize] else {
                    gate!("casez pattern width unknown");
                };
                if pw > 64 {
                    gate!("wide operand (casez)");
                }
                let casex = matches!(insn, Insn::CasexEq(..));
                let (pv, px) = if let Some((v, x)) = xc[*b as usize] {
                    (v, x)
                } else if let Some(v) = rc[*b as usize] {
                    (v, 0u64)
                } else {
                    gate!("casez pattern not const");
                };
                let m = ts_mask(sw.max(pw));
                // Z (val=1,xz=1) is the wildcard for casez; casex also
                // treats X (val=0,xz=1) as one. A literal X in a casez
                // pattern never equals a clean selector bit.
                let wild = if casex { px } else { px & pv };
                let (d, a) = (*d as u16, *a as u16);
                def!(rw, d, 1);
                if !casex && (px & !pv) != 0 {
                    rc[d as usize] = Some(0);
                    out.push(TsInsn::Const { d, v: 0 });
                } else {
                    let mask = m & !wild;
                    out.push(TsInsn::MaskEq {
                        d,
                        s: a,
                        mask,
                        v: pv & mask,
                    });
                }
            }
            Insn::Eq(d, a, b) | Insn::CaseEq(d, a, b) => {
                if matches!(insn, Insn::CaseEq(..)) {
                    has_case_eq = true;
                }
                let wa = narrow_reg!(rw, *a, "wide operand (eq)");
                let wb = narrow_reg!(rw, *b, "wide operand (eq)");
                if sg[*a as usize]
                    && sg[*b as usize]
                    && wa != wb
                    && (if wa < wb { mn!(*a) } else { mn!(*b) })
                {
                    gate!("signed widening (eq)");
                }
                def!(rw, *d, 1);
                out.push(TsInsn::Eq {
                    d: *d as u16,
                    a: *a as u16,
                    b: *b as u16,
                });
            }
            Insn::Neq(d, a, b) => {
                let wa = narrow_reg!(rw, *a, "wide operand (neq)");
                let wb = narrow_reg!(rw, *b, "wide operand (neq)");
                if sg[*a as usize]
                    && sg[*b as usize]
                    && wa != wb
                    && (if wa < wb { mn!(*a) } else { mn!(*b) })
                {
                    gate!("signed widening (neq)");
                }
                def!(rw, *d, 1);
                out.push(TsInsn::Neq {
                    d: *d as u16,
                    a: *a as u16,
                    b: *b as u16,
                });
            }
            // §5.5.1: unsigned compare when either operand is unsigned —
            // always here, since every lowered register is unsigned.
            Insn::Lt(d, a, b) | Insn::Leq(d, a, b) | Insn::Gt(d, a, b) | Insn::Geq(d, a, b) => {
                let wa = narrow_reg!(rw, *a, "wide operand (cmp)");
                let wb = narrow_reg!(rw, *b, "wide operand (cmp)");
                let both_sg = sg[*a as usize] && sg[*b as usize];
                def!(rw, *d, 1);
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                if both_sg {
                    let kind = match insn {
                        Insn::Lt(..) => 0,
                        Insn::Leq(..) => 1,
                        Insn::Gt(..) => 2,
                        _ => 3,
                    };
                    out.push(TsInsn::CmpS {
                        d,
                        a,
                        b,
                        kind,
                        sa: (64 - wa) as u8,
                        sb: (64 - wb) as u8,
                    });
                    continue;
                }
                out.push(match insn {
                    Insn::Lt(..) => TsInsn::Lt { d, a, b },
                    Insn::Leq(..) => TsInsn::Leq { d, a, b },
                    Insn::Gt(..) => TsInsn::Gt { d, a, b },
                    _ => TsInsn::Geq { d, a, b },
                });
            }
            Insn::LogNot(d, s) => {
                narrow_reg!(rw, *s, "wide operand (lognot)");
                def!(rw, *d, 1);
                out.push(TsInsn::LogNot {
                    d: *d as u16,
                    s: *s as u16,
                });
            }
            Insn::LogAnd(d, a, b) | Insn::LogOr(d, a, b) => {
                narrow_reg!(rw, *a, "wide operand (logic)");
                narrow_reg!(rw, *b, "wide operand (logic)");
                def!(rw, *d, 1);
                let (d, a, b) = (*d as u16, *a as u16, *b as u16);
                out.push(if matches!(insn, Insn::LogAnd(..)) {
                    TsInsn::LogAnd { d, a, b }
                } else {
                    TsInsn::LogOr { d, a, b }
                });
            }
            Insn::BinOpConstAdd2(a) => {
                if mn!(a.s1) || mn!(a.s2) {
                    gate!("signed operand (addc2)");
                }
                // Exactly the two `AddC` lowerings the pair had before
                // merging — in order, so a chained `s2 == d1` sees d1's
                // freshly defined width.
                let w1 = narrow_reg!(rw, a.s1, "wide operand (addc2)");
                let v1 = clean_const(&a.k1)?;
                let wr1 = w1.max(a.k1.width);
                def!(rw, a.d1, wr1);
                out.push(TsInsn::AddC {
                    d: a.d1 as u16,
                    s: a.s1 as u16,
                    k: v1,
                    mask: ts_mask(wr1),
                });
                let w2 = narrow_reg!(rw, a.s2, "wide operand (addc2)");
                let v2 = clean_const(&a.k2)?;
                let wr2 = w2.max(a.k2.width);
                def!(rw, a.d2, wr2);
                out.push(TsInsn::AddC {
                    d: a.d2 as u16,
                    s: a.s2 as u16,
                    k: v2,
                    mask: ts_mask(wr2),
                });
            }
            Insn::BinOpConst(d, s, k, kind) | Insn::CmpBranchConst(kind, s, k, d, _) => {
                if matches!(insn, Insn::CmpBranchConst(..))
                    && !matches!(kind, BinOpConstKind::Eq | BinOpConstKind::CaseEq)
                {
                    gate!("unsupported constant comparison");
                }
                let w = narrow_reg!(rw, *s, "wide operand (binop-const)");
                let v = clean_const(k)?;
                let both_sg = sg[*s as usize] && k.is_signed;
                if both_sg && w < k.width && mn!(*s) {
                    gate!("signed widening (binop-const)");
                }
                match kind {
                    BinOpConstKind::Xor => {
                        def!(rw, *d, w.max(k.width));
                        sg[*d as usize] = both_sg;
                        out.push(TsInsn::XorC {
                            d: *d as u16,
                            s: *s as u16,
                            k: v,
                        });
                    }
                    BinOpConstKind::Eq | BinOpConstKind::CaseEq => {
                        if matches!(kind, BinOpConstKind::CaseEq) {
                            has_case_eq = true;
                        }
                        def!(rw, *d, 1);
                        out.push(TsInsn::EqC {
                            d: *d as u16,
                            s: *s as u16,
                            k: v,
                        });
                    }
                    BinOpConstKind::Add => {
                        let wr = w.max(k.width);
                        def!(rw, *d, wr);
                        sg[*d as usize] = both_sg;
                        out.push(TsInsn::AddC {
                            d: *d as u16,
                            s: *s as u16,
                            k: v,
                            mask: ts_mask(wr),
                        });
                    }
                    // No constant-operand two-state form for these. The
                    // constant is materialised in the DESTINATION register,
                    // which this instruction defines anyway, and the
                    // register form follows; `d == s` would clobber the
                    // source first.
                    BinOpConstKind::And
                    | BinOpConstKind::Or
                    | BinOpConstKind::Mul
                    | BinOpConstKind::Sub => {
                        if d == s {
                            gate!("in-place binop-const");
                        }
                        if k.width > 64 {
                            gate!("bare bail at bytecode.rs:15625");
                        }
                        let wr = w.max(k.width);
                        def!(rw, *d, wr);
                        sg[*d as usize] = both_sg;
                        let (d, s) = (*d as u16, *s as u16);
                        out.push(TsInsn::Const { d, v });
                        out.push(match kind {
                            BinOpConstKind::And => TsInsn::And { d, a: s, b: d },
                            BinOpConstKind::Or => TsInsn::Or { d, a: s, b: d },
                            BinOpConstKind::Sub => TsInsn::Sub {
                                d,
                                a: s,
                                b: d,
                                mask: ts_mask(wr),
                            },
                            _ => TsInsn::Mul {
                                d,
                                a: s,
                                b: d,
                                mask: ts_mask(wr),
                            },
                        });
                    }
                }
                if let Insn::CmpBranchConst(_, _, _, _, t) = insn {
                    out.push(TsInsn::BrFalse { s: *d, t: *t });
                }
            }
            Insn::Concat(d, parts) => {
                let mut total = 0u32;
                let mut any_wide = false;
                let mut lowered: Vec<(u16, u16, bool)> = Vec::with_capacity(parts.len());
                for &p in parts.iter() {
                    let Some(w) = rw[p as usize] else {
                        gate!("concat part width unknown");
                    };
                    if w > 512 {
                        gate!("concat part >512b");
                    }
                    total += w;
                    if w > 64 {
                        any_wide = true;
                    }
                    lowered.push((p as u16, w as u16, w > 64));
                }
                if total == 0 || total > 512 {
                    gate!("concat total >512b");
                }
                def!(rw, *d, total);
                out.push(if total > 64 || any_wide {
                    TsInsn::WConcat {
                        d: *d as u16,
                        parts: lowered.into_boxed_slice(),
                    }
                } else if lowered.len() == 2 {
                    // Narrow forms: every part is ≤ 64 bits here.
                    TsInsn::Concat2 {
                        d: *d as u16,
                        a: lowered[0].0,
                        wa: lowered[0].1 as u8,
                        b: lowered[1].0,
                        wb: lowered[1].1 as u8,
                    }
                } else if lowered.len() == 3 {
                    TsInsn::Concat3 {
                        d: *d as u16,
                        a: lowered[0].0,
                        wa: lowered[0].1 as u8,
                        b: lowered[1].0,
                        wb: lowered[1].1 as u8,
                        c: lowered[2].0,
                        wc: lowered[2].1 as u8,
                    }
                } else {
                    TsInsn::Concat {
                        d: *d as u16,
                        parts: lowered
                            .into_iter()
                            .map(|(r, w, _)| (r, w as u8))
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    }
                });
            }
            Insn::Resize(r, w) => {
                let cur = rw[*r as usize]?;
                if *w > 512 {
                    gate!("bare bail at bytecode.rs:15697");
                }
                if *w > cur && mn!(*r) {
                    gate!("signed widening (resize)");
                }
                if *w > 64 {
                    if *w > max_wide {
                        max_wide = *w;
                    }
                    if cur <= 64 {
                        out.push(TsInsn::WFromN { r: *r as u16 });
                    } else if *w < cur {
                        out.push(TsInsn::WMask {
                            d: *r as u16,
                            w: *w as u16,
                        });
                    }
                    rw[*r as usize] = Some(*w);
                    def_tc[*r as usize] = tcount[cur_i];
                    rc[*r as usize] = None;
                    continue;
                }
                if cur > 64 {
                    out.push(TsInsn::NFromW {
                        r: *r as u16,
                        mask: ts_mask(*w),
                    });
                    rw[*r as usize] = Some(*w);
                    def_tc[*r as usize] = tcount[cur_i];
                    rc[*r as usize] = None;
                    continue;
                }
                if *w < cur {
                    out.push(TsInsn::Mask {
                        d: *r as u16,
                        mask: ts_mask(*w),
                    });
                }
                // Widening zero-extends — free for an unsigned register.
                // Redefinition width-conflict does not apply: Resize is a
                // width CHANGE of the same value, not a second definition.
                // It IS the point the width now dates from, though, so the
                // phi guard measures from here (without marking a conflict,
                // which would newly reject blocks that lower correctly today).
                rw[*r as usize] = Some(*w);
                def_tc[*r as usize] = tcount[cur_i];
                // Keep const knowledge in sync with the (possible) truncation.
                if let Some(k) = rc[*r as usize] {
                    rc[*r as usize] = Some(k & ts_mask(*w));
                }
            }
            Insn::BranchIfSignalFalse(sig, t, bit) => {
                let sig = *sig as usize;
                if *bit == u32::MAX {
                    // Whole-value truthiness needs the value in one word.
                    if !sig_ok(sig) {
                        gate!("bare bail at bytecode.rs:15744");
                    }
                    note_read(
                        sig,
                        0,
                        signal_widths[sig],
                        true,
                        stored.contains(&(sig as u32)),
                        &mut reads_whole,
                        &mut reads_slice,
                    );
                } else {
                    if !sig_ok_slice(sig) || *bit >= signal_widths[sig] || *bit > u16::MAX as u32 {
                        gate!("bare bail at bytecode.rs:15752");
                    }
                    let narrow = signal_widths[sig] <= 64;
                    note_read(
                        sig,
                        *bit,
                        1,
                        narrow,
                        stored.contains(&(sig as u32)),
                        &mut reads_whole,
                        &mut reads_slice,
                    );
                }
                out.push(TsInsn::BrSigFalse {
                    sig: sig as u32,
                    bit: *bit,
                    t: *t,
                });
            }
            Insn::BranchIfFalse(c, t) => {
                narrow_reg!(rw, *c, "wide branch condition");
                out.push(TsInsn::BrFalse {
                    s: *c as u16,
                    t: *t,
                });
            }
            // Fused compare+branch: decompose to the exact unfused lowering,
            // reusing the embedded dead register as the compare scratch.
            Insn::CmpBranch(kind, a, b, tmp, t) => {
                let wa = narrow_reg!(rw, *a, "wide operand (cmpbranch)");
                let wb = narrow_reg!(rw, *b, "wide operand (cmpbranch)");
                let both_sg = sg[*a as usize] && sg[*b as usize];
                if both_sg
                    && matches!(kind, CmpKind::Eq | CmpKind::CaseEq | CmpKind::Neq)
                    && wa != wb
                    && (if wa < wb { mn!(*a) } else { mn!(*b) })
                {
                    gate!("signed widening (cmpbranch)");
                }
                def!(rw, *tmp, 1);
                let (d, a, b) = (*tmp as u16, *a as u16, *b as u16);
                if both_sg && !matches!(kind, CmpKind::Eq | CmpKind::CaseEq | CmpKind::Neq) {
                    let k = match kind {
                        CmpKind::Lt => 0,
                        CmpKind::Leq => 1,
                        CmpKind::Gt => 2,
                        _ => 3,
                    };
                    out.push(TsInsn::CmpS {
                        d,
                        a,
                        b,
                        kind: k,
                        sa: (64 - wa) as u8,
                        sb: (64 - wb) as u8,
                    });
                    out.push(TsInsn::BrFalse { s: d, t: *t });
                    continue;
                }
                out.push(match kind {
                    CmpKind::Eq | CmpKind::CaseEq => TsInsn::Eq { d, a, b },
                    CmpKind::Neq => TsInsn::Neq { d, a, b },
                    CmpKind::Lt => TsInsn::Lt { d, a, b },
                    CmpKind::Leq => TsInsn::Leq { d, a, b },
                    CmpKind::Gt => TsInsn::Gt { d, a, b },
                    CmpKind::Geq => TsInsn::Geq { d, a, b },
                });
                out.push(TsInsn::BrFalse { s: d, t: *t });
            }
            Insn::BranchUnlessZero(s, t) => {
                narrow_reg!(rw, *s, "wide branch condition");
                // do-while back-edge: see `TsInsn::LoopPoll`.
                if (*t as usize) <= ins_i {
                    out.push(TsInsn::LoopPoll);
                }
                out.push(TsInsn::BrNz {
                    s: *s as u16,
                    t: *t,
                });
            }
            Insn::Jump(t) => {
                out.push(TsInsn::Jmp { t: *t });
            }
            Insn::CaseJump(src, cj) => {
                // The selector must live in the u64 register file: a wider
                // one is kept in the wide plane file, where `regs[s]` is not
                // its value at all.
                let sw = rw[*src as usize]?;
                if sw > 64 {
                    gate!("casejump sel >64b");
                }
                out.push(TsInsn::CaseJmp {
                    s: *src as u16,
                    cj: cj.clone(),
                });
            }
            Insn::CaseMaskJump(src, mj) => {
                has_wild_case = true;
                let sw = rw[*src as usize]?;
                if sw > 64 {
                    gate!("casemaskjump sel >64b");
                }
                if mj.lo + mj.width > 64 {
                    gate!("casemaskjump window >=64");
                }
                out.push(TsInsn::CaseMaskJmp {
                    s: *src as u16,
                    mj: Box::new(TsCaseMaskJmp {
                        mask: ts_mask(sw),
                        lo: mj.lo,
                        wmask: ts_mask(mj.width),
                        mj: mj.clone(),
                    }),
                });
            }
            Insn::ReduceOr(d, src) => {
                // Registers wider than 64 bits live in the WIDE plane file;
                // `regs[s]` for such a register is whatever the previous
                // block's evaluation left in that slot. Emitting the narrow
                // form regardless of width made `|wide_bus` evaluate from
                // stale garbage — on the C910 SoC that wedged the LSU after
                // exactly 250 retired instructions (the first `|128-bit`
                // control reduction on the store path).
                let sw = rw[*src as usize]?;
                def!(rw, *d, 1);
                out.push(if sw > 64 {
                    TsInsn::WRedOr {
                        d: *d as u16,
                        s: *src as u16,
                    }
                } else {
                    TsInsn::RedOr {
                        d: *d as u16,
                        s: *src as u16,
                    }
                });
            }
            Insn::ReduceAnd(d, src) => {
                let sw = rw[*src as usize]?;
                def!(rw, *d, 1);
                out.push(if sw > 64 {
                    TsInsn::WRedAnd {
                        d: *d as u16,
                        s: *src as u16,
                        w: sw as u16,
                    }
                } else {
                    TsInsn::RedAnd {
                        d: *d as u16,
                        s: *src as u16,
                        mask: ts_mask(sw),
                    }
                });
            }
            // A signed tag on a narrow register: the LoadSignalSigned
            // treatment — an unsigned bit pattern marked in `sg`, which the
            // consumers that care (`mn!`) test. `int` loop counters produce
            // this on every iteration.
            Insn::SetSigned(r) => {
                let w = rw[*r as usize]?;
                if w > 64 {
                    gate!("wide signed");
                }
                sg[*r as usize] = true;
            }
            // Two-state registers hold no X/Z: nothing to drop.
            Insn::DropXz(_) => {}
            // Nor reals: real signals and real constants gate above.
            Insn::RealToInt(_) => {}
            Insn::StoreLocalSlice(..) => gate!("four-state local slice store"),
            // A copy with the Resize arm's narrow rules folded in.
            Insn::MoveResize(d, s, w) => {
                let cur = rw[*s as usize]?;
                if *w == 0 || *w > 64 || cur > 64 {
                    gate!("wide move-resize");
                }
                if xc[*s as usize].is_some() || wfill[*s as usize].is_some() {
                    gate!("x-const move-resize");
                }
                if *w > cur && mn!(*s) {
                    gate!("signed widening (move-resize)");
                }
                let src_sg = sg[*s as usize];
                let src_rc = rc[*s as usize];
                def!(rw, *d, *w);
                sg[*d as usize] = src_sg;
                rc[*d as usize] = src_rc.map(|k| k & ts_mask(*w));
                out.push(TsInsn::Range {
                    d: *d as u16,
                    s: *s as u16,
                    lo: 0,
                    mask: ts_mask(*w),
                });
            }
            // `sig[lo +: W]` with a run-time `lo`: the whole signal counts
            // as read, since the slice can land anywhere in it.
            Insn::LoadSignalRangeDyn(d, sig, lo, w) => {
                let sig = *sig as usize;
                let Some(&sw) = signal_widths.get(sig) else {
                    gate!("dyn slice source oob");
                };
                if signal_real[sig] || *w == 0 || *w > 64 || sw > u16::MAX as u32 {
                    gate!("dyn slice source shape");
                }
                narrow_reg!(rw, *lo, "wide slice offset");
                let s32 = sig as u32;
                if sw > 64 {
                    if !sig_ok_wide(sig) {
                        gate!("dyn slice wide source");
                    }
                    let skip = !side_effects || stored.contains(&s32);
                    if let Some(e) = reads_wide.iter_mut().find(|(x, _)| *x == s32) {
                        e.1 &= skip;
                    } else {
                        reads_wide.push((s32, skip));
                    }
                } else {
                    if !sig_ok(sig) {
                        gate!("dyn slice source");
                    }
                    note_read(
                        sig,
                        0,
                        sw,
                        true,
                        stored.contains(&s32),
                        &mut reads_whole,
                        &mut reads_slice,
                    );
                }
                def!(rw, *d, *w);
                out.push(TsInsn::SigRangeDyn {
                    d: *d as u16,
                    sig: s32,
                    i: *lo as u16,
                    w: *w as u16,
                    sw,
                    mask: ts_mask(*w),
                });
            }
            // `sig[lo +: W] = v` / `<= v` with a run-time `lo`. The width is
            // static only in the compiler's own shapes: `hi == lo` (one
            // bit) or `hi` defined as `lo + K` immediately upstream.
            Insn::BlockingAssignRangeDyn(sig, hi, lo, r)
            | Insn::NbaAssignRangeDyn(sig, hi, lo, r) => {
                let sig = *sig as usize;
                let Some(&sw) = signal_widths.get(sig) else {
                    gate!("dyn range dest oob");
                };
                if signal_real[sig] || sw > u16::MAX as u32 {
                    gate!("dyn range dest shape");
                }
                let w = if hi == lo {
                    1
                } else {
                    let mut k = None;
                    for j in (0..ins_i).rev() {
                        if bt_map[j + 1] {
                            break;
                        }
                        let ins = &insns[j];
                        if BytecodeCompiler::dest_reg(ins) == Some(*lo)
                            || BytecodeCompiler::in_place_reg(ins) == Some(*lo)
                        {
                            break;
                        }
                        if BytecodeCompiler::dest_reg(ins) == Some(*hi)
                            || BytecodeCompiler::in_place_reg(ins) == Some(*hi)
                        {
                            if let Insn::BinOpConst(_, s, kk, BinOpConstKind::Add) = ins {
                                if *s == *lo && !kk.is_signed && !kk.has_xz() {
                                    k = kk.to_u64();
                                }
                            }
                            break;
                        }
                    }
                    match k {
                        Some(k) if k < 64 => k as u32 + 1,
                        _ => gate!("dyn range width"),
                    }
                };
                if xc[*r as usize].is_some() || wfill[*r as usize].is_some() {
                    gate!("x-const dyn range store");
                }
                narrow_reg!(rw, *lo, "wide range offset");
                let cw = narrow_reg!(rw, *r, "wide store source");
                if cw < w && mn!(*r) {
                    gate!("signed widening (dyn range store)");
                }
                side_effects = true;
                let nba = matches!(insn, Insn::NbaAssignRangeDyn(..));
                if !nba {
                    stored.push(sig as u32);
                }
                let (sig, i, s, w16, mask) =
                    (sig as u32, *lo as u16, *r as u16, w as u16, ts_mask(w));
                out.push(if nba {
                    TsInsn::RangeStoreNbaDyn {
                        sig,
                        i,
                        s,
                        w: w16,
                        sw,
                        mask,
                    }
                } else {
                    TsInsn::RangeStoreDyn {
                        sig,
                        i,
                        s,
                        w: w16,
                        sw,
                        mask,
                    }
                });
            }
            Insn::NbaAssignBitDyn(sig, idx, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] || signal_widths[sig] > 64 {
                    gate!("nba bit dest oob/real/wide");
                }
                narrow_reg!(rw, *idx, "wide bit index");
                narrow_reg!(rw, *r, "wide store source");
                side_effects = true;
                out.push(TsInsn::BitStoreNbaDyn {
                    sig: sig as u32,
                    i: *idx as u16,
                    s: *r as u16,
                    w: signal_widths[sig],
                });
            }
            Insn::BlockingAssignBitDyn(sig, idx, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] {
                    gate!("dest oob/real");
                }
                // A >64-bit destination is fine: the executor sets one bit in
                // the wide planes (`ts_wide_bit_store`) instead of splicing
                // the inline word.
                narrow_reg!(rw, *idx, "wide bit index");
                narrow_reg!(rw, *r, "wide store source");
                side_effects = true;
                stored.push(sig as u32);
                out.push(TsInsn::BitStoreDyn {
                    sig: sig as u32,
                    i: *idx as u16,
                    s: *r as u16,
                    w: signal_widths[sig],
                });
            }
            Insn::BlockingAssign(sig, r, w) => {
                let sig = *sig as usize;
                if let Some((v, _)) = xfill[*r as usize] {
                    if sig >= signal_widths.len()
                        || signal_widths[sig] != *w
                        || signal_real[sig]
                        || *w == 0
                    {
                        gate!("x-fill dest shape");
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::RangeFillXW {
                        sig: sig as u32,
                        hi: *w - 1,
                        lo: 0,
                        v,
                    });
                    continue;
                }
                if let Some(bit) = wfill[*r as usize] {
                    if sig >= signal_widths.len()
                        || signal_widths[sig] != *w
                        || signal_real[sig]
                        || *w == 0
                    {
                        gate!("fill dest shape");
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::RangeFillW {
                        sig: sig as u32,
                        hi: *w - 1,
                        lo: 0,
                        bit,
                    });
                    continue;
                }
                let cw = rw[*r as usize]?;
                if let Some((v, x)) = xc[*r as usize] {
                    // §10.4.1 with a folded constant source. Every lowered
                    // register is unsigned, so a narrow source zero-extends:
                    // masking to the narrower of the two widths IS the resize.
                    if sig >= signal_widths.len()
                        || signal_widths[sig] != *w
                        || signal_real[sig]
                        || *w > 64
                    {
                        gate!("x-const dest shape");
                    }
                    let m = ts_mask(cw.min(*w));
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::ConstStoreX {
                        sig: sig as u32,
                        v: v & m,
                        x: x & m,
                    });
                    continue;
                }
                // Same-width, non-real destination only: the 4-state slow
                // path's fit/resize semantics are not reproduced here.
                if sig >= signal_widths.len() || signal_widths[sig] != *w || signal_real[sig] {
                    gate!("bare bail at bytecode.rs:16076");
                }
                if *w > 64 {
                    // Wide store: register and destination agree exactly (a
                    // Resize precedes otherwise); signed wide targets bail.
                    if *w > 512 || cw != *w || signal_signed[sig] {
                        gate!("bare bail at bytecode.rs:16082");
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::WStore {
                        sig: sig as u32,
                        s: *r as u16,
                    });
                } else {
                    if cw > 64 {
                        gate!("bare bail at bytecode.rs:16089");
                    }
                    if cw < *w && mn!(*r) {
                        gate!("signed widening (store)");
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::Store {
                        sig: sig as u32,
                        s: *r as u16,
                        mask: ts_mask(*w),
                    });
                }
            }
            Insn::NbaAssignConst(sig, k, w) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] || *w > 64 {
                    gate!("bare bail at bytecode.rs:16102");
                }
                let v = clean_const(k)?;
                side_effects = true;
                out.push(TsInsn::ConstStoreNba {
                    sig: sig as u32,
                    v: v & ts_mask(*w),
                    w: *w,
                });
            }
            Insn::BlockingAssignRange(sig, hi, lo, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] {
                    gate!("dest oob/real");
                }
                if let Some((v, _)) = xfill[*r as usize] {
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if high >= signal_widths[sig] {
                        gate!("x-fill range past dest");
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::RangeFillXW {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        v,
                    });
                    continue;
                }
                if let Some(bit) = wfill[*r as usize] {
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if high >= signal_widths[sig] {
                        gate!("fill range past dest");
                    }
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::RangeFillW {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        bit,
                    });
                    continue;
                }
                let wide_dest = signal_widths[sig] > 64;
                if let Some((v, x)) = xc[*r as usize] {
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if high - low + 1 > 64 {
                        gate!("range span >64");
                    }
                    if !wide_dest && high >= 64 {
                        gate!("range top >=64");
                    }
                    if high >= signal_widths[sig] {
                        gate!("range top past dest");
                    }
                    let m = ts_mask(high - low + 1);
                    side_effects = true;
                    stored.push(sig as u32);
                    let px = Box::new(TsRangeStoreX {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        v: v & m,
                        x: x & m,
                    });
                    out.push(if wide_dest {
                        TsInsn::RangeStoreXW(px)
                    } else {
                        TsInsn::RangeStoreX(px)
                    });
                    continue;
                }
                let Some(cw) = rw[*r as usize] else {
                    gate!("src reg width unknown");
                };
                let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                let w = high - low + 1;
                if cw > 64 && cw <= 512 && w == cw && wide_dest && high < signal_widths[sig] {
                    side_effects = true;
                    stored.push(sig as u32);
                    out.push(TsInsn::WRangeStore {
                        sig: sig as u32,
                        lo: low,
                        s: *r as u16,
                        w,
                    });
                    continue;
                }
                if cw > 64 || w > 64 {
                    gate!("src reg >64b");
                }
                if cw < w && mn!(*r) {
                    gate!("signed widening (range store)");
                }
                if !wide_dest && high >= 64 {
                    gate!("range top >=64");
                }
                if high >= signal_widths[sig] {
                    gate!("range top past dest");
                }
                side_effects = true;
                stored.push(sig as u32);
                out.push(if wide_dest {
                    TsInsn::RangeStoreW {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        s: *r as u16,
                        mask: ts_mask(w),
                    }
                } else {
                    TsInsn::RangeStore {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        s: *r as u16,
                        mask: ts_mask(w),
                    }
                });
            }
            Insn::NbaAssignRange(sig, hi, lo, r) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] {
                    gate!("nba range dest oob/real");
                }
                if let Some(bit) = wfill[*r as usize] {
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if high >= signal_widths[sig] {
                        gate!("fill range past dest");
                    }
                    side_effects = true;
                    out.push(TsInsn::RangeFillNbaW {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        bit,
                    });
                    continue;
                }
                let Some(cw) = rw[*r as usize] else {
                    gate!("untracked source (nba range store)");
                };
                let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                let w = high - low + 1;
                if signal_widths[sig] > 64 {
                    // Wide destination: a ≤64-bit window spliced into the
                    // pending value (the id_dp pipeline registers on c906).
                    if cw > 64 && cw <= 512 && w == cw && high < signal_widths[sig] {
                        side_effects = true;
                        out.push(TsInsn::WRangeStoreNba {
                            sig: sig as u32,
                            lo: low,
                            s: *r as u16,
                            w,
                        });
                        continue;
                    }
                    if cw > 64 || w > 64 || high >= signal_widths[sig] {
                        gate!("wide nba range shape");
                    }
                    if cw < w && mn!(*r) {
                        gate!("signed widening (nba range store)");
                    }
                    side_effects = true;
                    out.push(TsInsn::RangeStoreNbaW {
                        sig: sig as u32,
                        hi: high,
                        lo: low,
                        s: *r as u16,
                        mask: ts_mask(w),
                    });
                    continue;
                }
                if cw > 64 || high >= 64 {
                    gate!("wide source (nba range store)");
                }
                if cw < w && mn!(*r) {
                    gate!("signed widening (nba range store)");
                }
                side_effects = true;
                out.push(TsInsn::RangeStoreNba {
                    sig: sig as u32,
                    hi: high,
                    lo: low,
                    s: *r as u16,
                    mask: ts_mask(w),
                });
            }
            Insn::NbaAssign(sig, r, w) => {
                let sig = *sig as usize;
                if sig >= signal_widths.len() || signal_real[sig] {
                    gate!("nba dest oob/real");
                }
                if let Some(bit) = wfill[*r as usize] {
                    if signal_widths[sig] != *w || *w == 0 {
                        gate!("fill dest shape");
                    }
                    side_effects = true;
                    out.push(TsInsn::RangeFillNbaW {
                        sig: sig as u32,
                        hi: *w - 1,
                        lo: 0,
                        bit,
                    });
                    continue;
                }
                let Some(cw) = rw[*r as usize] else {
                    gate!("untracked source (nba)");
                };
                if *w > 64 {
                    if *w > 512 || cw != *w || signal_signed[sig] {
                        gate!("wide nba dest shape");
                    }
                    side_effects = true;
                    out.push(TsInsn::WStoreNba {
                        sig: sig as u32,
                        s: *r as u16,
                        w: *w,
                    });
                } else {
                    if cw > 64 {
                        gate!("bare bail at bytecode.rs:16281");
                    }
                    if cw < *w && mn!(*r) {
                        gate!("signed widening (nba store)");
                    }
                    side_effects = true;
                    out.push(TsInsn::StoreNba {
                        sig: sig as u32,
                        s: *r as u16,
                        w: *w,
                        mask: ts_mask(*w),
                    });
                }
            }
            Insn::LoadArrayElem(d, array, idx_reg) => {
                // Every element shares the first one's shape. Wide elements
                // (65..=512 bits) load into the wide bank; signed and real
                // elements stay on the four-state VM.
                let (first, lo, hi) = match &**array {
                    ArrayOperand::Dense {
                        first_id, lo, hi, ..
                    } => (*first_id, *lo, *hi),
                    ArrayOperand::Named(name) => match array_first_id.get(name.as_str()) {
                        Some(&t) => t,
                        None => gate!("array unknown"),
                    },
                };
                if hi < lo || first >= signal_widths.len() || signal_real[first] {
                    gate!("array elements real/empty");
                }
                if signal_signed[first] {
                    gate!("array elements signed");
                }
                let ew = signal_widths[first];
                if ew > 512 {
                    gate!("array elements >512b");
                }
                let wide = ew > 64;
                let _ = side_effects;
                // A constant, in-range index names one element: that is a
                // plain signal load with the ordinary x-read contract, not an
                // abortable element read — so it stays admissible after a
                // store (`q <= mem[3][7:0]` behind a reset arm was 256 of
                // the c906 edge bails).
                if let Some(k) = rc[*idx_reg as usize] {
                    let ki = k as i64;
                    if ki < lo || ki > hi {
                        gate!("const element out of range");
                    }
                    let eid = first + (ki - lo) as usize;
                    if eid >= signal_widths.len()
                        || signal_widths[eid] != ew
                        || signal_signed[eid]
                        || signal_real[eid]
                    {
                        gate!("const element shape");
                    }
                    def!(rw, *d, ew);
                    if wide {
                        let skip = !side_effects || stored.contains(&(eid as u32));
                        let s32 = eid as u32;
                        if let Some(e) = reads_wide.iter_mut().find(|(x, _)| *x == s32) {
                            e.1 &= skip;
                        } else {
                            reads_wide.push((s32, skip));
                        }
                        out.push(TsInsn::WLoadSig {
                            d: *d as u16,
                            sig: s32,
                        });
                    } else {
                        note_read(
                            eid,
                            0,
                            ew,
                            true,
                            stored.contains(&(eid as u32)),
                            &mut reads_whole,
                            &mut reads_slice,
                        );
                        out.push(TsInsn::LoadSig {
                            d: *d as u16,
                            sig: eid as u32,
                        });
                    }
                    continue;
                }
                // Abortable (X/out-of-range element read). Admissible after
                // side effects too: `ts_raw_hazard` proved the four-state
                // re-run reproduces every store made before the abort.
                narrow_reg!(rw, *idx_reg, "wide array index");
                def!(rw, *d, ew);
                let op = Box::new(TsElemOp {
                    first: first as u32,
                    lo,
                    hi,
                    idx: *idx_reg as u16,
                    s: *d as u16,
                    w: 0,
                    mask: 0,
                });
                out.push(if wide {
                    TsInsn::WElemLoad(op)
                } else {
                    TsInsn::ElemLoad(op)
                });
            }
            Insn::NbaAssignArray(array, idx_reg, val_reg, w) => {
                let (first, lo, hi) = array_span(array)?;
                let vw = narrow_reg!(rw, *val_reg, "wide store source");
                if *w > 64 || signal_widths[first] != *w {
                    gate!("bare bail at bytecode.rs:16339");
                }
                if vw < *w && mn!(*val_reg) {
                    gate!("signed widening (array store)");
                }
                side_effects = true;
                if let Some(k) = rc[*idx_reg as usize] {
                    // Const index folds to a static element target (an
                    // out-of-range constant mirrors the silent 4-state drop
                    // by emitting nothing).
                    let ki = k as i64;
                    if ki >= lo && ki <= hi {
                        let eid = first + (ki - lo) as usize;
                        out.push(TsInsn::StoreNba {
                            sig: eid as u32,
                            s: *val_reg as u16,
                            w: *w,
                            mask: ts_mask(*w),
                        });
                    }
                } else {
                    narrow_reg!(rw, *idx_reg, "wide array index");
                    out.push(TsInsn::ElemStoreNba(Box::new(TsElemOp {
                        first: first as u32,
                        lo,
                        hi,
                        idx: *idx_reg as u16,
                        s: *val_reg as u16,
                        w: *w,
                        mask: ts_mask(*w),
                    })));
                }
            }
            Insn::BlockingAssignArray(array, idx_reg, val_reg, w) => {
                let (first, lo, hi) = array_span(array)?;
                let vw = narrow_reg!(rw, *val_reg, "wide store source");
                if *w > 64 || signal_widths[first] != *w {
                    gate!("bare bail at bytecode.rs:16376");
                }
                if vw < *w && mn!(*val_reg) {
                    gate!("signed widening (array store)");
                }
                side_effects = true;
                if let Some(k) = rc[*idx_reg as usize] {
                    let ki = k as i64;
                    if ki >= lo && ki <= hi {
                        let eid = first + (ki - lo) as usize;
                        out.push(TsInsn::Store {
                            sig: eid as u32,
                            s: *val_reg as u16,
                            mask: ts_mask(*w),
                        });
                    }
                } else {
                    narrow_reg!(rw, *idx_reg, "wide array index");
                    out.push(TsInsn::ElemStore(Box::new(TsElemOp {
                        first: first as u32,
                        lo,
                        hi,
                        idx: *idx_reg as u16,
                        s: *val_reg as u16,
                        w: *w,
                        mask: ts_mask(*w),
                    })));
                }
            }
            Insn::NbaAssignArrayRead(dst, array, idx_sig, w) => {
                let isig = *idx_sig as usize;
                let d = *dst as usize;
                if !sig_ok(isig) || d >= signal_widths.len() || signal_real[d] {
                    gate!("nba array read shape");
                }
                let (first, lo, hi) = match &**array {
                    ArrayOperand::Dense {
                        first_id, lo, hi, ..
                    } => (*first_id, *lo, *hi),
                    ArrayOperand::Named(name) => match array_first_id.get(name.as_str()) {
                        Some(&t) => t,
                        None => gate!("array unknown"),
                    },
                };
                if hi < lo
                    || first >= signal_widths.len()
                    || signal_real[first]
                    || signal_signed[first]
                {
                    gate!("array elements signed/real");
                }
                let ew = signal_widths[first];
                if ew > 64 {
                    // Wide element: queue the element Value itself.
                    if *w != ew || signal_widths[d] != ew {
                        gate!("wide nba array read shape");
                    }
                    note_read(
                        isig,
                        0,
                        signal_widths[isig],
                        true,
                        stored.contains(&(isig as u32)),
                        &mut reads_whole,
                        &mut reads_slice,
                    );
                    side_effects = true;
                    out.push(TsInsn::WNbaFromElem(Box::new(TsNbaFromElem {
                        dst: *dst,
                        first: first as u32,
                        lo,
                        hi,
                        idx_sig: isig as u32,
                        w: *w,
                    })));
                    continue;
                }
                if *w > 64 {
                    gate!("nba array read width");
                }
                // Historically the read was ABORTABLE (X index, X data,
                // out-of-range), so it had to precede every side effect: a
                // mid-block bail replays the block 4-state, and queued NBAs
                // would double-apply. Two of the three abort sources are now
                // gone — X element DATA is queued as a 4-state Value (the NBA
                // queue holds Values; see the executor), and an X INDEX is
                // impossible when the guard covers `isig`. When the array
                // also spans the index's whole range (RAM with power-of-two
                // depth: lo == 0, hi >= 2^idx_w - 1), out-of-range is
                // statically impossible too — the read is then NON-abortable
                // and may follow side effects. Anything else keeps the old
                // ordering rule. c906: 536 edge blocks bailed here.
                let idx_w = signal_widths[isig];
                let statically_in_range = lo <= 0
                    && idx_w < 63
                    && hi >= (1i64 << idx_w) - 1
                    && !stored.contains(&(isig as u32));
                let _ = statically_in_range;
                if false {
                    gate!("bare bail at bytecode.rs:16435");
                }
                note_read(
                    isig,
                    0,
                    signal_widths[isig],
                    true,
                    stored.contains(&(isig as u32)),
                    &mut reads_whole,
                    &mut reads_slice,
                );
                side_effects = true;
                out.push(TsInsn::NbaFromElem(Box::new(TsNbaFromElem {
                    dst: *dst,
                    first: first as u32,
                    lo,
                    hi,
                    idx_sig: isig as u32,
                    w: *w,
                })));
            }
            Insn::Mul(d, a, b) => {
                let (wa, wb) = (rw[*a as usize]?, rw[*b as usize]?);
                if wa > 64 || wb > 64 {
                    gate!("bare bail at bytecode.rs:16451");
                }
                let w = wa.max(wb);
                let both_sg = sg[*a as usize] && sg[*b as usize];
                if both_sg && wa != wb && (if wa < wb { mn!(*a) } else { mn!(*b) }) {
                    gate!("signed widening (mul)");
                }
                def!(rw, *d, w);
                sg[*d as usize] = both_sg;
                if let (Some(ka), Some(kb)) = (rc[*a as usize], rc[*b as usize]) {
                    // Both operands are known constants (a genvar expression
                    // the compiler left unfolded): fold at lower time.
                    let v = ka.wrapping_mul(kb) & ts_mask(w);
                    rc[*d as usize] = Some(v);
                    out.push(TsInsn::Const { d: *d as u16, v });
                } else {
                    out.push(TsInsn::Mul {
                        d: *d as u16,
                        a: *a as u16,
                        b: *b as u16,
                        mask: ts_mask(w),
                    });
                }
            }
            Insn::Replicate(d, src, n) => {
                let sw = rw[*src as usize]?;
                let n = *n;
                if n > 128 && sw == 1 && xc[*src as usize].is_none() {
                    if let Some(k) = rc[*src as usize] {
                        if k <= 1 {
                            def!(rw, *d, 1);
                            rw[*d as usize] = None;
                            wfill[*d as usize] = Some(k as u8);
                            continue;
                        }
                    }
                }
                if n == 0 || sw > 64 || n > 512 {
                    gate!("bare bail at bytecode.rs:16489");
                }
                let total = sw.saturating_mul(n);
                if let Some((v, x)) = xc[*src as usize] {
                    if total > 64 {
                        if sw != 1 || x & 1 == 0 {
                            gate!("x-const repl >64b");
                        }
                        def!(rw, *d, 1);
                        rw[*d as usize] = None;
                        xfill[*d as usize] = Some(((v & 1) as u8, 1));
                        continue;
                    }
                    let (mut rv, mut rx) = (0u64, 0u64);
                    for i in 0..n {
                        rv |= v << (i * sw);
                        rx |= x << (i * sw);
                    }
                    def!(rw, *d, total);
                    xc[*d as usize] = Some((rv, rx));
                    xc_live.push(*d);
                    continue;
                }
                if total == 0 || total > 512 {
                    gate!("concat total >512b");
                }
                def!(rw, *d, total);
                out.push(if total > 64 {
                    TsInsn::WRepl {
                        d: *d as u16,
                        s: *src as u16,
                        w: sw as u8,
                        count: n as u16,
                    }
                } else {
                    TsInsn::Repl {
                        d: *d as u16,
                        s: *src as u16,
                        w: sw as u8,
                        count: n as u8,
                    }
                });
            }
            Insn::StmtFallback(payload) => {
                // Run by the interpreter in place; its reads and writes go
                // through the four-state paths, so nothing here needs the
                // register model to know about it — unless it carries
                // register-backed locals, which live in the 4-state file.
                if !payload.2.is_empty() {
                    gate!("fallback carries locals");
                }
                side_effects = true;
                out.push(TsInsn::Fallback(payload.0.clone()));
            }
            Insn::InitLocalColl(payload) => {
                // Declares empty local collection storage; its writes go
                // through the interpreter's signal paths.
                side_effects = true;
                out.push(TsInsn::Fallback(payload.0.clone()));
            }
            // The executors note each count on the save list, so a bail takes
            // it back before the four-state re-run counts again.
            Insn::CovHit(c) => out.push(TsInsn::CovHit(*c)),
            _ => return None,
        }
    }
    idx_map.push(out.len() as u32);
    for &(pc, ix) in &wait_resume_fix {
        if (pc as usize) < idx_map.len() {
            idx_map[pc as usize] = ix;
        }
    }
    // Fixup: branch targets were recorded as 4-state indices.
    for insn in out.iter_mut() {
        match insn {
            TsInsn::BrSigFalse { t, .. }
            | TsInsn::BrFalse { t, .. }
            | TsInsn::BrNz { t, .. }
            | TsInsn::Jmp { t } => {
                let old = *t as usize;
                if old >= idx_map.len() {
                    gate!("bare bail at bytecode.rs:16557");
                }
                *t = idx_map[old];
            }
            TsInsn::CaseMaskJmp { mj, .. } => {
                // `xz_path` is unreachable on X-free registers but is still
                // remapped: leaving a 4-state index in a lowered stream would
                // be a live landmine if the invariant ever weakened.
                let mj = &mut mj.mj;
                for t in mj.table.iter_mut().chain(std::iter::once(&mut mj.xz_path)) {
                    let old = *t as usize;
                    if old >= idx_map.len() {
                        gate!("bare bail at bytecode.rs:16569");
                    }
                    *t = idx_map[old];
                }
            }
            TsInsn::CaseJmp { cj, .. } => {
                for t in cj.table.iter_mut().chain(std::iter::once(&mut cj.default)) {
                    let old = *t as usize;
                    if old >= idx_map.len() {
                        gate!("bare bail at bytecode.rs:16578");
                    }
                    *t = idx_map[old];
                }
            }
            _ => {}
        }
    }
    // A block that stores nothing is useless to run in 2-state.
    //
    // This list must name EVERY store opcode. It did not: the partial-range
    // and constant NBA stores were absent, so a block whose only side effect
    // was a range store lowered perfectly and was then discarded here as
    // "stores nothing". On the C906 SoC that silently rejected 176.4M
    // interpreter evaluations — 58.4% of all of them — in blocks of exactly
    // the shape `LoadSignalRange(r, wide, 127, 64); BlockingAssignRange(dst,
    // 63, 0, r)` (wide-bus word shuffles). Found by labelling the arm's own
    // gates and seeing that NONE of them fired: the bail was here, after the
    // instruction loop, with the stamp merely naming the last instruction
    // entered.
    if !out.iter().any(|i| {
        matches!(
            i,
            TsInsn::Store { .. }
                | TsInsn::StoreNba { .. }
                | TsInsn::ConstStoreNba { .. }
                | TsInsn::RangeStore { .. }
                | TsInsn::RangeStoreNba { .. }
                | TsInsn::BitStoreDyn { .. }
                | TsInsn::BitStoreNbaDyn { .. }
                | TsInsn::RangeStoreDyn { .. }
                | TsInsn::RangeStoreNbaDyn { .. }
                | TsInsn::RangeFillXW { .. }
                | TsInsn::ConstStoreX { .. }
                | TsInsn::RangeStoreX(..)
                | TsInsn::RangeStoreW { .. }
                | TsInsn::RangeStoreNbaW { .. }
                | TsInsn::RangeFillW { .. }
                | TsInsn::RangeFillNbaW { .. }
                | TsInsn::WRangeStore { .. }
                | TsInsn::WRangeStoreNba { .. }
                | TsInsn::LogOrStore { .. }
                | TsInsn::OrRangeStore { .. }
                | TsInsn::RangeStoreXW(..)
                | TsInsn::ElemStore { .. }
                | TsInsn::ElemStoreNba { .. }
                | TsInsn::ElemStoreNbaFromSig { .. }
                | TsInsn::NbaFromElem { .. }
                | TsInsn::WNbaFromElem { .. }
                | TsInsn::WStore { .. }
                | TsInsn::WStoreNba { .. }
        )
    }) {
        gate!("bare bail at bytecode.rs:16633");
    }
    // 512-bit wide class, on by default (XEZIM_TS_WIDE512=0 disables). It
    // was opt-in while admitting the c906 vector-unit buses cost +5.4%:
    // entries that x-bailed on every evaluation and never demoted. Those now
    // sleep (see the executor's x-bail backoff), and with wide hazard saves,
    // wide selects and x-fill stores lowering as well the class measures
    // c906 memcpy -1.2% and C910 memcpy -2.5%, outputs identical.
    static WIDE512: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let wide512 =
        *WIDE512.get_or_init(|| std::env::var("XEZIM_TS_WIDE512").ok().as_deref() != Some("0"));
    if max_wide > 128 && !wide512 {
        gate!("bare bail at bytecode.rs:16643");
    }
    let has_wide = out.iter().any(|i| {
        matches!(
            i,
            TsInsn::WLoadSig { .. }
                | TsInsn::WRangeStore { .. }
                | TsInsn::WRangeStoreNba { .. }
                | TsInsn::WRedOr { .. }
                | TsInsn::WRedAnd { .. }
                | TsInsn::WConst { .. }
                | TsInsn::WXor { .. }
                | TsInsn::WAnd { .. }
                | TsInsn::WOr { .. }
                | TsInsn::WSel { .. }
                | TsInsn::WElemLoad(..)
                | TsInsn::WNot { .. }
                | TsInsn::WRange { .. }
                | TsInsn::RangeFromW { .. }
                | TsInsn::BitFromW { .. }
                | TsInsn::WConcat { .. }
                | TsInsn::WMask { .. }
                | TsInsn::WFromN { .. }
                | TsInsn::NFromW { .. }
                | TsInsn::WStore { .. }
                | TsInsn::WStoreNba { .. }
                | TsInsn::WRepl { .. }
                | TsInsn::WSigRange { .. }
        )
    });
    let has_ctrl = out.iter().any(|i| {
        matches!(
            i,
            TsInsn::BrSigFalse { .. }
                | TsInsn::BrFalse { .. }
                | TsInsn::BrNz { .. }
                | TsInsn::Jmp { .. }
                | TsInsn::CaseJmp { .. }
                | TsInsn::CaseMaskJmp { .. }
        )
    });
    let apply_skip = !has_ctrl;
    let reads_whole: Vec<u32> = reads_whole
        .into_iter()
        .filter(|&(_, sk)| !(apply_skip && sk))
        .map(|(x, _)| x)
        .collect();
    let reads_slice: Vec<(u32, u16, u16)> = reads_slice
        .into_iter()
        .filter(|&(_, _, _, sk)| !(apply_skip && sk))
        .map(|(a, b, c, _)| (a, b, c))
        .collect();
    let reads_wide: Vec<u32> = reads_wide
        .into_iter()
        .filter(|&(_, sk)| !(apply_skip && sk))
        .map(|(x, _)| x)
        .collect();
    let mut writes: Vec<u32> = Vec::new();
    let mut writes_span: Vec<(u32, u32)> = Vec::new();
    // §10.6.2: every store kind names its destination here — a block whose
    // write a force drops must leave the two-state executor, and a store
    // left out (a part-select `RangeStore`) kept writing a forced net.
    for i in out.iter() {
        match i {
            TsInsn::LogAndStore { sig, .. }
            | TsInsn::AndRangeStore { sig, .. }
            | TsInsn::LogOrStore { sig, .. }
            | TsInsn::OrRangeStore { sig, .. }
            | TsInsn::BitStoreDyn { sig, .. }
            | TsInsn::ConstStoreX { sig, .. }
            | TsInsn::ConstStoreNba { sig, .. }
            | TsInsn::RangeStore { sig, .. }
            | TsInsn::RangeStoreNba { sig, .. }
            | TsInsn::RangeStoreW { sig, .. }
            | TsInsn::Store { sig, .. }
            | TsInsn::StoreNba { sig, .. }
            | TsInsn::BitStoreNbaDyn { sig, .. }
            | TsInsn::RangeStoreDyn { sig, .. }
            | TsInsn::RangeStoreNbaDyn { sig, .. }
            | TsInsn::RangeFillXW { sig, .. }
            | TsInsn::RangeStoreNbaW { sig, .. }
            | TsInsn::RangeFillW { sig, .. }
            | TsInsn::RangeFillNbaW { sig, .. }
            | TsInsn::WRangeStore { sig, .. }
            | TsInsn::WRangeStoreNba { sig, .. }
            | TsInsn::WStore { sig, .. }
            | TsInsn::WStoreNba { sig, .. } => writes.push(*sig),
            TsInsn::RangeStoreX(p) | TsInsn::RangeStoreXW(p) => writes.push(p.sig),
            TsInsn::NbaFromElem(op) | TsInsn::WNbaFromElem(op) => writes.push(op.dst),
            TsInsn::ElemStoreNbaFromSig(f) => {
                let op = &f.op;
                writes_span.push((op.first, (op.hi - op.lo + 1).max(0) as u32));
            }
            TsInsn::ElemStore(op) | TsInsn::ElemStoreNba(op) => {
                if op.hi >= op.lo {
                    writes_span.push((op.first, (op.hi - op.lo + 1) as u32));
                }
            }
            _ => {}
        }
    }
    writes.sort_unstable();
    writes.dedup();
    writes_span.sort_unstable();
    writes_span.dedup();
    if !has_wait {
        // Fusion re-indexes the stream; a wait-bearing stream keeps its
        // four-state index map exact instead.
        fuse_ts_pairs(&mut out);
    }
    // Loop back-edges poll Ctrl-C (see `TsInsn::JmpPoll`). In place, so no
    // index moves; nothing after this point re-reads branch targets.
    for (i, insn) in out.iter_mut().enumerate() {
        if let TsInsn::Jmp { t } = *insn {
            if (t as usize) <= i {
                *insn = TsInsn::JmpPoll { t };
            }
        }
    }
    if track && std::env::var_os("XEZIM_TS_DUMP").is_some() {
        eprintln!("[TS-DUMP] {} insns:", out.len());
        for (i, ti) in out.iter().enumerate() {
            eprintln!("[TS-DUMP]   {i:>3} {ti:?}");
        }
    }
    // §6.24.1: DropXz is a no-op only for clean two-state registers.
    // The X-plane executor must use the original stream for such a cast.
    let tsx = !has_wide
        && !has_wait
        && !has_wild_case
        && !has_case_eq
        && !cb.instructions.iter().any(|i| matches!(i, Insn::DropXz(_)))
        && out.iter().all(tsx_insn_ok);
    Some(TwoStateBlock {
        insns: out,
        num_regs: cb.num_regs,
        reads_whole: reads_whole.into_boxed_slice(),
        reads_slice: reads_slice.into_boxed_slice(),
        has_ctrl,
        has_wide,
        wide_words: if !has_wide {
            0
        } else if max_wide > 128 {
            8
        } else {
            2
        },
        has_wait,
        tsx,
        vm_to_ts: if has_wait {
            idx_map.clone().into_boxed_slice()
        } else {
            Box::new([])
        },
        wait_regs: wait_regs.into_boxed_slice(),
        reads_wide: reads_wide.into_boxed_slice(),
        writes: writes.into_boxed_slice(),
        writes_span: writes_span.into_boxed_slice(),
    })
}
