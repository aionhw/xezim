//! X-plane two-state executor.
//!
//! The two-state executors bail on the first x/z bit they load and the
//! block re-runs on the four-state VM. On the SoC benchmarks most of what
//! the VM still executes is such re-runs: tiny muxes and range copies
//! whose unselected operand, or an unwritten register file, holds x.
//! This executor runs the SAME lowered stream on (value, x/z) register
//! pairs with the four-state rules the VM applies (`xezim_core::Value`):
//! bitwise ops are Kleene, `&&`/`||`/`!` reduce through "definite 1 /
//! definite 0 / unknown", equality is x unless a known bit differs,
//! ordering and arithmetic with any x operand are all-x, a shift by a
//! known amount moves both planes, `if (x)` takes the else arm, a `case`
//! selector with x takes the default arm, an x index reads all-x and
//! writes nothing. Stores go through the x-plane store helpers the
//! executors already have. It never aborts, so it is only entered after
//! the two-state run bailed on an x read (and the stores that run made
//! were restored), and only for streams the narrow banks can hold —
//! `TwoStateBlock::tsx` says which.
use super::*;
use crate::compiler::bytecode::TsInsn;

#[inline(always)]
fn t3(v: u64, x: u64) -> Option<bool> {
    if v & !x != 0 {
        Some(true)
    } else if x != 0 {
        None
    } else {
        Some(false)
    }
}
#[inline(always)]
fn from_t3(t: Option<bool>) -> (u64, u64) {
    match t {
        Some(true) => (1, 0),
        Some(false) => (0, 0),
        None => (0, 1),
    }
}
#[inline(always)]
fn and3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}
#[inline(always)]
fn or3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}
#[inline(always)]
fn not3(a: Option<bool>) -> Option<bool> {
    a.map(|t| !t)
}
#[inline(always)]
fn and_p(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let any = ax | bx;
    (av & bv & !any, any & !((!av & !ax) | (!bv & !bx)))
}
#[inline(always)]
fn or_p(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let known1 = (av & !ax) | (bv & !bx);
    (known1, (ax | bx) & !known1)
}
#[inline(always)]
fn xor_p(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let rx = ax | bx;
    ((av ^ bv) & !rx, rx)
}
/// `==` on same-width unsigned operands: known mismatch → 0, else x.
#[inline(always)]
fn eq_p(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    if ax | bx == 0 {
        ((av == bv) as u64, 0)
    } else if (av ^ bv) & !ax & !bx != 0 {
        (0, 0)
    } else {
        (0, 1)
    }
}
#[inline(always)]
fn ne_p(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let (v, x) = eq_p(av, ax, bv, bx);
    if x != 0 { (0, 1) } else { (v ^ 1, 0) }
}
/// `c ? a : b` with an x selector: bits where both arms agree and are
/// known keep their value, everything else is x (`Value::merge_unknown`).
#[inline(always)]
fn merge_p(av: u64, ax: u64, bv: u64, bx: u64, mask: u64) -> (u64, u64) {
    let agree = !ax & !bx & !(av ^ bv);
    (av & agree, !agree & mask)
}
#[inline(always)]
fn xmask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

impl Simulator {
    #[inline(always)]
    fn sig_planes(&self, sig: u32) -> (u64, u64) {
        match self.signal_inline_bits.get(sig as usize) {
            Some(sl) => (sl[0], sl[1]),
            None => self.signal_table[sig as usize].raw_bits(),
        }
    }

    fn ts_store_nba_xz(&mut self, id: usize, v: u64, x: u64, w: u32) {
        if x == 0 {
            self.ts_store_nba(id, v, w);
        } else {
            let mut val = Value::from_inline(v, x, w.max(1));
            val.is_signed = false;
            self.ts_store_nba_val(id, val);
        }
    }

