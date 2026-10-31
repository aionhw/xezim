//! Guarded word execution for read-only parallel epochs (§4.6 / §10.4.2).
use super::super::bytecode::{TsInsn, TwoStateBlock};
use super::*;

#[derive(Clone, Copy)]
enum WordOp {
    Repl {
        d: u16,
        s: u16,
        mask: u64,
        factor: u64,
    },
    LoadSigRepl {
        dl: u16,
        sig: u32,
        d: u16,
        mask: u64,
        factor: u64,
    },
    WRepl {
        d: usize,
        s: u16,
        w: u8,
        count: u16,
    },
    WSigRange {
        d: usize,
        sig: u32,
        lo: u16,
        w: u16,
    },
    LoadSig {
        d: u16,
        sig: u32,
    },
    Const {
        d: u16,
        v: u64,
    },
    SigBit {
        d: u16,
        sig: u32,
        bit: u16,
    },
    SigRange {
        d: u16,
        sig: u32,
        lo: u16,
        mask: u64,
    },
    SigSlice {
        d: u16,
        sig: u32,
        lo: u16,
        w: u16,
        mask: u64,
    },
    Bit {
        d: u16,
        s: u16,
        bit: u8,
    },
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
    Sel {
        d: u16,
        c: u16,
        a: u16,
        b: u16,
    },
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
    MaskEq {
        d: u16,
        s: u16,
        mask: u64,
        v: u64,
    },
    EqC {
        d: u16,
        s: u16,
        k: u64,
    },
    Add {
        d: u16,
        a: u16,
        b: u16,
        mask: u64,
    },
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
    Sub {
        d: u16,
        a: u16,
        b: u16,
        mask: u64,
    },
    Eq {
        d: u16,
        a: u16,
        b: u16,
    },
    Neq {
        d: u16,
        a: u16,
        b: u16,
    },
    CmpS {
        d: u16,
        a: u16,
        b: u16,
        kind: u8,
        sa: u8,
        sb: u8,
    },
    BitDyn {
        d: u16,
        s: u16,
        i: u16,
        w: u32,
    },
    LoadSigNot {
        dl: u16,
        d: u16,
        sig: u32,
    },
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
    SigBitNot {
        db: u16,
        d: u16,
        sig: u32,
        bit: u16,
    },
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
    AndOr {
        d1: u16,
        a1: u16,
        b1: u16,
        d: u16,
        a: u16,
        b: u16,
    },
    Lt {
        d: u16,
        a: u16,
        b: u16,
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
    RedOr {
        d: u16,
        s: u16,
    },
    RedAnd {
        d: u16,
        s: u16,
        mask: u64,
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
    Concat2 {
        d: u16,
        a: u16,
        b: u16,
        wb: u8,
    },
    Concat3 {
        d: u16,
        a: u16,
        b: u16,
        wb: u8,
        c: u16,
        wc: u8,
    },
    Mask {
        d: u16,
        mask: u64,
    },
    Mul {
        d: u16,
        a: u16,
        b: u16,
        mask: u64,
    },
    BrFalse {
        s: u16,
        t: u32,
    },
    BrNz {
        s: u16,
        t: u32,
    },
    BrSigFalse {
        sig: u32,
        bit: u32,
        t: u32,
    },
    Jmp {
        t: u32,
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
    EqCBrFalse {
        d: u16,
        s: u16,
        k: u64,
        t: u32,
    },
    StoreNba {
        sig: u32,
        s: u16,
        w: u32,
        mask: u64,
    },
    ConstStoreNba {
        sig: u32,
        v: u64,
        w: u32,
    },
    RangeStoreNba {
        sig: u32,
        hi: u32,
        lo: u32,
        s: u16,
        mask: u64,
    },
    RangeFillNba {
        sig: u32,
        hi: u32,
        lo: u32,
        bit: u8,
    },
    WLoadSig {
        d: usize,
        sig: u32,
    },
    WConst {
        d: usize,
        k: usize,
    },
    WStoreNba {
        sig: u32,
        s: usize,
        w: u32,
    },
    WRangeStoreNba {
        sig: u32,
        lo: u32,
        s: usize,
        w: u32,
    },
    WMask {
        d: usize,
        w: u16,
    },
    WFromN {
        r: u16,
        d: usize,
    },
    NFromW {
        r: u16,
        s: usize,
        mask: u64,
    },
}

// Stop Debug formatting at the variant boundary, before any operand payload.
#[derive(Default)]
struct LowerVariantName(String);
impl std::fmt::Write for LowerVariantName {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = text.find(['{', '(']).unwrap_or(text.len());
        self.0.push_str(&text[..end]);
        if end < text.len() {
            Err(std::fmt::Error)
        } else {
            Ok(())
        }
    }
}

pub(super) struct WordProgram {
    ops: Vec<WordOp>,
    regs: usize,
    wide_words: usize,
    scratch: usize,
    constants: Vec<Box<[u64]>>,
}
impl WordProgram {
    fn reject_lower(insn: Option<&TsInsn>, reason: &str) -> Option<Self> {
        if env_is_one_cached!("XEZIM_MULTICORE_LOWER_CENSUS") {
            use std::fmt::Write;
            let mut name = LowerVariantName::default();
            if let Some(insn) = insn {
                let _ = write!(&mut name, "{insn:?}");
            }
            eprintln!(
                "[PROF] multicore_word_reject reason={} insn={}",
                reason,
                name.0.trim()
            );
        }
        None
    }

    pub(super) fn lower(ts: TwoStateBlock) -> Option<Self> {
        Self::lower_with_options(
            ts,
            env_is_one_cached!("XEZIM_MULTICORE_REPLICATION"),
            env_is_one_cached!("XEZIM_MULTICORE_WIDE_RANGE"),
        )
    }

    fn replication_factor(w: u8, count: u8) -> Option<(u64, u64)> {
        if w == 0 || w > 64 || count == 0 || u32::from(w) * u32::from(count) > 64 {
            return None;
        }
        let mask = u64::MAX >> (64 - w);
        let factor =
            (0..u32::from(count)).fold(0, |factor, i| factor | (1u64 << (i * u32::from(w))));
        Some((mask, factor))
    }

    fn lower_with_options(ts: TwoStateBlock, replication: bool, wide_range: bool) -> Option<Self> {
        if ts.has_wait {
            return Self::reject_lower(None, "wait");
        }
        let wide_words = match (ts.has_wide, ts.wide_words) {
            (false, 0) => 0,
            (true, n @ 1..=8) if ts.num_regs <= u16::MAX as u32 + 1 => n as usize,
            _ => return Self::reject_lower(None, "wide_bank"),
        };
        let regs = ts.num_regs as usize;
        let Some(scratch) = regs.checked_mul(wide_words + 1) else {
            return Self::reject_lower(None, "scratch_capacity");
        };
        let wide_offset = |r: u16| {
            (wide_words != 0 && (r as usize) < regs).then_some(regs + r as usize * wide_words)
        };
        let fits_wide = |w: u32| w != 0 && w as usize <= wide_words * 64;
        let mut constants = Vec::new();
        let mut ops = Vec::with_capacity(ts.insns.len());
        for (pc, insn) in ts.insns.iter().enumerate() {
            let target = match insn {
                TsInsn::BrFalse { t, .. }
                | TsInsn::BrNz { t, .. }
                | TsInsn::BrSigFalse { t, .. }
                | TsInsn::Jmp { t }
                | TsInsn::LoadSigBrNz { t, .. }
                | TsInsn::BrFalseLoadSig { t, .. }
                | TsInsn::EqBrFalse { t, .. }
                | TsInsn::EqCBrFalse { t, .. } => Some(*t as usize),
                _ => None,
            };
            if target.is_some_and(|t| t <= pc || t > ts.insns.len()) {
                return Self::reject_lower(Some(insn), "branch_target");
            }
            // Keep one operation per instruction: fused branch targets already
            // refer to this stream, including its one-past-the-end exit.
            ops.push(match insn {
                TsInsn::Repl { d, s, w, count } if replication => {
                    let Some((mask, factor)) = Self::replication_factor(*w, *count) else {
                        return Self::reject_lower(Some(insn), "replication_shape");
                    };
                    WordOp::Repl {
                        d: *d,
                        s: *s,
                        mask,
                        factor,
                    }
                }
                TsInsn::LoadSigRepl {
                    dl,
                    sig,
                    d,
                    w,
                    count,
                } if replication => {
                    let Some((mask, factor)) = Self::replication_factor(*w, *count) else {
                        return Self::reject_lower(Some(insn), "replication_shape");
                    };
                    WordOp::LoadSigRepl {
                        dl: *dl,
                        sig: *sig,
                        d: *d,
                        mask,
                        factor,
                    }
                }
                TsInsn::WRepl { d, s, w, count } if replication => {
                    let total = u32::from(*w) * u32::from(*count);
                    if *w == 0 || *w > 64 || *count == 0 || total <= 64 || !fits_wide(total) {
                        return Self::reject_lower(Some(insn), "replication_shape");
                    }
                    WordOp::WRepl {
                        d: wide_offset(*d)?,
                        s: *s,
                        w: *w,
                        count: *count,
                    }
                }
                TsInsn::WSigRange { d, sig, lo, w }
                    if wide_range
                        && *w > 64
                        && fits_wide(u32::from(*w))
                        && lo.checked_add(*w - 1).is_some() =>
                {
                    WordOp::WSigRange {
                        d: wide_offset(*d)?,
                        sig: *sig,
                        lo: *lo,
                        w: *w,
                    }
                }
                TsInsn::WLoadSig { d, sig } => WordOp::WLoadSig {
                    d: wide_offset(*d)?,
                    sig: *sig,
                },
                TsInsn::WConst { d, v } if !v.is_empty() && v.len() <= wide_words => {
                    let d = wide_offset(*d)?;
                    let k = constants.len();
                    constants.push(v.clone());
                    WordOp::WConst { d, k }
                }
                TsInsn::WStoreNba { sig, s, w } if fits_wide(*w) => WordOp::WStoreNba {
                    sig: *sig,
                    s: wide_offset(*s)?,
                    w: *w,
                },
                TsInsn::WRangeStoreNba { sig, lo, s, w } if fits_wide(*w) => {
                    WordOp::WRangeStoreNba {
                        sig: *sig,
                        lo: *lo,
                        s: wide_offset(*s)?,
                        w: *w,
                    }
                }
                TsInsn::WMask { d, w } if fits_wide(u32::from(*w)) => WordOp::WMask {
                    d: wide_offset(*d)?,
                    w: *w,
                },
                TsInsn::WFromN { r } => WordOp::WFromN {
                    r: *r,
                    d: wide_offset(*r)?,
                },
                TsInsn::NFromW { r, mask } => WordOp::NFromW {
                    r: *r,
                    s: wide_offset(*r)?,
                    mask: *mask,
                },
                TsInsn::BrFalse { s, t } => WordOp::BrFalse { s: *s, t: *t },
                TsInsn::BrNz { s, t } => WordOp::BrNz { s: *s, t: *t },
                TsInsn::BrSigFalse { sig, bit, t } => WordOp::BrSigFalse {
                    sig: *sig,
                    bit: *bit,
                    t: *t,
                },
                TsInsn::Jmp { t } => WordOp::Jmp { t: *t },
                TsInsn::LoadSigBrNz { dl, sig, t } => WordOp::LoadSigBrNz {
                    dl: *dl,
                    sig: *sig,
                    t: *t,
                },
                TsInsn::BrFalseLoadSig { s, t, dl, sig } => WordOp::BrFalseLoadSig {
                    s: *s,
                    t: *t,
                    dl: *dl,
                    sig: *sig,
                },
                TsInsn::EqBrFalse { d, a, b, t } => WordOp::EqBrFalse {
                    d: *d,
                    a: *a,
                    b: *b,
                    t: *t,
                },
                TsInsn::EqCBrFalse { d, s, k, t } => WordOp::EqCBrFalse {
                    d: *d,
                    s: *s,
                    k: *k,
                    t: *t,
                },
                TsInsn::LoadSig { d, sig } => WordOp::LoadSig { d: *d, sig: *sig },
                TsInsn::Const { d, v } => WordOp::Const { d: *d, v: *v },
                TsInsn::SigBit { d, sig, bit } => WordOp::SigBit {
                    d: *d,
                    sig: *sig,
                    bit: *bit,
                },
                TsInsn::SigRange { d, sig, lo, mask } => WordOp::SigRange {
                    d: *d,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                },
                TsInsn::SigBitW { d, sig, bit } => WordOp::SigSlice {
                    d: *d,
                    sig: *sig,
                    lo: *bit,
                    w: 1,
                    mask: 1,
                },
                TsInsn::SigRangeW {
                    d,
                    sig,
                    lo,
                    w,
                    mask,
                } if (1..=64).contains(w) => WordOp::SigSlice {
                    d: *d,
                    sig: *sig,
                    lo: *lo,
                    w: *w,
                    mask: *mask,
                },
                TsInsn::Bit { d, s, bit } => WordOp::Bit {
                    d: *d,
                    s: *s,
                    bit: *bit,
                },
                TsInsn::Range { d, s, lo, mask } => WordOp::Range {
                    d: *d,
                    s: *s,
                    lo: *lo,
                    mask: *mask,
                },
                TsInsn::Xor { d, a, b } => WordOp::Xor {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::And { d, a, b } => WordOp::And {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Or { d, a, b } => WordOp::Or {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Sel { d, c, a, b, .. } => WordOp::Sel {
                    d: *d,
                    c: *c,
                    a: *a,
                    b: *b,
                },
                TsInsn::Not { d, s, mask } => WordOp::Not {
                    d: *d,
                    s: *s,
                    mask: *mask,
                },
                TsInsn::XorC { d, s, k } => WordOp::XorC {
                    d: *d,
                    s: *s,
                    k: *k,
                },
                TsInsn::MaskEq { d, s, mask, v } => WordOp::MaskEq {
                    d: *d,
                    s: *s,
                    mask: *mask,
                    v: *v,
                },
                TsInsn::EqC { d, s, k } => WordOp::EqC {
                    d: *d,
                    s: *s,
                    k: *k,
                },
                TsInsn::Add { d, a, b, mask } => WordOp::Add {
                    d: *d,
                    a: *a,
                    b: *b,
                    mask: *mask,
                },
                TsInsn::AddC { d, s, k, mask } => WordOp::AddC {
                    d: *d,
                    s: *s,
                    k: *k,
                    mask: *mask,
                },
                TsInsn::Shl { d, a, b, w, mask } => WordOp::Shl {
                    d: *d,
                    a: *a,
                    b: *b,
                    w: *w,
                    mask: *mask,
                },
                TsInsn::Shr { d, a, b, w } => WordOp::Shr {
                    d: *d,
                    a: *a,
                    b: *b,
                    w: *w,
                },
                TsInsn::Sub { d, a, b, mask } => WordOp::Sub {
                    d: *d,
                    a: *a,
                    b: *b,
                    mask: *mask,
                },
                TsInsn::Eq { d, a, b } => WordOp::Eq {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Neq { d, a, b } => WordOp::Neq {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::CmpS {
                    d,
                    a,
                    b,
                    kind,
                    sa,
                    sb,
                } => WordOp::CmpS {
                    d: *d,
                    a: *a,
                    b: *b,
                    kind: *kind,
                    sa: *sa,
                    sb: *sb,
                },
                TsInsn::BitDyn { d, s, i, w } => WordOp::BitDyn {
                    d: *d,
                    s: *s,
                    i: *i,
                    w: *w,
                },
                TsInsn::LoadSigNot { dl, d, sig } => WordOp::LoadSigNot {
                    dl: *dl,
                    d: *d,
                    sig: *sig,
                },
                TsInsn::SigRangeEqC {
                    dr,
                    d,
                    sig,
                    lo,
                    w,
                    k,
                } => WordOp::SigRangeEqC {
                    dr: *dr,
                    d: *d,
                    sig: *sig,
                    lo: *lo,
                    w: *w,
                    k: *k,
                },
                TsInsn::LoadSigLogAnd { dl, sig, d, a, b } => WordOp::LoadSigLogAnd {
                    dl: *dl,
                    sig: *sig,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::LogNotAnd { dn, s, d, a, b } => WordOp::LogNotAnd {
                    dn: *dn,
                    s: *s,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::LogNotLogAnd { dn, s, d, a, b } => WordOp::LogNotLogAnd {
                    dn: *dn,
                    s: *s,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::SigBitNot { db, d, sig, bit } => WordOp::SigBitNot {
                    db: *db,
                    d: *d,
                    sig: *sig,
                    bit: *bit,
                },
                TsInsn::LoadSig2 { d1, sig1, d2, sig2 } => WordOp::LoadSig2 {
                    d1: *d1,
                    sig1: *sig1,
                    d2: *d2,
                    sig2: *sig2,
                },
                TsInsn::SigBit2 {
                    d1,
                    sig1,
                    bit1,
                    d2,
                    sig2,
                    bit2,
                } => WordOp::SigBit2 {
                    d1: *d1,
                    sig1: *sig1,
                    bit1: *bit1,
                    d2: *d2,
                    sig2: *sig2,
                    bit2: *bit2,
                },
                TsInsn::LoadSigLogOr { dl, sig, d, a, b } => WordOp::LoadSigLogOr {
                    dl: *dl,
                    sig: *sig,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::LoadSigAnd { dl, sig, d, a, b } => WordOp::LoadSigAnd {
                    dl: *dl,
                    sig: *sig,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::LoadSigSigRange {
                    dl,
                    sig,
                    d,
                    sig2,
                    lo,
                    mask,
                } => WordOp::LoadSigSigRange {
                    dl: *dl,
                    sig: *sig,
                    d: *d,
                    sig2: *sig2,
                    lo: *lo,
                    mask: *mask,
                },
                TsInsn::SigRangeAnd {
                    dr,
                    sig,
                    lo,
                    mask,
                    d,
                    a,
                    b,
                } => WordOp::SigRangeAnd {
                    dr: *dr,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::SigRangeEq {
                    dr,
                    sig,
                    lo,
                    mask,
                    d,
                    a,
                    b,
                } => WordOp::SigRangeEq {
                    dr: *dr,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::ConstEq { dc, v, d, a, b } => WordOp::ConstEq {
                    dc: *dc,
                    v: *v,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::AndOr {
                    d1,
                    a1,
                    b1,
                    d,
                    a,
                    b,
                } => WordOp::AndOr {
                    d1: *d1,
                    a1: *a1,
                    b1: *b1,
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Lt { d, a, b } => WordOp::Lt {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Leq { d, a, b } => WordOp::Leq {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Gt { d, a, b } => WordOp::Gt {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Geq { d, a, b } => WordOp::Geq {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::LogNot { d, s } => WordOp::LogNot { d: *d, s: *s },
                TsInsn::RedOr { d, s } => WordOp::RedOr { d: *d, s: *s },
                TsInsn::RedAnd { d, s, mask } => WordOp::RedAnd {
                    d: *d,
                    s: *s,
                    mask: *mask,
                },
                TsInsn::LogAnd { d, a, b } => WordOp::LogAnd {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::LogOr { d, a, b } => WordOp::LogOr {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                TsInsn::Concat2 { d, a, b, wb, .. } => WordOp::Concat2 {
                    d: *d,
                    a: *a,
                    b: *b,
                    wb: *wb,
                },
                TsInsn::Concat3 {
                    d,
                    a,
                    wa: _,
                    b,
                    wb,
                    c,
                    wc,
                } => WordOp::Concat3 {
                    d: *d,
                    a: *a,
                    b: *b,
                    wb: *wb,
                    c: *c,
                    wc: *wc,
                },
                TsInsn::Mask { d, mask } => WordOp::Mask { d: *d, mask: *mask },
                TsInsn::Mul { d, a, b, mask } => WordOp::Mul {
                    d: *d,
                    a: *a,
                    b: *b,
                    mask: *mask,
                },
                TsInsn::StoreNba { sig, s, w, mask } => WordOp::StoreNba {
                    sig: *sig,
                    s: *s,
                    w: *w,
                    mask: *mask,
                },
                TsInsn::ConstStoreNba { sig, v, w } => WordOp::ConstStoreNba {
                    sig: *sig,
                    v: *v,
                    w: *w,
                },
                TsInsn::RangeStoreNba {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                }
                | TsInsn::RangeStoreNbaW {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                } if hi.checked_sub(*lo).is_some_and(|n| n < 64) => WordOp::RangeStoreNba {
                    sig: *sig,
                    hi: *hi,
                    lo: *lo,
                    s: *s,
                    mask: *mask,
                },
                TsInsn::RangeFillNbaW { sig, hi, lo, bit } if *hi >= *lo && *bit <= 1 => {
                    WordOp::RangeFillNba {
                        sig: *sig,
                        hi: *hi,
                        lo: *lo,
                        bit: *bit,
                    }
                }
                _ => return Self::reject_lower(Some(insn), "unsupported_or_guard"),
            });
        }
        Some(Self {
            ops,
            regs,
            wide_words,
            scratch,
            constants,
        })
    }
    pub(super) fn execute(
        &self,
        signals: &[Value],
        signed: &[bool],
        regs: &mut Vec<u64>,
        out: &mut Vec<NbaFast>,
        bi: u32,
        duplicates: bool,
    ) -> bool {
        debug_assert_eq!(self.scratch, self.regs * (self.wide_words + 1));
        if regs.len() < self.scratch {
            regs.resize(self.scratch, 0);
        }
        let base = out.len();
        macro_rules! r {
            ($i:expr) => {
                regs[$i as usize]
            };
        }
        macro_rules! planes {
            ($s:expr) => {
                signals[$s].raw_bits()
            };
        }
        macro_rules! xbail {
            () => {{
                out.truncate(base);
                return false;
            }};
        }
        let mut stream = self.ops.iter();
        while let Some(insn) = stream.next() {
            match insn {
                WordOp::Repl { d, s, mask, factor } => {
                    r!(*d) = (r!(*s) & mask).wrapping_mul(*factor);
                }
                WordOp::LoadSigRepl {
                    dl,
                    sig,
                    d,
                    mask,
                    factor,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    r!(*d) = (v & mask).wrapping_mul(*factor);
                }
                WordOp::WRepl { d, s, w, count } => {
                    // Capture the narrow source before writing an aliased wide bank.
                    let part = r!(*s) & (u64::MAX >> (64 - u32::from(*w)));
                    let words = &mut regs[*d..*d + self.wide_words];
                    words.fill(0);
                    if *w == 1 {
                        if part != 0 {
                            words.fill(u64::MAX);
                        }
                    } else {
                        for i in 0..usize::from(*count) {
                            let bit = i * usize::from(*w);
                            let word = bit / 64;
                            let shift = bit % 64;
                            words[word] |= part << shift;
                            if shift != 0 && usize::from(*w) > 64 - shift {
                                words[word + 1] |= part >> (64 - shift);
                            }
                        }
                    }
                    let total = usize::from(*w) * usize::from(*count);
                    let top = (total - 1) / 64;
                    words[top] &= u64::MAX >> (63 - ((total - 1) % 64));
                    words[top + 1..].fill(0);
                }
                WordOp::WSigRange { d, sig, lo, w } => {
                    let signal = &signals[*sig as usize];
                    if u32::from(*lo) + u32::from(*w) > signal.width {
                        xbail!();
                    }
                    let words = &mut regs[*d..*d + self.wide_words];
                    words.fill(0);
                    for off in (0..usize::from(*w)).step_by(64) {
                        let width = (usize::from(*w) - off).min(64) as u16;
                        let (v, x) = Simulator::raw_bits_slice(signal, *lo + off as u16, width);
                        if x != 0 {
                            xbail!();
                        }
                        words[off / 64] = v;
                    }
                }
                WordOp::WLoadSig { d, sig } => {
                    if !signals[*sig as usize].words_if_clean(&mut regs[*d..*d + self.wide_words]) {
                        xbail!();
                    }
                }
                WordOp::WConst { d, k } => {
                    let words = &mut regs[*d..*d + self.wide_words];
                    words.fill(0);
                    let value = &self.constants[*k];
                    words[..value.len()].copy_from_slice(value);
                }
                WordOp::WStoreNba { sig, s, w } => {
                    let id = *sig as usize;
                    if *w != signals[id].width {
                        xbail!();
                    }
                    let mut value = Value::from_words(&regs[*s..*s + self.wide_words], *w);
                    value.is_signed = signed[id];
                    Self::store(out, base, id, value, bi, duplicates);
                }
                WordOp::WRangeStoreNba { sig, lo, s, w } => {
                    let id = *sig as usize;
                    let current = &signals[id];
                    if lo.checked_add(*w).is_none_or(|end| end > current.width) {
                        xbail!();
                    }
                    let part = Value::from_words(&regs[*s..*s + self.wide_words], *w);
                    NbaFast::queue_partial_from(out, base, current, id, *lo, &part, bi);
                }
                WordOp::WMask { d, w } => {
                    let words = &mut regs[*d..*d + self.wide_words];
                    let top = (*w as usize - 1) / 64;
                    let count = (*w as usize - 1) % 64 + 1;
                    words[top] &= u64::MAX >> (64 - count);
                    words[top + 1..].fill(0);
                }
                WordOp::WFromN { r, d } => {
                    let value = r!(*r);
                    let words = &mut regs[*d..*d + self.wide_words];
                    words.fill(0);
                    words[0] = value;
                }
                WordOp::NFromW { r, s, mask } => r!(*r) = regs[*s] & mask,
                WordOp::BrFalse { s, t } => {
                    if r!(*s) == 0 {
                        stream = self.ops[*t as usize..].iter();
                    }
                }
                WordOp::BrNz { s, t } => {
                    if r!(*s) != 0 {
                        stream = self.ops[*t as usize..].iter();
                    }
                }
                WordOp::Jmp { t } => stream = self.ops[*t as usize..].iter(),
                WordOp::BrSigFalse { sig, bit, t } => {
                    let signal = &signals[*sig as usize];
                    let (v, x) = if *bit == u32::MAX {
                        signal.raw_bits()
                    } else if (*bit as usize) < signal.width as usize {
                        signal.slice_bits_swar(*bit as usize, 1)
                    } else {
                        xbail!();
                    };
                    if x != 0 {
                        xbail!();
                    }
                    if v == 0 {
                        stream = self.ops[*t as usize..].iter();
                    }
                }
                WordOp::LoadSigBrNz { dl, sig, t } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    if v != 0 {
                        stream = self.ops[*t as usize..].iter();
                    }
                }
                WordOp::BrFalseLoadSig { s, t, dl, sig } => {
                    if r!(*s) == 0 {
                        stream = self.ops[*t as usize..].iter();
                    } else {
                        let (v, x) = planes!(*sig as usize);
                        if x != 0 {
                            xbail!();
                        }
                        r!(*dl) = v;
                    }
                }
                WordOp::EqBrFalse { d, a, b, t } => {
                    r!(*d) = (r!(*a) == r!(*b)) as u64;
                    if r!(*d) == 0 {
                        stream = self.ops[*t as usize..].iter();
                    }
                }
                WordOp::EqCBrFalse { d, s, k, t } => {
                    r!(*d) = (r!(*s) == *k) as u64;
                    if r!(*d) == 0 {
                        stream = self.ops[*t as usize..].iter();
                    }
                }
                WordOp::LoadSig { d, sig } => {
                    // Read only the immutable epoch; unknowns retry privately.
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d) = v;
                }
                WordOp::Const { d, v } => r!(*d) = *v,
                WordOp::SigBit { d, sig, bit } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d) = (v >> bit) & 1;
                }
                WordOp::SigRange { d, sig, lo, mask } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d) = (v >> lo) & mask;
                }
                WordOp::SigSlice {
                    d,
                    sig,
                    lo,
                    w,
                    mask,
                } => {
                    let signal = &signals[*sig as usize];
                    if u32::from(*lo) + u32::from(*w) > signal.width {
                        xbail!();
                    }
                    let (v, x) = signal.slice_bits_swar(*lo as usize, *w as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d) = v & mask;
                }
                WordOp::Bit { d, s, bit } => {
                    r!(*d) = (r!(*s) >> bit) & 1;
                }
                WordOp::Range { d, s, lo, mask } => {
                    r!(*d) = (r!(*s) >> lo) & mask;
                }
                WordOp::Xor { d, a, b } => {
                    r!(*d) = r!(*a) ^ r!(*b);
                }
                WordOp::And { d, a, b } => {
                    r!(*d) = r!(*a) & r!(*b);
                }
                WordOp::Or { d, a, b } => {
                    r!(*d) = r!(*a) | r!(*b);
                }
                WordOp::Sel { d, c, a, b, .. } => {
                    r!(*d) = if r!(*c) != 0 { r!(*a) } else { r!(*b) };
                }
                WordOp::Not { d, s, mask } => {
                    r!(*d) = !r!(*s) & mask;
                }
                WordOp::XorC { d, s, k } => {
                    r!(*d) = r!(*s) ^ k;
                }
                WordOp::MaskEq { d, s, mask, v } => {
                    r!(*d) = ((r!(*s) & mask) == *v) as u64;
                }
                WordOp::EqC { d, s, k } => {
                    r!(*d) = (r!(*s) == *k) as u64;
                }
                WordOp::Add { d, a, b, mask } => {
                    r!(*d) = r!(*a).wrapping_add(r!(*b)) & mask;
                }
                WordOp::AddC { d, s: sr, k, mask } => {
                    r!(*d) = r!(*sr).wrapping_add(*k) & mask;
                }
                WordOp::Shl { d, a, b, w, mask } => {
                    let amt = r!(*b);
                    r!(*d) = if amt >= *w as u64 {
                        0
                    } else {
                        (r!(*a) << amt) & mask
                    };
                }
                WordOp::Shr { d, a, b, w } => {
                    let amt = r!(*b);
                    r!(*d) = if amt >= *w as u64 { 0 } else { r!(*a) >> amt };
                }
                WordOp::Sub { d, a, b, mask } => {
                    r!(*d) = r!(*a).wrapping_sub(r!(*b)) & mask;
                }
                WordOp::Eq { d, a, b } => {
                    r!(*d) = (r!(*a) == r!(*b)) as u64;
                }
                WordOp::Neq { d, a, b } => {
                    r!(*d) = (r!(*a) != r!(*b)) as u64;
                }
                WordOp::CmpS {
                    d,
                    a,
                    b,
                    kind,
                    sa,
                    sb,
                } => {
                    let x = ((r!(*a) << *sa) as i64) >> *sa;
                    let y = ((r!(*b) << *sb) as i64) >> *sb;
                    r!(*d) = match *kind {
                        0 => x < y,
                        1 => x <= y,
                        2 => x > y,
                        _ => x >= y,
                    } as u64;
                }
                WordOp::BitDyn { d, s, i, w } => {
                    let idx = r!(*i);
                    if idx >= *w as u64 {
                        xbail!();
                    }
                    r!(*d) = (r!(*s) >> idx) & 1;
                }
                WordOp::LoadSigNot { dl, d, sig } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    r!(*d) = (v == 0) as u64;
                }
                WordOp::SigRangeEqC {
                    dr,
                    d,
                    sig,
                    lo,
                    w,
                    k,
                } => {
                    let mask = if *w >= 64 { u64::MAX } else { (1u64 << *w) - 1 };
                    let mask = &mask;
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    let rv = (v >> lo) & mask;
                    r!(*dr) = rv;
                    r!(*d) = (rv == *k) as u64;
                }
                WordOp::LoadSigLogAnd { dl, sig, d, a, b } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    r!(*d) = (r!(*a) != 0 && r!(*b) != 0) as u64;
                }
                WordOp::LogNotAnd { dn, s, d, a, b } => {
                    r!(*dn) = (r!(*s) == 0) as u64;
                    r!(*d) = r!(*a) & r!(*b);
                }
                WordOp::LogNotLogAnd { dn, s, d, a, b } => {
                    r!(*dn) = (r!(*s) == 0) as u64;
                    r!(*d) = (r!(*a) != 0 && r!(*b) != 0) as u64;
                }
                WordOp::SigBitNot { db, d, sig, bit } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    let bv = (v >> bit) & 1;
                    r!(*db) = bv;
                    r!(*d) = (bv == 0) as u64;
                }
                WordOp::LoadSig2 { d1, sig1, d2, sig2 } => {
                    let (v, x) = planes!(*sig1 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d1) = v;
                    let (v, x) = planes!(*sig2 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d2) = v;
                }
                WordOp::SigBit2 {
                    d1,
                    sig1,
                    bit1,
                    d2,
                    sig2,
                    bit2,
                } => {
                    let (v, x) = planes!(*sig1 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d1) = (v >> bit1) & 1;
                    let (v, x) = planes!(*sig2 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d2) = (v >> bit2) & 1;
                }
                WordOp::LoadSigLogOr { dl, sig, d, a, b } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    r!(*d) = (r!(*a) != 0 || r!(*b) != 0) as u64;
                }
                WordOp::LoadSigAnd { dl, sig, d, a, b } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    r!(*d) = r!(*a) & r!(*b);
                }
                WordOp::LoadSigSigRange {
                    dl,
                    sig,
                    d,
                    sig2,
                    lo,
                    mask,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dl) = v;
                    let (v, x) = planes!(*sig2 as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*d) = (v >> lo) & mask;
                }
                WordOp::SigRangeAnd {
                    dr,
                    sig,
                    lo,
                    mask,
                    d,
                    a,
                    b,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dr) = (v >> lo) & mask;
                    r!(*d) = r!(*a) & r!(*b);
                }
                WordOp::SigRangeEq {
                    dr,
                    sig,
                    lo,
                    mask,
                    d,
                    a,
                    b,
                } => {
                    let (v, x) = planes!(*sig as usize);
                    if x != 0 {
                        xbail!();
                    }
                    r!(*dr) = (v >> lo) & mask;
                    r!(*d) = (r!(*a) == r!(*b)) as u64;
                }
                WordOp::ConstEq { dc, v, d, a, b } => {
                    r!(*dc) = *v;
                    r!(*d) = (r!(*a) == r!(*b)) as u64;
                }
                WordOp::AndOr {
                    d1,
                    a1,
                    b1,
                    d,
                    a,
                    b,
                } => {
                    r!(*d1) = r!(*a1) & r!(*b1);
                    r!(*d) = r!(*a) | r!(*b);
                }
                WordOp::Lt { d, a, b } => {
                    r!(*d) = (r!(*a) < r!(*b)) as u64;
                }
                WordOp::Leq { d, a, b } => {
                    r!(*d) = (r!(*a) <= r!(*b)) as u64;
                }
                WordOp::Gt { d, a, b } => {
                    r!(*d) = (r!(*a) > r!(*b)) as u64;
                }
                WordOp::Geq { d, a, b } => {
                    r!(*d) = (r!(*a) >= r!(*b)) as u64;
                }
                WordOp::LogNot { d, s } => {
                    r!(*d) = (r!(*s) == 0) as u64;
                }
                WordOp::RedOr { d, s } => {
                    r!(*d) = (r!(*s) != 0) as u64;
                }
                WordOp::RedAnd { d, s, mask } => {
                    r!(*d) = (r!(*s) == *mask) as u64;
                }
                WordOp::LogAnd { d, a, b } => {
                    r!(*d) = (r!(*a) != 0 && r!(*b) != 0) as u64;
                }
                WordOp::LogOr { d, a, b } => {
                    r!(*d) = (r!(*a) != 0 || r!(*b) != 0) as u64;
                }
                WordOp::Concat2 { d, a, b, wb } => {
                    r!(*d) = (r!(*a) << *wb) | r!(*b);
                }
                WordOp::Concat3 { d, a, b, wb, c, wc } => {
                    let acc = (r!(*a) << *wb) | r!(*b);
                    r!(*d) = (acc << *wc) | r!(*c);
                }
                WordOp::Mask { d, mask } => {
                    r!(*d) &= mask;
                }
                WordOp::Mul { d, a, b, mask } => {
                    r!(*d) = r!(*a).wrapping_mul(r!(*b)) & mask;
                }
                WordOp::StoreNba { sig, s, w, mask } => {
                    let id = *sig as usize;
                    let mut value = Value::from_u64(regs[*s as usize] & mask, *w);
                    value.is_signed = signed[id];
                    Self::store(out, base, id, value, bi, duplicates);
                }
                WordOp::ConstStoreNba { sig, v, w } => {
                    let id = *sig as usize;
                    let mut value = Value::from_u64(*v, *w);
                    value.is_signed = signed[id];
                    Self::store(out, base, id, value, bi, duplicates);
                }
                WordOp::RangeStoreNba {
                    sig,
                    hi,
                    lo,
                    s,
                    mask,
                } => {
                    let id = *sig as usize;
                    let current = &signals[id];
                    if *hi >= current.width {
                        xbail!();
                    }
                    let part = Value::from_u64(r!(*s) & mask, hi - lo + 1);
                    NbaFast::queue_partial_from(out, base, current, id, *lo, &part, bi);
                }
                WordOp::RangeFillNba { sig, hi, lo, bit } => {
                    let id = *sig as usize;
                    let current = &signals[id];
                    if *hi >= current.width {
                        xbail!();
                    }
                    let width = hi - lo + 1;
                    let part = if *bit == 0 {
                        Value::zero(width)
                    } else {
                        Value::ones(width)
                    };
                    NbaFast::queue_partial_from(out, base, current, id, *lo, &part, bi);
                }
            }
        }
        true
    }
    fn store(
        out: &mut Vec<NbaFast>,
        base: usize,
        id: usize,
        value: Value,
        bi: u32,
        duplicates: bool,
    ) {
        if duplicates && let Some(entry) = out[base..].iter_mut().find(|n| n.signal_id == id) {
            entry.value = value;
            entry.mask = None;
            return;
        }
        out.push(NbaFast {
            signal_id: id,
            value,
            block_index: bi,
            mask: None,
        });
    }
}

#[cfg(test)]
#[path = "parallel_word_tests.rs"]
mod tests;