    fn ts_range_store_nba_xz(&mut self, id: usize, lo: u32, hi: u32, v: u64, x: u64) {
        if let Some(i) = self.nba_fast_index.get(id) {
            let target = &mut self.nba_fast[i].value;
            let (base_v, base_x) = target.raw_bits();
            let (new_v, new_x) = Self::compose_inline_range_bits(base_v, base_x, v, x, lo, hi);
            target.set_inline_bits(new_v, new_x);
            target.is_signed = self.signal_signed[id];
        } else {
            let (base_v, base_x) = self.signal_table[id].raw_bits();
            let (new_v, new_x) = Self::compose_inline_range_bits(base_v, base_x, v, x, lo, hi);
            if new_v == base_v && new_x == base_x {
                self.prof_nba_elided += 1;
            } else {
                let mut nv = Value::from_inline(new_v, new_x, self.signal_widths[id]);
                nv.is_signed = self.signal_signed[id];
                self.nba_fast_index.insert(id, self.nba_fast.len());
                self.nba_fast.push(NbaFast {
                    block_index: 0,
                    signal_id: id,
                    value: nv,
                });
            }
        }
    }

    fn ts_wide_range_store_nba_xz(&mut self, id: usize, lo: u32, hi: u32, v: u64, x: u64) {
        let n = (hi - lo + 1) as usize;
        if let Some(i) = self.nba_fast_index.get(id) {
            self.nba_fast[i].value.splice_bits64(lo as usize, v, x, n);
        } else {
            let mut nv = self.signal_table[id].clone();
            if !nv.splice_bits64(lo as usize, v, x, n) {
                self.prof_nba_elided += 1;
            } else {
                nv.is_signed = self.signal_signed[id];
                self.nba_fast_index.insert(id, self.nba_fast.len());
                self.nba_fast.push(NbaFast {
                    block_index: 0,
                    signal_id: id,
                    value: nv,
                });
            }
        }
    }

    /// Run a narrow two-state stream on x-planes. Returns false only for
    /// an instruction the executor does not carry (the stream was not
    /// screened) — the caller then takes the four-state VM as before.
    #[inline(never)]
    pub(super) fn exec_two_state_x(&mut self, insns: &[TsInsn], num_regs: u32) -> bool {
        let n = num_regs as usize;
        if self.ts_regs.len() < n {
            self.ts_regs.resize(n, 0);
        }
        if self.ts_xregs.len() < n {
            self.ts_xregs.resize(n, 0);
        }
        // Same discipline as the two-state executors: the register vectors
        // never reallocate inside the block and no callee touches them.
        let vp: *mut u64 = self.ts_regs.as_mut_ptr();
        let xp: *mut u64 = self.ts_xregs.as_mut_ptr();
        macro_rules! v {
            ($i:expr) => {
                (*vp.add($i as usize))
            };
        }
        macro_rules! x {
            ($i:expr) => {
                (*xp.add($i as usize))
            };
        }
        macro_rules! set {
            ($d:expr, $p:expr) => {{
                let (pv, px) = $p;
                v!($d) = pv;
                x!($d) = px;
            }};
        }
        let len = insns.len();
        let mut pc = 0usize;
        while pc < len {
            let insn = &insns[pc];
            pc += 1;
            unsafe {
                match insn {
                    TsInsn::LoadSig { d, sig } => set!(*d, self.sig_planes(*sig)),
                    TsInsn::Const { d, v } => set!(*d, (*v, 0)),
                    TsInsn::SigBit { d, sig, bit } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*d, ((v >> bit) & 1, (x >> bit) & 1));
                    }
                    TsInsn::SigRange { d, sig, lo, mask } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*d, ((v >> lo) & mask, (x >> lo) & mask));
                    }
                    TsInsn::SigBitW { d, sig, bit } => {
                        set!(
                            *d,
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *bit, 1)
                        );
                    }
                    TsInsn::SigRangeW {
                        d,
                        sig,
                        lo,
                        w,
                        mask,
                    } => {
                        let (v, x) =
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *lo, *w);
                        set!(*d, (v & mask, x & mask));
                    }
                    TsInsn::Bit { d, s, bit } => {
                        set!(*d, ((v!(*s) >> bit) & 1, (x!(*s) >> bit) & 1))
                    }
                    TsInsn::Range { d, s, lo, mask } => {
                        set!(*d, ((v!(*s) >> lo) & mask, (x!(*s) >> lo) & mask))
                    }
                    TsInsn::Xor { d, a, b } => set!(*d, xor_p(v!(*a), x!(*a), v!(*b), x!(*b))),
                    TsInsn::And { d, a, b } => set!(*d, and_p(v!(*a), x!(*a), v!(*b), x!(*b))),
                    TsInsn::Or { d, a, b } => set!(*d, or_p(v!(*a), x!(*a), v!(*b), x!(*b))),
                    TsInsn::Sel { d, c, a, b, m } => {
                        let p = if x!(*c) != 0 {
                            merge_p(v!(*a), x!(*a), v!(*b), x!(*b), *m)
                        } else if v!(*c) != 0 {
                            (v!(*a), x!(*a))
                        } else {
                            (v!(*b), x!(*b))
                        };
                        set!(*d, p);
                    }
                    TsInsn::Not { d, s, mask } => {
                        set!(*d, (!v!(*s) & !x!(*s) & mask, x!(*s) & mask))
                    }
                    TsInsn::XorC { d, s, k } => set!(*d, ((v!(*s) ^ k) & !x!(*s), x!(*s))),
                    TsInsn::MaskEq { d, s, mask, v } => {
                        set!(*d, eq_p(v!(*s) & mask, x!(*s) & mask, *v, 0))
                    }
                    TsInsn::EqC { d, s, k } => set!(*d, eq_p(v!(*s), x!(*s), *k, 0)),
                    TsInsn::Add { d, a, b, mask } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, *mask)
                        } else {
                            (v!(*a).wrapping_add(v!(*b)) & mask, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::AddC { d, s, k, mask } => {
                        let p = if x!(*s) != 0 {
                            (0, *mask)
                        } else {
                            (v!(*s).wrapping_add(*k) & mask, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Sub { d, a, b, mask } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, *mask)
                        } else {
                            (v!(*a).wrapping_sub(v!(*b)) & mask, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Mul { d, a, b, mask } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, *mask)
                        } else {
                            (v!(*a).wrapping_mul(v!(*b)) & mask, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Shl { d, a, b, w, mask } => {
                        let p = if x!(*b) != 0 {
                            (0, *mask)
                        } else {
                            let amt = v!(*b);
                            if amt >= *w as u64 {
                                (0, 0)
                            } else {
                                ((v!(*a) << amt) & mask, (x!(*a) << amt) & mask)
                            }
                        };
                        set!(*d, p);
                    }
                    TsInsn::Shr { d, a, b, w } => {
                        let p = if x!(*b) != 0 {
                            (0, xmask(*w))
                        } else {
                            let amt = v!(*b);
                            if amt >= *w as u64 {
                                (0, 0)
                            } else {
                                (v!(*a) >> amt, x!(*a) >> amt)
                            }
                        };
                        set!(*d, p);
                    }
                    TsInsn::Eq { d, a, b } => set!(*d, eq_p(v!(*a), x!(*a), v!(*b), x!(*b))),
                    TsInsn::Neq { d, a, b } => set!(*d, ne_p(v!(*a), x!(*a), v!(*b), x!(*b))),
                    TsInsn::Lt { d, a, b } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, 1)
                        } else {
                            ((v!(*a) < v!(*b)) as u64, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Leq { d, a, b } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, 1)
                        } else {
                            ((v!(*a) <= v!(*b)) as u64, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Gt { d, a, b } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, 1)
                        } else {
                            ((v!(*a) > v!(*b)) as u64, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Geq { d, a, b } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, 1)
                        } else {
                            ((v!(*a) >= v!(*b)) as u64, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::CmpS {
                        d,
                        a,
                        b,
                        kind,
                        sa,
                        sb,
                    } => {
                        let p = if x!(*a) | x!(*b) != 0 {
                            (0, 1)
                        } else {
                            let xa = ((v!(*a) << *sa) as i64) >> *sa;
                            let yb = ((v!(*b) << *sb) as i64) >> *sb;
                            (
                                match *kind {
                                    0 => xa < yb,
                                    1 => xa <= yb,
                                    2 => xa > yb,
                                    _ => xa >= yb,
                                } as u64,
                                0,
                            )
                        };
                        set!(*d, p);
                    }
                    TsInsn::LoadSigNot { dl, d, sig } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*dl, (v, x));
                        set!(*d, from_t3(not3(t3(v, x))));
                    }
                    TsInsn::SigRangeEqC {
                        dr,
                        d,
                        sig,
                        lo,
                        w,
                        k,
                    } => {
                        let mask = xmask(*w as u32);
                        let (v, x) = self.sig_planes(*sig);
                        let (rv, rx) = ((v >> lo) & mask, (x >> lo) & mask);
                        set!(*dr, (rv, rx));
                        set!(*d, eq_p(rv, rx, *k, 0));
                    }
                    TsInsn::LoadSigLogAnd { dl, sig, d, a, b } => {
                        set!(*dl, self.sig_planes(*sig));
                        set!(*d, from_t3(and3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b)))));
                    }
                    TsInsn::LogNotAnd { dn, s, d, a, b } => {
                        set!(*dn, from_t3(not3(t3(v!(*s), x!(*s)))));
                        set!(*d, and_p(v!(*a), x!(*a), v!(*b), x!(*b)));
                    }
                    TsInsn::LogNotLogAnd { dn, s, d, a, b } => {
                        set!(*dn, from_t3(not3(t3(v!(*s), x!(*s)))));
                        set!(*d, from_t3(and3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b)))));
                    }
                    TsInsn::LogAndStore { d, a, b, sig, mask } => {
                        let (rv, rx) = from_t3(and3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b))));
                        set!(*d, (rv, rx));
                        self.ts_store_xz(*sig as usize, rv & mask, rx & mask);
                    }
                    TsInsn::AndRangeStore {
                        d,
                        a,
                        b,
                        sig,
                        hi,
                        lo,
                    } => {
                        let mask = xmask(*hi - *lo + 1);
                        let (rv, rx) = and_p(v!(*a), x!(*a), v!(*b), x!(*b));
                        set!(*d, (rv, rx));
                        self.ts_range_store_xz(*sig as usize, rv & mask, rx & mask, *lo, *hi);
                    }
                    TsInsn::SigBitNot { db, d, sig, bit } => {
                        let (v, x) = self.sig_planes(*sig);
                        let (bv, bx) = ((v >> bit) & 1, (x >> bit) & 1);
                        set!(*db, (bv, bx));
                        set!(*d, from_t3(not3(t3(bv, bx))));
                    }
                    TsInsn::LoadSig2 { d1, sig1, d2, sig2 } => {
                        set!(*d1, self.sig_planes(*sig1));
                        set!(*d2, self.sig_planes(*sig2));
                    }
                    TsInsn::SigBit2 {
                        d1,
                        sig1,
                        bit1,
                        d2,
                        sig2,
                        bit2,
                    } => {
                        let (v, x) = self.sig_planes(*sig1);
                        set!(*d1, ((v >> bit1) & 1, (x >> bit1) & 1));
                        let (v, x) = self.sig_planes(*sig2);
                        set!(*d2, ((v >> bit2) & 1, (x >> bit2) & 1));
                    }
                    TsInsn::LoadSigBrNz { dl, sig, t } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*dl, (v, x));
                        if t3(v, x) == Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::BrFalseLoadSig { s, t, dl, sig } => {
                        if t3(v!(*s), x!(*s)) != Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                        set!(*dl, self.sig_planes(*sig));
                    }
                    TsInsn::EqBrFalse { d, a, b, t } => {
                        let (rv, rx) = eq_p(v!(*a), x!(*a), v!(*b), x!(*b));
                        set!(*d, (rv, rx));
                        if t3(rv, rx) != Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::EqCBrFalse { d, s, k, t } => {
                        let (rv, rx) = eq_p(v!(*s), x!(*s), *k, 0);
                        set!(*d, (rv, rx));
                        if t3(rv, rx) != Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::LoadSigLogOr { dl, sig, d, a, b } => {
                        set!(*dl, self.sig_planes(*sig));
                        set!(*d, from_t3(or3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b)))));
                    }
                    TsInsn::LoadSigAnd { dl, sig, d, a, b } => {
                        set!(*dl, self.sig_planes(*sig));
                        set!(*d, and_p(v!(*a), x!(*a), v!(*b), x!(*b)));
                    }
                    TsInsn::LoadSigRepl {
                        dl,
                        sig,
                        d,
                        w,
                        count,
                    } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*dl, (v, x));
                        set!(
                            *d,
                            (
                                replicate_narrow(v, *w, *count),
                                replicate_narrow(x, *w, *count)
                            )
                        );
                    }
                    TsInsn::LoadSigSigRange {
                        dl,
                        sig,
                        d,
                        sig2,
                        lo,
                        mask,
                    } => {
                        set!(*dl, self.sig_planes(*sig));
                        let (v, x) = self.sig_planes(*sig2);
                        set!(*d, ((v >> lo) & mask, (x >> lo) & mask));
                    }
                    TsInsn::SigRangeAnd {
                        dr,
                        sig,
                        lo,
                        mask,
                        d,
                        a,
                        b,
                    } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*dr, ((v >> lo) & mask, (x >> lo) & mask));
                        set!(*d, and_p(v!(*a), x!(*a), v!(*b), x!(*b)));
                    }
                    TsInsn::SigRangeEq {
                        dr,
                        sig,
                        lo,
                        mask,
                        d,
                        a,
                        b,
                    } => {
                        let (v, x) = self.sig_planes(*sig);
                        set!(*dr, ((v >> lo) & mask, (x >> lo) & mask));
                        set!(*d, eq_p(v!(*a), x!(*a), v!(*b), x!(*b)));
                    }
                    TsInsn::ConstEq { dc, v, d, a, b } => {
                        set!(*dc, (*v, 0));
                        set!(*d, eq_p(v!(*a), x!(*a), v!(*b), x!(*b)));
                    }
                    TsInsn::LogOrStore { d, a, b, sig, mask } => {
                        let (rv, rx) = from_t3(or3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b))));
                        set!(*d, (rv, rx));
                        self.ts_store_xz(*sig as usize, rv & mask, rx & mask);
                    }
                    TsInsn::AndOr {
                        d1,
                        a1,
                        b1,
                        d,
                        a,
                        b,
                    } => {
                        set!(*d1, and_p(v!(*a1), x!(*a1), v!(*b1), x!(*b1)));
                        set!(*d, or_p(v!(*a), x!(*a), v!(*b), x!(*b)));
                    }
                    TsInsn::OrRangeStore {
                        d,
                        a,
                        b,
                        sig,
                        hi,
                        lo,
                    } => {
                        let mask = xmask(*hi - *lo + 1);
                        let (rv, rx) = or_p(v!(*a), x!(*a), v!(*b), x!(*b));
                        set!(*d, (rv, rx));
                        self.ts_range_store_xz(*sig as usize, rv & mask, rx & mask, *lo, *hi);
                    }
                    TsInsn::LogNot { d, s } => set!(*d, from_t3(not3(t3(v!(*s), x!(*s))))),
                    TsInsn::LogAnd { d, a, b } => {
                        set!(*d, from_t3(and3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b)))))
                    }
                    TsInsn::LogOr { d, a, b } => {
                        set!(*d, from_t3(or3(t3(v!(*a), x!(*a)), t3(v!(*b), x!(*b)))))
                    }
                    TsInsn::RedOr { d, s } => set!(*d, from_t3(t3(v!(*s), x!(*s)))),
                    TsInsn::RedAnd { d, s, mask } => {
                        let (sv, sx) = (v!(*s), x!(*s));
                        let p = if !sv & !sx & mask != 0 {
                            (0, 0)
                        } else if sx & mask != 0 {
                            (0, 1)
                        } else {
                            (1, 0)
                        };
                        set!(*d, p);
                    }
                    TsInsn::Concat { d, parts } => {
                        // §11.4.12: preserve both planes of a full-word
                        // first part without shifting zero by 64.
                        let (mut av, mut ax) =
                            parts.first().map_or((0, 0), |&(r, _)| (v!(r), x!(r)));
                        for &(r, w) in parts.iter().skip(1) {
                            // A 64-bit later part follows only zero-width
                            // parts (`{{0{b}}, word}`): it IS the result.
                            if w >= 64 {
                                av = v!(r);
                                ax = x!(r);
                            } else {
                                av = (av << w) | v!(r);
                                ax = (ax << w) | x!(r);
                            }
                        }
                        set!(*d, (av, ax));
                    }
                    TsInsn::Concat2 { d, a, wa: _, b, wb } => {
                        set!(*d, ((v!(*a) << *wb) | v!(*b), (x!(*a) << *wb) | x!(*b)))
                    }
                    TsInsn::Concat3 {
                        d,
                        a,
                        wa: _,
                        b,
                        wb,
                        c,
                        wc,
                    } => {
                        let av = ((v!(*a) << *wb) | v!(*b)) << *wc | v!(*c);
                        let ax = ((x!(*a) << *wb) | x!(*b)) << *wc | x!(*c);
                        set!(*d, (av, ax));
                    }
                    TsInsn::Mask { d, mask } => {
                        v!(*d) &= mask;
                        x!(*d) &= mask;
                    }
                    TsInsn::Repl { d, s, w, count } => {
                        set!(
                            *d,
                            (
                                replicate_narrow(v!(*s), *w, *count),
                                replicate_narrow(x!(*s), *w, *count)
                            )
                        )
                    }
                    TsInsn::BitDyn { d, s, i, w } => {
                        let p = if x!(*i) != 0 || v!(*i) >= *w as u64 {
                            (0, 1)
                        } else {
                            let idx = v!(*i);
                            ((v!(*s) >> idx) & 1, (x!(*s) >> idx) & 1)
                        };
                        set!(*d, p);
                    }
                    // ---- control ----
                    TsInsn::BrSigFalse { sig, bit, t } => {
                        let (v, x) = if *bit == u32::MAX {
                            self.signal_table[*sig as usize].raw_bits()
                        } else {
                            Self::raw_bits_slice(&self.signal_table[*sig as usize], *bit as u16, 1)
                        };
                        if t3(v, x) != Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::BrFalse { s, t } => {
                        if t3(v!(*s), x!(*s)) != Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::BrNz { s, t } => {
                        if t3(v!(*s), x!(*s)) == Some(true) {
                            pc = *t as usize;
                            continue;
                        }
                    }
                    TsInsn::Jmp { t } => {
                        pc = *t as usize;
                        continue;
                    }
                    // This executor has no bail path; stop the run here.
                    TsInsn::JmpPoll { t } => {
                        if loop_stop_requested() {
                            self.take_interrupt();
                            return true;
                        }
                        pc = *t as usize;
                        continue;
                    }
                    TsInsn::CaseJmp { s, cj } => {
                        pc = if x!(*s) != 0 {
                            cj.default as usize
                        } else {
                            cj.table.get(v!(*s) as usize).copied().unwrap_or(cj.default) as usize
                        };
                        continue;
                    }
                    TsInsn::CaseMaskJmp { s, mj: cm } => {
                        let (mask, lo, wmask, mj) = (&cm.mask, &cm.lo, &cm.wmask, &cm.mj);
                        pc = if x!(*s) & *mask != 0 {
                            mj.xz_path as usize
                        } else {
                            mj.table[(((v!(*s) & *mask) >> *lo) & *wmask) as usize] as usize
                        };
                        continue;
                    }
                    // ---- dynamic index ----
                    TsInsn::SigRangeDyn {
                        d,
                        sig,
                        i,
                        w,
                        sw,
                        mask,
                    } => {
                        let lo = v!(*i);
                        let p = if x!(*i) != 0 || lo + *w as u64 > *sw as u64 {
                            (0, *mask)
                        } else {
                            let (v, x) = Self::raw_bits_slice(
                                &self.signal_table[*sig as usize],
                                lo as u16,
                                *w,
                            );
                            (v & mask, x & mask)
                        };
                        set!(*d, p);
                    }
                    TsInsn::RangeStoreDyn {
                        sig,
                        i,
                        s,
                        w,
                        sw,
                        mask,
                    }
                    | TsInsn::RangeStoreNbaDyn {
                        sig,
                        i,
                        s,
                        w,
                        sw,
                        mask,
                    } => {
                        let lo = v!(*i);
                        if x!(*i) == 0 && lo + *w as u64 <= *sw as u64 {
                            let (id, lo, hi) =
                                (*sig as usize, lo as u32, lo as u32 + *w as u32 - 1);
                            let (pv, px) = (v!(*s) & mask, x!(*s) & mask);
                            let nba = matches!(insn, TsInsn::RangeStoreNbaDyn { .. });
                            match (nba, *sw > 64) {
                                (false, true) => self.ts_wide_range_store(id, lo, hi, pv, px),
                                (false, false) => self.ts_range_store_xz(id, pv, px, lo, hi),
                                (true, true) => self.ts_wide_range_store_nba_xz(id, lo, hi, pv, px),
                                (true, false) => self.ts_range_store_nba_xz(id, lo, hi, pv, px),
                            }
                        }
                    }
                    TsInsn::BitStoreDyn { sig, i, s, w }
                    | TsInsn::BitStoreNbaDyn { sig, i, s, w } => {
                        let idx = v!(*i);
                        if x!(*i) == 0 && idx < *w as u64 {
                            let (bv, bx) = (v!(*s) & 1, x!(*s) & 1);
                            let (id, b) = (*sig as usize, idx as u32);
                            let nba = matches!(insn, TsInsn::BitStoreNbaDyn { .. });
                            match (nba, *w > 64) {
                                (false, true) => self.ts_wide_range_store(id, b, b, bv, bx),
                                (false, false) => self.ts_range_store_xz(id, bv, bx, b, b),
                                (true, true) => self.ts_wide_range_store_nba_xz(id, b, b, bv, bx),
                                (true, false) => self.ts_range_store_nba_xz(id, b, b, bv, bx),
                            }
                        }
                    }
                    // ---- stores ----
                    TsInsn::Store { sig, s, mask } => {
                        self.ts_store_xz(*sig as usize, v!(*s) & mask, x!(*s) & mask);
                    }
                    TsInsn::RangeStore {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        self.ts_range_store_xz(
                            *sig as usize,
                            v!(*s) & mask,
                            x!(*s) & mask,
                            *lo,
                            *hi,
                        );
                    }
                    TsInsn::StoreNba { sig, s, w, mask } => {
                        self.ts_store_nba_xz(*sig as usize, v!(*s) & mask, x!(*s) & mask, *w);
                    }
                    TsInsn::ConstStoreNba { sig, v, w } => self.ts_store_nba(*sig as usize, *v, *w),
                    TsInsn::RangeStoreNba {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        self.ts_range_store_nba_xz(
                            *sig as usize,
                            *lo,
                            *hi,
                            v!(*s) & mask,
                            x!(*s) & mask,
                        );
                    }
                    TsInsn::RangeStoreW {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        self.ts_wide_range_store(
                            *sig as usize,
                            *lo,
                            *hi,
                            v!(*s) & mask,
                            x!(*s) & mask,
                        );
                    }
                    TsInsn::RangeStoreNbaW {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        self.ts_wide_range_store_nba_xz(
                            *sig as usize,
                            *lo,
                            *hi,
                            v!(*s) & mask,
                            x!(*s) & mask,
                        );
                    }
                    TsInsn::ConstStoreX { sig, v, x } => self.ts_store_xz(*sig as usize, *v, *x),
                    TsInsn::RangeStoreX(p) => {
                        self.ts_range_store_xz(p.sig as usize, p.v, p.x, p.lo, p.hi);
                    }
                    TsInsn::RangeStoreXW(p) => {
                        self.ts_wide_range_store(p.sig as usize, p.lo, p.hi, p.v, p.x);
                    }
                    TsInsn::RangeFillW { sig, hi, lo, bit } => {
                        self.ts_wide_range_fill(*sig as usize, *lo, *hi, *bit);
                    }
                    TsInsn::RangeFillNbaW { sig, hi, lo, bit } => {
                        self.ts_wide_range_fill_nba(*sig as usize, *lo, *hi, *bit);
                    }
                    TsInsn::RangeFillXW { sig, hi, lo, v } => {
                        self.ts_wide_range_fill_xz(*sig as usize, *lo, *hi, *v);
                    }
                    // ---- array elements (§7.4.6 / §11.5.1: an x or out-of-range
                    // index reads all-x and writes nothing) ----
                    TsInsn::ElemLoad(op) => {
                        let ew = self.signal_widths[op.first as usize];
                        let i = v!(op.idx) as i64;
                        let p = if x!(op.idx) != 0 || i < op.lo || i > op.hi {
                            (0, xmask(ew))
                        } else {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            self.signal_table[eid].raw_bits()
                        };
                        set!(op.s, p);
                    }
                    TsInsn::ElemStore(op) | TsInsn::ElemStoreNba(op) => {
                        let i = v!(op.idx) as i64;
                        if x!(op.idx) == 0 && i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            let (pv, px) = (v!(op.s) & op.mask, x!(op.s) & op.mask);
                            if matches!(insn, TsInsn::ElemStoreNba(..)) {
                                self.ts_store_nba_xz(eid, pv, px, op.w);
                            } else {
                                self.ts_store_xz(eid, pv, px);
                            }
                        }
                    }
                    TsInsn::ElemStoreNbaFromSig(f) => {
                        let op = &f.op;
                        let (sv, sx) = self.signal_table[f.sig as usize].raw_bits();
                        set!(f.dl, (sv, sx));
                        let i = v!(op.idx) as i64;
                        if x!(op.idx) == 0 && i >= op.lo && i <= op.hi {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            self.ts_store_nba_xz(eid, sv & op.mask, sx & op.mask, op.w);
                        }
                    }
                    TsInsn::NbaFromElem(op) | TsInsn::WNbaFromElem(op) => {
                        let (iv, ix) = self.sig_planes(op.idx_sig);
                        let i = iv as i64;
                        if ix != 0 || i < op.lo || i > op.hi {
                            let mut v = Value::new(op.w.max(1));
                            v.is_signed = false;
                            self.ts_store_nba_val(op.dst as usize, v);
                        } else {
                            let eid = op.first as usize + (i - op.lo) as usize;
                            if matches!(insn, TsInsn::WNbaFromElem(..)) {
                                let mut v = self.signal_table[eid].clone();
                                v.is_signed = false;
                                self.ts_store_nba_val(op.dst as usize, v);
                            } else {
                                let (ev, ex) = self.signal_table[eid].raw_bits();
                                let m = xmask(op.w);
                                self.ts_store_nba_xz(op.dst as usize, ev & m, ex & m, op.w);
                            }
                        }
                    }
                    // Hazard saves exist for the abort path; this executor
                    // never aborts.
                    TsInsn::SaveSig { .. } | TsInsn::SaveSigW { .. } => {}
                    TsInsn::CovHit(c) => self.ts_cov_hit(*c),
                    // This executor has no bail path; stop the run here.
                    TsInsn::LoopPoll => {
                        if loop_stop_requested() {
                            self.take_interrupt();
                            return true;
                        }
                    }
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
                    | TsInsn::WElemLoad(..) => return false,
                }
            }
        }
        true
    }
}
