//! AOT-to-native compilation of bytecode blocks via generated Rust source.
//!
//! `XEZIM_AOT=1` (with a `--features jit` build): instead of cranelift, each
//! JIT-eligible comb block is emitted as a Rust function on (val, xz) u64
//! plane locals, the whole set is compiled ONCE with `rustc -C opt-level=2`
//! into a cdylib, and the resulting function pointers are installed into
//! `comb_jit_fns` (same `JitFn` ABI, always rc 0 — the plane-exact code
//! never needs an interpreter re-run).
//!
//! Motivation: cranelift keeps every VM register in a stack slot and cannot
//! optimize across insns; rustc/LLVM keeps planes in machine registers and
//! folds the per-insn plane algebra. The per-insn SEMANTICS here are a
//! mechanical port of the validated cranelift arms in `jit.rs` — any change
//! there needs a mirror here.
//!
//! Signal traffic still goes through the same `xezim_jit_*` bridges, passed
//! into the dylib as a `#[repr(C)]` table of function pointers at bind time.

#![allow(dead_code)]

use super::bytecode::{ArrayOperand, Insn};
use super::jit::JitFn;
use std::fmt::Write as _;

/// Host-side mirror of the bridge table baked into the generated prelude.
/// Field order is ABI — keep in sync with `PRELUDE`.
#[repr(C)]
pub struct AotBridge {
    pub load: unsafe extern "C" fn(*mut u8, u32) -> u64,
    pub load_xz: unsafe extern "C" fn(*mut u8, u32) -> u64,
    pub store4s: unsafe extern "C" fn(*mut u8, u32, u64, u64, u32),
    pub nba4s: unsafe extern "C" fn(*mut u8, u32, u64, u64, u32),
    pub nba_range: unsafe extern "C" fn(*mut u8, u32, u64, u64, u64, u64),
    pub nba_bit: unsafe extern "C" fn(*mut u8, u32, u64, u64, u64),
    pub blk_range: unsafe extern "C" fn(*mut u8, u32, u64, u64, u64, u64),
    /// Wide-signal (>64-bit) slice loads — the same bridges the cranelift
    /// path uses. Step 8 coverage: without these every block touching a
    /// wide select was rejected outright (4,238 blocks on a C906 build).
    pub load_slice: unsafe extern "C" fn(*mut u8, u32, u32, u32) -> u64,
    pub load_slice_xz: unsafe extern "C" fn(*mut u8, u32, u32, u32) -> u64,
    /// Native-backend step 1 (NativeCtx): base address of the SoA
    /// val/xz planes (`signal_inline_bits`, `[u64; 2]` per id, so an
    /// id's planes live at `planes + id*16`) and the signal count they
    /// cover. `planes_len == 0` disables the direct path (mirror opted
    /// out) and every load takes the FFI bridge. The pointer is baked
    /// for the process lifetime — same contract the cranelift JIT's
    /// `raw_sig_loads` already relies on (the mirror vec is allocated
    /// to its final size before any native code is compiled).
    pub planes: u64,
    pub planes_len: u32,
}

/// Prelude of the generated crate: bridge plumbing + the per-insn plane
/// helpers (ports of the cranelift arms; §-references live on those arms).
const PRELUDE: &str = r#"
#![allow(unused_variables, unused_mut, unused_parens, dead_code, unreachable_code)]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AotBridge {
    pub load: unsafe extern "C" fn(*mut u8, u32) -> u64,
    pub load_xz: unsafe extern "C" fn(*mut u8, u32) -> u64,
    pub store4s: unsafe extern "C" fn(*mut u8, u32, u64, u64, u32),
    pub nba4s: unsafe extern "C" fn(*mut u8, u32, u64, u64, u32),
    pub nba_range: unsafe extern "C" fn(*mut u8, u32, u64, u64, u64, u64),
    pub nba_bit: unsafe extern "C" fn(*mut u8, u32, u64, u64, u64),
    pub blk_range: unsafe extern "C" fn(*mut u8, u32, u64, u64, u64, u64),
    pub load_slice: unsafe extern "C" fn(*mut u8, u32, u32, u32) -> u64,
    pub load_slice_xz: unsafe extern "C" fn(*mut u8, u32, u32, u32) -> u64,
    pub planes: u64,
    pub planes_len: u32,
}
static mut BRIDGE: Option<AotBridge> = None;
#[no_mangle]
pub unsafe extern "C" fn xezim_aot_bind(b: *const AotBridge) {
    BRIDGE = Some(*b);
}
#[inline(always)]
unsafe fn br() -> &'static AotBridge {
    // SAFETY: xezim_aot_bind runs before any block fn is installed.
    (&raw const BRIDGE).as_ref().unwrap_unchecked().as_ref().unwrap_unchecked()
}
#[inline(always)] unsafe fn ld(sim: *mut u8, id: u32) -> (u64, u64) {
    // NativeCtx direct load: the val/xz planes live at planes + id*16.
    // Blocks compiled here only touch fit-u64 signals (the same gate the
    // cranelift raw loads rely on); ids past the baked snapshot (created
    // at runtime) and the planes-disabled case take the FFI bridge.
    let b = br();
    if id < b.planes_len {
        let p = (b.planes as *const u64).add((id as usize) * 2);
        (*p, *p.add(1))
    } else {
        ((b.load)(sim, id), (b.load_xz)(sim, id))
    }
}
#[inline(always)] unsafe fn st4(sim: *mut u8, id: u32, v: u64, x: u64, w: u32, mask: u64) {
    // NativeCtx step 3 (store side): no-change fast exit directly on the
    // planes — the overwhelmingly common case in a settled design — before
    // paying the FFI call. Mirrors the bridge's own precheck; the generator
    // emits this form only when the store width equals the signal's declared
    // width (the condition under which the plane compare is exact). A
    // CHANGED value still commits through the bridge, which owns the Value
    // write, dirty marking and after-write side effects.
    let b = br();
    if id < b.planes_len {
        let p = (b.planes as *const u64).add((id as usize) * 2);
        if *p == v & mask && *p.add(1) == x & mask { return; }
    }
    (b.store4s)(sim, id, v, x, w);
}
/// Template-mode signal lookup: `S` is the per-BLOCK mapping array, `k` the
/// canonical first-use ordinal baked into the shared template body.
#[inline(always)] unsafe fn sg(s: *const u32, k: usize) -> u32 { *s.add(k) }
#[inline(always)] fn mask_w(w: u32) -> u64 { if w >= 64 { !0u64 } else { (1u64 << w) - 1 } }
#[inline(always)] fn and4(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let one = (av & !ax) & (bv & !bx);
    let zero = (!av & !ax) | (!bv & !bx);
    (one, !one & !zero)
}
#[inline(always)] fn or4(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let one = (av & !ax) | (bv & !bx);
    let zero = (!av & !ax) & (!bv & !bx);
    (one, !one & !zero)
}
#[inline(always)] fn xor4(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let unk = ax | bx;
    ((av ^ bv) & !unk, unk)
}
#[inline(always)] fn xnor4(av: u64, ax: u64, bv: u64, bx: u64) -> (u64, u64) {
    let unk = ax | bx;
    (!(av ^ bv) & !unk, unk)
}
#[inline(always)] fn not4(v: u64, x: u64) -> (u64, u64) { (!v & !x, x) }
#[inline(always)] fn arith4(res: u64, lx: u64, rx: u64) -> (u64, u64) {
    if (lx | rx) != 0 { (0, !0u64) } else { (res, 0) }
}
#[inline(always)] fn neg4(v: u64, x: u64) -> (u64, u64) {
    if x != 0 { (0, !0u64) } else { (v.wrapping_neg(), 0) }
}
#[inline(always)] fn sext(v: u64, w: u32) -> u64 {
    if w == 0 || w >= 64 { v } else { (((v << (64 - w)) as i64) >> (64 - w)) as u64 }
}
#[inline(always)] fn truth(v: u64, x: u64) -> (bool, bool) {
    ((v & !x) != 0, (v | x) == 0)
}
#[inline(always)] fn logand4(lv: u64, lx: u64, rv: u64, rx: u64) -> (u64, u64) {
    let (lt, lf) = truth(lv, lx);
    let (rt, rf) = truth(rv, rx);
    let t = lt && rt;
    let f = lf || rf;
    (t as u64, if t || f { 0 } else { 1 })
}
#[inline(always)] fn logor4(lv: u64, lx: u64, rv: u64, rx: u64) -> (u64, u64) {
    let (lt, lf) = truth(lv, lx);
    let (rt, rf) = truth(rv, rx);
    let t = lt || rt;
    let f = lf && rf;
    (t as u64, if t || f { 0 } else { 1 })
}
#[inline(always)] fn lognot4(v: u64, x: u64) -> (u64, u64) {
    let (t, f) = truth(v, x);
    (f as u64, if t || f { 0 } else { 1 })
}
// §11.4.5/§11.8.1: signed case equality extends BOTH planes. The
// canonical 64-bit encodings compare equally iff the common-width ones do.
#[inline(always)] fn caseeq4(
    mut lv: u64, mut lx: u64, mut rv: u64, mut rx: u64,
    both_signed: bool, lw: u32, rw: u32,
) -> (u64, u64) {
    if both_signed {
        lv = sext(lv, lw); lx = sext(lx, lw);
        rv = sext(rv, rw); rx = sext(rx, rw);
    }
    (((lv == rv) && (lx == rx)) as u64, 0)
}
/// op: 0 ==, 1 !=, 2 <, 3 <=, 4 >, 5 >=
#[inline(always)] fn cmp4(
    mut lv: u64, mut lx: u64, mut rv: u64, mut rx: u64,
    both_signed: bool, lw: u32, rw: u32, op: u8,
) -> (u64, u64) {
    if both_signed {
        if lw > 0 && lw < 64 { lv = sext(lv, lw); lx = sext(lx, lw); }
        if rw > 0 && rw < 64 { rv = sext(rv, rw); rx = sext(rx, rw); }
    }
    let c = match op {
        0 => lv == rv,
        1 => lv != rv,
        2 => if both_signed { (lv as i64) < (rv as i64) } else { lv < rv },
        3 => if both_signed { (lv as i64) <= (rv as i64) } else { lv <= rv },
        4 => if both_signed { (lv as i64) > (rv as i64) } else { lv > rv },
        _ => if both_signed { (lv as i64) >= (rv as i64) } else { lv >= rv },
    };
    let anyx = lx | rx;
    let (mut ov, mut ox) = if anyx != 0 { (0, 1) } else { (c as u64, 0) };
    if op <= 1 {
        // A commonly-known differing bit decides ==/!= even under X.
        let decided = ((lv ^ rv) & !anyx) != 0;
        if decided {
            ov = (op == 1) as u64;
            ox = 0;
        }
    }
    (ov, ox)
}
#[inline(always)] fn shl4(lv: u64, lx: u64, rv: u64, rx: u64) -> (u64, u64) {
    if rx != 0 { return (0, !0u64); }
    if rv >= 64 { (0, 0) } else { (lv << rv, lx << rv) }
}
#[inline(always)] fn shr4(lv: u64, lx: u64, rv: u64, rx: u64) -> (u64, u64) {
    if rx != 0 { return (0, !0u64); }
    if rv >= 64 { (0, 0) } else { (lv >> rv, lx >> rv) }
}
#[inline(always)] fn ashr4(lv: u64, lx: u64, rv: u64, rx: u64) -> (u64, u64) {
    if rx != 0 { return (0, !0u64); }
    let a = if rv >= 63 { 63 } else { rv as u32 };
    (((lv as i64) >> a) as u64, ((lx as i64) >> a) as u64)
}
#[inline(always)] fn reduceor4(v: u64, x: u64) -> (u64, u64) {
    let (t, f) = truth(v, x);
    (t as u64, if t || f { 0 } else { 1 })
}
#[inline(always)] fn reducexor4(v: u64, x: u64) -> (u64, u64) {
    if x != 0 { (0, 1) } else { ((v.count_ones() & 1) as u64, 0) }
}
#[inline(always)] fn bitsel4(bv: u64, bx: u64, i: u64, w: u32) -> (u64, u64) {
    if i < w as u64 { ((bv >> i) & 1, (bx >> i) & 1) } else { (0, 1) }
}
#[inline(always)] fn rangesel4(bv: u64, bx: u64, l0: u64, r0: u64, w: u32) -> (u64, u64) {
    // Bounds are 32-bit index arithmetic reinterpreted as i32 (a `-:` low
    // bound arrives as 0xFFFF_FFFE and must read as -2).
    let l = (l0 as u32) as i32 as i64;
    let r = (r0 as u32) as i32 as i64;
    let (lsb, msb) = if l <= r { (l, r) } else { (r, l) };
    let sr: i64 = if lsb >= 0 { lsb } else { 0 };
    let sl: i64 = if lsb < 0 { -lsb } else { 0 };
    let shr = |v: u64, a: i64| if a >= 64 { 0 } else { v >> a };
    let shl = |v: u64, a: i64| if a >= 64 { 0 } else { v << a };
    let v2 = shl(shr(bv, sr), sl);
    let x2 = shl(shr(bx, sr), sl);
    let resw = msb - lsb + 1;
    let resm: u64 = if resw >= 64 { !0 } else { (!0u64) >> (64 - resw) };
    let lo_m: u64 = if sl >= 64 { !0 } else if sl == 0 { 0 } else { (!0u64) >> (64 - sl) };
    let hi_start = { let h = (w as i64) - lsb; if h < 0 { 0 } else { h } };
    let hi_m: u64 = if hi_start >= 64 { 0 } else { (!0u64) << hi_start };
    let oor = lo_m | hi_m;
    (((v2 & !oor) & resm), ((x2 | oor) & resm))
}
#[inline(always)] fn resize4(v: u64, x: u64, cur_w: u32, signed: bool, width: u32) -> (u64, u64) {
    let (mut v, mut x) = (v, x);
    if signed && cur_w > 0 && width > cur_w {
        v = sext(v, cur_w);
        x = sext(x, cur_w);
    }
    let m = mask_w(width);
    (v & m, x & m)
}
#[inline(always)] fn select4(cv: u64, cx: u64, tv: u64, tx: u64, ev: u64, ex: u64) -> (u64, u64) {
    let (ct, cf) = truth(cv, cx);
    if ct { (tv, tx) } else if cf { (ev, ex) } else {
        let agree = !tx & !ex & !(tv ^ ev);
        (tv & agree, !agree)
    }
}
"#;

/// Per-generation register metadata mirror (same tables the cranelift
/// codegen tracks; delegated to the jit module's shared fns).
struct RegMeta {
    w: Vec<u32>,
    s: Vec<bool>,
}

/// Generate the Rust source for one block fn, or None if any insn (or
/// operand shape) is outside the ported set. Mirrors `jit.rs` gating.
pub fn gen_block_fn(
    fn_name: &str,
    insns: &[Insn],
    num_regs: u32,
    sig_w: &[u32],
    sig_signed: &[bool],
) -> Option<String> {
    gen_block_fn_mapped(fn_name, insns, num_regs, sig_w, sig_signed, None)
}

/// `sigmap = None` bakes signal ids (one body per block). `Some(map)` emits
/// mapping-array loads so a single body serves every block with the same
/// canonical shape; the caller then emits a per-block trampoline supplying
/// the array. See the template census (`XEZIM_TEMPLATE_CENSUS`).
pub fn gen_block_fn_mapped(
    fn_name: &str,
    insns: &[Insn],
    num_regs: u32,
    sig_w: &[u32],
    sig_signed: &[bool],
    sigmap: Option<&std::collections::HashMap<u32, u32>>,
) -> Option<String> {
    if super::jit::first_unsupported(insns).is_some() {
        return None;
    }
    let mut meta = RegMeta {
        w: vec![0; num_regs as usize],
        s: vec![false; num_regs as usize],
    };
    let n = insns.len();
    // `+:` part-select width recovery: the bytecode compiler lowers
    // `base[l +: K]` to `BinOpConst(h, l, Add, K-1)` immediately followed by
    // `RangeSelect(d, base, h, l)` — h == l + (K-1) always holds at runtime,
    // so d's width is statically K even though both bounds are dynamic.
    // Without this the select leaves width 0 and every downstream
    // const-select on d bails the whole block out of AOT (the c906 GPR
    // write decoders — ~20M interp evals — died exactly here).
    let rsel_w: std::collections::HashMap<usize, (u16, u32)> = insns
        .windows(2)
        .enumerate()
        .filter_map(|(j, pair)| {
            let Insn::BinOpConst(h2, l2, k, super::bytecode::BinOpConstKind::Add) = &pair[0] else {
                return None;
            };
            let Insn::RangeSelect(d, _, h, l) = &pair[1] else {
                return None;
            };
            if h2 != h || l2 != l || k.is_fill || k.is_real || k.width > 64 {
                return None;
            }
            let (kv, kx) = k.raw_bits();
            if kx != 0 || kv >= 64 {
                return None;
            }
            Some((j + 1, (*d, kv as u32 + 1)))
        })
        .collect();
    // Leaders (same scan as the cranelift CFG construction).
    let mut is_leader = vec![false; n.max(1)];
    is_leader[0] = true;
    for (i, insn) in insns.iter().enumerate() {
        let mut targets: Vec<usize> = Vec::new();
        match insn {
            Insn::BranchIfFalse(_, t)
            | Insn::Jump(t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::BranchUnlessZero(_, t)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::CmpBranchConst(_, _, _, _, t) => targets.push(*t as usize),
            Insn::CaseJump(_, cj) => {
                targets.extend(cj.table.iter().map(|&t| t as usize));
                targets.push(cj.default as usize);
            }
            _ => {}
        }
        if !targets.is_empty() {
            for t in targets {
                if t < n {
                    is_leader[t] = true;
                }
            }
            if i + 1 < n {
                is_leader[i + 1] = true;
            }
        }
    }
    let has_cf = insns.iter().any(|i| {
        matches!(
            i,
            Insn::BranchIfFalse(..)
                | Insn::Jump(..)
                | Insn::BranchIfSignalFalse(..)
                | Insn::BranchUnlessZero(..)
                | Insn::CmpBranch(..)
                | Insn::CmpBranchConst(..)
                | Insn::CaseJump(..)
        )
    });
    let single_block = !has_cf && is_leader.iter().filter(|&&b| b).count() == 1;

    let mut body = String::new();
    let mut tables = String::new();
    let mut ntab = 0usize;
    let w = &mut body;
    for (i, insn) in insns.iter().enumerate() {
        if is_leader[i] && !single_block {
            if i != 0 {
                // Fall through into the next leader.
                let _ = writeln!(w, "pc = {i}; continue 'sm; }}");
            }
            let _ = writeln!(w, "{i} => {{");
        }
        if emit_insn_rust(
            w,
            &mut tables,
            &mut ntab,
            insn,
            i,
            n,
            &meta,
            sig_w,
            sig_signed,
            sigmap,
        )
        .is_none()
        {
            if std::env::var("XEZIM_JIT_VERBOSE").is_ok() {
                eprintln!(
                    "[AOT-GEN-BAIL] insn[{i}] {} :: {:?}",
                    super::bytecode::insn_opcode_name(insn),
                    insn
                );
                if std::env::var("XEZIM_JIT_VERBOSE").as_deref() == Ok("2") {
                    for (k, ins) in insns.iter().enumerate() {
                        eprintln!(
                            "[AOT-GEN-BAIL]   [{k}]{} {:?}",
                            if k == i { "*" } else { " " },
                            ins
                        );
                    }
                }
            }
            return None;
        }
        // Post-op width mask (mirror of the codegen masking pass).
        if let Some((d, mw)) = super::jit::insn_result_width(insn, &meta.w, sig_w) {
            let m = (1u64 << mw) - 1;
            let _ = writeln!(w, "r{d}v &= {m:#x}; r{d}x &= {m:#x};");
        }
        super::jit::update_reg_meta(insn, &mut meta.w, &mut meta.s, sig_w, sig_signed);
        if let Some(&(d, wid)) = rsel_w.get(&i) {
            meta.w[d as usize] = wid;
        }
    }
    if !single_block {
        let _ = writeln!(w, "return 0; }}");
    }

    let mut out = String::new();
    // Template body: internal, takes the per-block mapping pointer.
    // `#[inline(never)]` is LOAD-BEARING — without it rustc inlines the body
    // back into every trampoline and undoes the dedup entirely.
    if sigmap.is_some() {
        // Template bodies are exported directly — the host owns the per-block
        // mapping arrays and calls `body(sim, map_ptr)` itself. No trampolines:
        // on C910 the first trampoline-based build kept ~116k items and rustc
        // time barely moved (3,870s -> 3,550s despite 13.2x fewer bodies) —
        // rustc cost scales with ITEM COUNT, not body text.
        let _ = writeln!(
            out,
            "#[no_mangle]\npub unsafe extern \"C\" fn {fn_name}(sim: *mut u8, S: *const u32) -> u32 {{"
        );
    } else {
        let _ = writeln!(
            out,
            "#[no_mangle]\npub unsafe extern \"C\" fn {fn_name}(sim: *mut u8) -> u32 {{"
        );
    }
    out.push_str(&tables);
    for r in 0..num_regs {
        let _ = writeln!(out, "let mut r{r}v = 0u64; let mut r{r}x = 0u64;");
    }
    if single_block {
        out.push_str(&body);
        out.push_str("0\n}\n");
    } else {
        out.push_str("let mut pc: u32 = 0;\n'sm: loop { match pc {\n");
        out.push_str(&body);
        out.push_str("_ => return 0,\n} }\n}\n");
    }
    Some(out)
}

/// Roadmap step 13: a PROCESS-FSM body as a native state machine — the
/// same per-insn emitter as `gen_block_fn`, with three differences:
///  - signature `(sim, start_pc, frame, out) -> next_pc`: registers load
///    from `frame` (two u64 planes per reg) at entry and spill back at
///    every suspension, so values survive across activations;
///  - `WaitDelayReg` / `WaitEdge` SUSPEND: `out[0]` = 1 (delay) / 2
///    (edge), `out[1]` = edge spec index, `out[2..4]` = the delay
///    register's value/xz planes; the return value is the resume pc.
///    Completion spills and sets `out[0] = 0`;
///  - an INTEGRAL `Real` constant whose register feeds only
///    `WaitDelayReg` emits as its integer tick count (elaboration folds
///    quantized delays into Real tick literals, which the general
///    emitter rejects).
pub fn gen_fsm_fn(
    fn_name: &str,
    insns: &[Insn],
    num_regs: u32,
    sig_w: &[u32],
    sig_signed: &[bool],
) -> Option<String> {
    use Insn::*;
    let mut meta = RegMeta {
        w: vec![0; num_regs as usize],
        s: vec![false; num_regs as usize],
    };
    let n = insns.len();
    let mut is_leader = vec![false; n.max(1)];
    is_leader[0] = true;
    for (i, insn) in insns.iter().enumerate() {
        let mut targets: Vec<usize> = Vec::new();
        match insn {
            BranchIfFalse(_, t)
            | Jump(t)
            | BranchIfSignalFalse(_, t, _)
            | BranchUnlessZero(_, t)
            | CmpBranch(_, _, _, _, t)
            | CmpBranchConst(_, _, _, _, t) => targets.push(*t as usize),
            CaseJump(_, cj) => {
                targets.extend(cj.table.iter().map(|&t| t as usize));
                targets.push(cj.default as usize);
            }
            // Resume points must be dispatchable.
            WaitDelayReg(..) | WaitEdge(..) => {
                if i + 1 < n {
                    is_leader[i + 1] = true;
                }
            }
            _ => {}
        }
        if !targets.is_empty() {
            for t in targets {
                if t < n {
                    is_leader[t] = true;
                }
            }
            if i + 1 < n {
                is_leader[i + 1] = true;
            }
        }
    }
    // An integral Real constant loaded IMMEDIATELY before the WaitDelayReg
    // that consumes it is a quantized tick literal (that adjacent pair is
    // exactly what the statement compiler emits for `#const`); it may emit
    // as an integer. Keyed by insn index — the register may be legitimately
    // redefined and reused later.
    let mut delay_const_ok: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (i, insn) in insns.iter().enumerate() {
        if let LoadConst(d, v) = insn {
            if v.is_real {
                let f = v.to_f64();
                let integral = f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f < 9.0e15;
                let feeds_wait = matches!(insns.get(i + 1), Some(WaitDelayReg(r)) if r == d);
                if integral && feeds_wait {
                    delay_const_ok.insert(i);
                }
            }
        }
    }
    let spill: String = (0..num_regs)
        .map(|r| {
            format!(
                "*frame.add({}) = r{r}v; *frame.add({}) = r{r}x;
",
                2 * r as usize,
                2 * r as usize + 1
            )
        })
        .collect();
    let mut body = String::new();
    let mut tables = String::new();
    let mut ntab = 0usize;
    let w = &mut body;
    for (i, insn) in insns.iter().enumerate() {
        if is_leader[i] {
            if i != 0 {
                let _ = writeln!(w, "pc = {i}; continue 'sm; }}");
            }
            let _ = writeln!(w, "{i} => {{");
        }
        match insn {
            WaitDelayReg(r) => {
                w.push_str(&spill);
                let _ = writeln!(
                    w,
                    "*out.add(0) = 1; *out.add(2) = r{r}v; *out.add(3) = r{r}x; return {};",
                    i + 1
                );
                continue;
            }
            WaitEdge(ix) => {
                w.push_str(&spill);
                let _ = writeln!(w, "*out.add(0) = 2; *out.add(1) = {ix}; return {};", i + 1);
                continue;
            }
            LoadConst(d, v) if v.is_real && delay_const_ok.contains(&i) => {
                let ticks = v.to_f64() as u64;
                let _ = writeln!(w, "r{d}v = {ticks}u64; r{d}x = 0;");
                super::jit::update_reg_meta(insn, &mut meta.w, &mut meta.s, sig_w, sig_signed);
                continue;
            }
            _ => {}
        }
        if emit_insn_rust(
            w,
            &mut tables,
            &mut ntab,
            insn,
            i,
            n,
            &meta,
            sig_w,
            sig_signed,
            None,
        )
        .is_none()
        {
            if std::env::var_os("XEZIM_JIT_VERBOSE").is_some() {
                let lo = i.saturating_sub(2);
                let hi = (i + 3).min(n);
                let win: Vec<String> = insns[lo..hi]
                    .iter()
                    .enumerate()
                    .map(|(k, x)| format!("#{}:{:?}", lo + k, x))
                    .collect();
                eprintln!("[AOT-FSM-EMIT] bail at #{i}: {}", win.join(" | "));
            }
            return None;
        }
        if let Some((d, mw)) = super::jit::insn_result_width(insn, &meta.w, sig_w) {
            let m = (1u64 << mw) - 1;
            let _ = writeln!(w, "r{d}v &= {m:#x}; r{d}x &= {m:#x};");
        }
        super::jit::update_reg_meta(insn, &mut meta.w, &mut meta.s, sig_w, sig_signed);
    }
    // Completion epilogue (fell past the last insn).
    let _ = writeln!(w, "{}", spill);
    let _ = writeln!(w, "*out.add(0) = 0; return 0; }}");

    let mut out = String::new();
    let _ = writeln!(
        out,
        "#[no_mangle]\npub unsafe extern \"C\" fn {fn_name}(sim: *mut u8, start_pc: u32, frame: *mut u64, out: *mut u64) -> u32 {{"
    );
    out.push_str(&tables);
    for r in 0..num_regs {
        let _ = writeln!(
            out,
            "let mut r{r}v = *frame.add({}); let mut r{r}x = *frame.add({});",
            2 * r as usize,
            2 * r as usize + 1
        );
    }
    out.push_str(
        "let mut pc: u32 = start_pc;
'sm: loop { match pc {
",
    );
    out.push_str(&body);
    out.push_str("_ => { ");
    out.push_str(&spill);
    out.push_str(
        "*out.add(0) = 0; return 0; }
} }
}
",
    );
    Some(out)
}

/// Native process-FSM entry: (sim, start_pc, frame, out) -> resume pc.
pub type AotFsmFn = unsafe extern "C" fn(*mut u8, u32, *mut u64, *mut u64) -> u32;

/// One insn -> Rust statements. Returns None to reject the whole block
/// (mirrors the `Err(())` bails in the cranelift emit path).
#[allow(clippy::too_many_arguments)]
fn emit_insn_rust(
    w: &mut String,
    tables: &mut String,
    ntab: &mut usize,
    insn: &Insn,
    i: usize,
    n: usize,
    meta: &RegMeta,
    sig_w: &[u32],
    sig_signed: &[bool],
    sigmap: Option<&std::collections::HashMap<u32, u32>>,
) -> Option<()> {
    use Insn::*;
    let rw = |r: u16| meta.w.get(r as usize).copied().unwrap_or(0);
    let rs = |r: u16| meta.s.get(r as usize).copied().unwrap_or(false);
    // How a signal id reaches the generated code. `None` bakes the literal
    // id (one body per block, the original behaviour, byte-identical).
    // `Some(map)` emits a load from the block's mapping array instead, so
    // ONE body can serve every block sharing this canonical shape.
    let sref = |id: u32| -> String {
        match sigmap {
            None => id.to_string(),
            Some(m) => format!("sg(S,{})", m.get(&id).copied().unwrap_or(0)),
        }
    };
    match insn {
        Nop | SetSigned(_) | ClearSigned(_) => {}
        LoadConst(d, v) => {
            if v.is_fill || v.is_real || v.width > 64 {
                return None;
            }
            let (vb, xb) = v.raw_bits();
            let _ = writeln!(w, "r{d}v = {vb:#x}; r{d}x = {xb:#x};");
        }
        LoadSignal(d, sig) | LoadSignalSigned(d, sig) => {
            let sig = sref(*sig);
            let _ = writeln!(w, "let t = ld(sim, {sig}); r{d}v = t.0; r{d}x = t.1;");
        }
        // §11.4.12.1 replication: n copies, high copy first — unrolled
        // shift/or, same gating as the cranelift arm (result must fit u64).
        Replicate(d, sr, cnt) => {
            let pw = rw(*sr);
            if pw == 0 || (pw as u64) * (*cnt as u64) > 64 {
                return None;
            }
            if *cnt == 0 {
                let _ = writeln!(w, "r{d}v = 0; r{d}x = 0;");
            } else {
                let m = if pw >= 64 { u64::MAX } else { (1u64 << pw) - 1 };
                let _ = writeln!(
                    w,
                    "let pv = r{sr}v & {m:#x}; let px = r{sr}x & {m:#x}; let mut av = pv; let mut ax = px;"
                );
                for _ in 1..*cnt {
                    let _ = writeln!(w, "av = (av << {pw}) | pv; ax = (ax << {pw}) | px;");
                }
                let _ = writeln!(w, "r{d}v = av; r{d}x = ax;");
            }
        }
        // Fused const-operand ALU ops — ports of the cranelift BinOpConst
        // arms (Add/Xor/CaseEq direct; Eq through the prelude cmp4, which
        // carries the known-bit-decided rule and signed extension).
        BinOpConst(d, sr, k, kind) => {
            use super::bytecode::BinOpConstKind as K;
            let (kv, kx) = k.raw_bits();
            match kind {
                K::And => {
                    let _ = writeln!(
                        w,
                        "let t = and4(r{sr}v, r{sr}x, {kv:#x}, {kx:#x}); r{d}v = t.0; r{d}x = t.1;"
                    );
                }
                K::Or => {
                    let _ = writeln!(
                        w,
                        "let t = or4(r{sr}v, r{sr}x, {kv:#x}, {kx:#x}); r{d}v = t.0; r{d}x = t.1;"
                    );
                }
                K::Mul => {
                    let _ = writeln!(
                        w,
                        "let t = arith4(r{sr}v.wrapping_mul({kv:#x}), r{sr}x, {kx:#x}); r{d}v = t.0; r{d}x = t.1;"
                    );
                }
                K::Sub => {
                    let _ = writeln!(
                        w,
                        "let t = arith4(r{sr}v.wrapping_sub({kv:#x}), r{sr}x, {kx:#x}); r{d}v = t.0; r{d}x = t.1;"
                    );
                }
                K::Add => {
                    if kx != 0 {
                        let _ = writeln!(w, "r{d}v = 0; r{d}x = !0u64;");
                    } else {
                        let _ = writeln!(
                            w,
                            "if r{sr}x != 0 {{ r{d}v = 0; r{d}x = !0u64; }} else {{ r{d}v = r{sr}v.wrapping_add({kv:#x}); r{d}x = 0; }}"
                        );
                    }
                }
                K::Xor => {
                    let _ = writeln!(
                        w,
                        "let unk = r{sr}x | {kx:#x}; r{d}v = (r{sr}v ^ {kv:#x}) & !unk; r{d}x = unk;"
                    );
                }
                K::CaseEq => {
                    let both = rs(*sr) && k.is_signed;
                    let lw = rw(*sr);
                    let kw = k.width;
                    let _ = writeln!(
                        w,
                        "let t = caseeq4(r{sr}v, r{sr}x, {kv:#x}, {kx:#x}, {both}, {lw}, {kw}); r{d}v = t.0; r{d}x = t.1;"
                    );
                }
                K::Eq => {
                    let both = rs(*sr) && k.is_signed;
                    let lw = rw(*sr);
                    let kw = k.width;
                    let _ = writeln!(
                        w,
                        "let t = cmp4(r{sr}v, r{sr}x, {kv:#x}, {kx:#x}, {both}, {lw}, {kw}, 0); r{d}v = t.0; r{d}x = t.1;"
                    );
                }
            }
        }
        // §11.4.9 reduction AND — 1 iff every bit known-1, 0 if any known-0,
        // else x (mirror of the cranelift arm).
        ReduceAnd(d, sr) => {
            let sw = rw(*sr);
            if sw == 0 || sw > 64 {
                return None;
            }
            let m = if sw >= 64 { u64::MAX } else { (1u64 << sw) - 1 };
            let _ = writeln!(
                w,
                "let known1 = (r{sr}v & !r{sr}x) & {m:#x}; let known0 = (!r{sr}v & !r{sr}x) & {m:#x}; if known0 != 0 {{ r{d}v = 0; r{d}x = 0; }} else if known1 == {m:#x} {{ r{d}v = 1; r{d}x = 0; }} else {{ r{d}v = 0; r{d}x = 1; }}"
            );
        }
        Move(d, s) => {
            let _ = writeln!(w, "r{d}v = r{s}v; r{d}x = r{s}x;");
        }
        MoveResize(d, sr, width) => {
            if *width > 64 {
                return None;
            }
            let cw = rw(*sr);
            let sg = rs(*sr);
            let _ = writeln!(
                w,
                "let t = resize4(r{sr}v, r{sr}x, {cw}, {sg}, {width}); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Add(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = arith4(r{l}v.wrapping_add(r{r}v), r{l}x, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Sub(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = arith4(r{l}v.wrapping_sub(r{r}v), r{l}x, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Mul(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = arith4(r{l}v.wrapping_mul(r{r}v), r{l}x, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        BitAnd(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = and4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        BitOr(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = or4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        BitXor(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = xor4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        BitXnor(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = xnor4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        BitNot(d, s) => {
            let _ = writeln!(w, "let t = not4(r{s}v, r{s}x); r{d}v = t.0; r{d}x = t.1;");
        }
        Negate(d, s) => {
            let _ = writeln!(w, "let t = neg4(r{s}v, r{s}x); r{d}v = t.0; r{d}x = t.1;");
        }
        LogAnd(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = logand4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        LogOr(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = logor4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        LogNot(d, s) => {
            let _ = writeln!(
                w,
                "let t = lognot4(r{s}v, r{s}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Eq(d, l, r) | Neq(d, l, r) | Lt(d, l, r) | Leq(d, l, r) | Gt(d, l, r) | Geq(d, l, r) => {
            let op = match insn {
                Eq(..) => 0,
                Neq(..) => 1,
                Lt(..) => 2,
                Leq(..) => 3,
                Gt(..) => 4,
                _ => 5,
            };
            let bs = rs(*l) && rs(*r);
            let (lw, rw_) = (rw(*l), rw(*r));
            let _ = writeln!(
                w,
                "let t = cmp4(r{l}v, r{l}x, r{r}v, r{r}x, {bs}, {lw}, {rw_}, {op}); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        CaseEq(d, l, r) => {
            let both = rs(*l) && rs(*r);
            let (lw, rw_) = (rw(*l), rw(*r));
            let _ = writeln!(
                w,
                "let t = caseeq4(r{l}v, r{l}x, r{r}v, r{r}x, {both}, {lw}, {rw_}); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Shl(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = shl4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Shr(d, l, r) => {
            let _ = writeln!(
                w,
                "let t = shr4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        AShr(d, l, r) => {
            // Pre-sext the left operand (writes back, like the cranelift arm).
            let lw = rw(*l);
            if rs(*l) && lw > 0 && lw < 64 {
                let _ = writeln!(w, "r{l}v = sext(r{l}v, {lw}); r{l}x = sext(r{l}x, {lw});");
            }
            let _ = writeln!(
                w,
                "let t = ashr4(r{l}v, r{l}x, r{r}v, r{r}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        ReduceOr(d, s) => {
            let _ = writeln!(
                w,
                "let t = reduceor4(r{s}v, r{s}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        ReduceXor(d, s) => {
            let _ = writeln!(
                w,
                "let t = reducexor4(r{s}v, r{s}x); r{d}v = t.0; r{d}x = t.1;"
            );
        }
        Resize(reg, width) => {
            let cw = rw(*reg);
            let sg = rs(*reg);
            let _ = writeln!(
                w,
                "let t = resize4(r{reg}v, r{reg}x, {cw}, {sg}, {width}); r{reg}v = t.0; r{reg}x = t.1;"
            );
        }
        Select(dest, c, t, e) => {
            let _ = writeln!(
                w,
                "let t = select4(r{c}v, r{c}x, r{t}v, r{t}x, r{e}v, r{e}x); r{dest}v = t.0; r{dest}x = t.1;"
            );
        }
        Concat(d, parts) => {
            let mut total: u32 = 0;
            for p in parts.iter() {
                let pw = rw(*p);
                if pw == 0 {
                    return None;
                }
                total += pw;
            }
            if total > 64 {
                return None;
            }
            let _ = writeln!(w, "let mut av = 0u64; let mut ax = 0u64;");
            for (pi, p) in parts.iter().enumerate() {
                let pw = rw(*p);
                let m = mask_c(pw);
                if pi == 0 {
                    // First part: no accumulated bits yet — skip the shift,
                    // which rustc rejects outright at pw == 64.
                    let _ = writeln!(w, "av = r{p}v & {m:#x}; ax = r{p}x & {m:#x};");
                } else if pw >= 64 {
                    // A later full-width part would need `<< 64` — the whole
                    // concat exceeds u64 anyway; not AOT-eligible.
                    return None;
                } else {
                    let _ = writeln!(
                        w,
                        "av = (av << {pw}) | (r{p}v & {m:#x}); ax = (ax << {pw}) | (r{p}x & {m:#x});"
                    );
                }
            }
            let _ = writeln!(w, "r{d}v = av; r{d}x = ax;");
        }
        BitSelect(dest, base, idx) => {
            let bw = rw(*base);
            if bw == 0 {
                return None;
            }
            let _ = writeln!(
                w,
                "let t = bitsel4(r{base}v, r{base}x, r{idx}v, {bw}); r{dest}v = t.0; r{dest}x = t.1;"
            );
        }
        BitSelectConst(dest, base, idx) => {
            let bw = rw(*base);
            if bw == 0 {
                return None;
            }
            if *idx >= bw {
                let _ = writeln!(w, "r{dest}v = 0; r{dest}x = 1;");
            } else {
                let _ = writeln!(
                    w,
                    "r{dest}v = (r{base}v >> {idx}) & 1; r{dest}x = (r{base}x >> {idx}) & 1;"
                );
            }
        }
        RangeSelect(dest, base, l_r, r_r) => {
            let bw = rw(*base);
            if bw == 0 {
                return None;
            }
            let _ = writeln!(
                w,
                "let t = rangesel4(r{base}v, r{base}x, r{l_r}v, r{r_r}v, {bw}); r{dest}v = t.0; r{dest}x = t.1;"
            );
        }
        RangeSelectConst(dest, base, l_imm, r_imm) => {
            let bw = rw(*base);
            if bw == 0 {
                return None;
            }
            let lsb = (*l_imm).min(*r_imm);
            let msb = (*l_imm).max(*r_imm);
            let resw = msb - lsb + 1;
            // resw == 64 fits a u64 exactly; only wider selects bail.
            if resw > 64 {
                return None;
            }
            let resm: u64 = if resw >= 64 {
                u64::MAX
            } else {
                (1u64 << resw) - 1
            };
            let oor: u64 = if lsb >= bw {
                resm
            } else if msb >= bw {
                (resm >> (bw - lsb)) << (bw - lsb)
            } else {
                0
            };
            let keep = resm & !oor;
            let _ = writeln!(
                w,
                "r{dest}v = (r{base}v >> {lsb}) & {keep:#x}; r{dest}x = ((r{base}x >> {lsb}) & {keep:#x}) | {oor:#x};"
            );
        }
        LoadSignalBit(dest, sig, bit) => {
            let sw = sig_w.get(*sig as usize).copied().unwrap_or(0);
            let sig = sref(*sig);
            if sw > 0 && *bit >= sw {
                let _ = writeln!(w, "r{dest}v = 0; r{dest}x = 1;");
            } else if *bit >= 64 {
                // In-range bit of a WIDE signal: the u64 shift form would
                // overflow — use the slice bridge (same as cranelift).
                let _ = writeln!(
                    w,
                    "r{dest}v = (br().load_slice)(sim, {sig}, {bit}, 1); r{dest}x = (br().load_slice_xz)(sim, {sig}, {bit}, 1);"
                );
            } else {
                let _ = writeln!(
                    w,
                    "let t = ld(sim, {sig}); r{dest}v = (t.0 >> {bit}) & 1; r{dest}x = (t.1 >> {bit}) & 1;"
                );
            }
        }
        LoadSignalRange(dest, sig, left, right) => {
            let sig_sw = sig_w.get(*sig as usize).copied().unwrap_or(0);
            let sig = sref(*sig);
            let lo = (*left).min(*right);
            let hi = (*left).max(*right);
            let wid = left.abs_diff(*right) + 1;
            // wid == 64 fits a u64 exactly (a c906 [127:64] slice); only
            // wider slices need multi-word moves the register file lacks.
            if wid > 64 {
                return None;
            }
            // A select reaching past bit 63 lives on a WIDE signal — the
            // u64 shift form would overflow (`deny(arithmetic_overflow)`,
            // seen on a C906 `[127:120]` slice). Route through the slice
            // bridge, exactly like cranelift; out-of-declared-range bits
            // are X-marked with the same keep/oor masks as the narrow arm.
            if hi >= 64 {
                let sw = sig_sw;
                let full: u64 = if wid >= 64 {
                    u64::MAX
                } else {
                    (1u64 << wid) - 1
                };
                let oor: u64 = if sw > 0 && lo >= sw {
                    full
                } else if sw > 0 && hi >= sw {
                    (full >> (sw - lo)) << (sw - lo)
                } else {
                    0
                };
                let keep = full & !oor;
                let _ = writeln!(
                    w,
                    "r{dest}v = (br().load_slice)(sim, {sig}, {lo}, {wid}) & {keep:#x}; r{dest}x = ((br().load_slice_xz)(sim, {sig}, {lo}, {wid}) & {keep:#x}) | {oor:#x};"
                );
                return Some(());
            }
            let sw = sig_sw;
            let full: u64 = if wid >= 64 {
                u64::MAX
            } else {
                (1u64 << wid) - 1
            };
            let oor: u64 = if sw > 0 && lo >= sw {
                full
            } else if sw > 0 && hi >= sw {
                (full >> (sw - lo)) << (sw - lo)
            } else {
                0
            };
            let keep = full & !oor;
            let _ = writeln!(
                w,
                "let t = ld(sim, {sig}); r{dest}v = (t.0 >> {lo}) & {keep:#x}; r{dest}x = ((t.1 >> {lo}) & {keep:#x}) | {oor:#x};"
            );
        }
        BlockingAssign(sig, val, width) => {
            // Native no-change precheck only when the store width matches
            // the signal's declared width (plane compare is exact then);
            // width 0 means "signal width" in the bridge, same condition.
            let sw = sig_w.get(*sig as usize).copied().unwrap_or(0);
            let sig = sref(*sig);
            let effw = if *width == 0 { sw } else { *width };
            if effw == sw && (1..=64).contains(&effw) {
                let mask = if effw >= 64 {
                    u64::MAX
                } else {
                    (1u64 << effw) - 1
                };
                let _ = writeln!(w, "st4(sim, {sig}, r{val}v, r{val}x, {width}, {mask:#x});");
            } else {
                let _ = writeln!(w, "(br().store4s)(sim, {sig}, r{val}v, r{val}x, {width});");
            }
        }
        NbaAssign(sig, val, width) => {
            let sig = sref(*sig);
            let _ = writeln!(w, "(br().nba4s)(sim, {sig}, r{val}v, r{val}x, {width});");
        }
        BlockingAssignRange(sig, hi, lo, val) => {
            let sig = sref(*sig);
            let _ = writeln!(
                w,
                "(br().blk_range)(sim, {sig}, {hi}, {lo}, r{val}v, r{val}x);"
            );
        }
        BlockingAssignRangeDyn(sig, hi_r, lo_r, val) => {
            let sig = sref(*sig);
            let _ = writeln!(
                w,
                "(br().blk_range)(sim, {sig}, r{hi_r}v, r{lo_r}v, r{val}v, r{val}x);"
            );
        }
        BlockingAssignBitDyn(sig, idx_r, val) => {
            let sig = sref(*sig);
            // §11.5.1: an x/z index modifies no bits. The x plane never
            // reached the bridge and the value plane has x bits masked to
            // zero, so an unknown index arrived as bit 0. Hand the bridge a
            // deliberately out-of-range index instead; it already drops those.
            let _ = writeln!(
                w,
                "let bi{idx_r} = if r{idx_r}x != 0 {{ u64::MAX }} else {{ r{idx_r}v }};\n\
                 (br().blk_range)(sim, {sig}, bi{idx_r}, bi{idx_r}, r{val}v, r{val}x);"
            );
        }
        NbaAssignConst(sig, v, width) => {
            let sig = sref(*sig);
            if v.is_fill || v.is_real || v.width > 64 {
                return None;
            }
            let (vb, xb) = v.raw_bits();
            let _ = writeln!(
                w,
                "(br().nba4s)(sim, {sig}, {vb:#x}u64, {xb:#x}u64, {width});"
            );
        }
        NbaAssignRange(sig, hi, lo, val) => {
            let sig = sref(*sig);
            let _ = writeln!(
                w,
                "(br().nba_range)(sim, {sig}, {hi}, {lo}, r{val}v, r{val}x);"
            );
        }
        NbaAssignRangeDyn(sig, hi_r, lo_r, val) => {
            let sig = sref(*sig);
            let _ = writeln!(
                w,
                "(br().nba_range)(sim, {sig}, r{hi_r}v, r{lo_r}v, r{val}v, r{val}x);"
            );
        }
        NbaAssignBitDyn(sig, idx_r, val) => {
            let sig = sref(*sig);
            // §11.5.1, as for the blocking form above.
            let _ = writeln!(
                w,
                "let bi{idx_r} = if r{idx_r}x != 0 {{ u64::MAX }} else {{ r{idx_r}v }};\n\
                 (br().nba_bit)(sim, {sig}, bi{idx_r}, r{val}v, r{val}x);"
            );
        }
        LoadArrayElem(d, arr, idx_reg) => {
            let ArrayOperand::Dense {
                first_id, lo, hi, ..
            } = arr.as_ref()
            else {
                return None;
            };
            // A dense array's BASE id is mapped like any other signal id: the
            // element address is base + (index - lo), so mapping the base
            // relocates the whole array for this block's instance.
            let first_id = sref(*first_id as u32);
            let _ = writeln!(
                w,
                "let eff = (r{idx_reg}v & !r{idx_reg}x) as i64;\n\
                 if eff >= {lo} && eff <= {hi} {{\n\
                   let t = ld(sim, ({first_id} as i64 + (eff - {lo})) as u32); r{d}v = t.0; r{d}x = t.1;\n\
                 }} else {{ r{d}v = 0; r{d}x = 1; }}"
            );
        }
        BlockingAssignArray(arr, idx_reg, val_reg, width) => {
            let ArrayOperand::Dense {
                first_id, lo, hi, ..
            } = arr.as_ref()
            else {
                return None;
            };
            // A dense array's BASE id is mapped like any other signal id: the
            // element address is base + (index - lo), so mapping the base
            // relocates the whole array for this block's instance.
            let first_id = sref(*first_id as u32);
            let _ = writeln!(
                w,
                "let eff = (r{idx_reg}v & !r{idx_reg}x) as i64;\n\
                 if eff >= {lo} && eff <= {hi} {{\n\
                   (br().store4s)(sim, ({first_id} as i64 + (eff - {lo})) as u32, r{val_reg}v, r{val_reg}x, {width});\n\
                 }}"
            );
        }
        NbaAssignArray(arr, idx_reg, val_reg, width) => {
            let ArrayOperand::Dense {
                first_id, lo, hi, ..
            } = arr.as_ref()
            else {
                return None;
            };
            // A dense array's BASE id is mapped like any other signal id: the
            // element address is base + (index - lo), so mapping the base
            // relocates the whole array for this block's instance.
            let first_id = sref(*first_id as u32);
            let _ = writeln!(
                w,
                "let eff = (r{idx_reg}v & !r{idx_reg}x) as i64;\n\
                 if eff >= {lo} && eff <= {hi} {{\n\
                   (br().nba4s)(sim, ({first_id} as i64 + (eff - {lo})) as u32, r{val_reg}v, r{val_reg}x, {width});\n\
                 }}"
            );
        }
        // Fused LoadSignal+LoadArrayElem+NbaAssign (RAM read port). The
        // unresolved-element arm schedules the 1-bit X the interpreter's
        // `Value::new(1).resize_for_assign(width)` produces (zero-extended).
        NbaAssignArrayRead(dst_sig, arr, idx_sig, width) => {
            let dst_sig = sref(*dst_sig);
            // The INDEX is itself read from a signal — map it too. This one
            // stays a plain u32 either way, so the type checker cannot flag
            // it; only an audit of every id interpolation finds it.
            let idx_sig = sref(*idx_sig);
            let ArrayOperand::Dense {
                first_id, lo, hi, ..
            } = arr.as_ref()
            else {
                return None;
            };
            // A dense array's BASE id is mapped like any other signal id: the
            // element address is base + (index - lo), so mapping the base
            // relocates the whole array for this block's instance.
            let first_id = sref(*first_id as u32);
            let _ = writeln!(
                w,
                "let t = ld(sim, {idx_sig});\n\
                 let eff = (t.0 & !t.1) as i64;\n\
                 if eff >= {lo} && eff <= {hi} {{\n\
                   let e = ld(sim, ({first_id} as i64 + (eff - {lo})) as u32);\n\
                   (br().nba4s)(sim, {dst_sig}, e.0, e.1, {width});\n\
                 }} else {{ (br().nba4s)(sim, {dst_sig}, 0u64, 1u64, {width}); }}"
            );
        }
        // Ranged NBA into a dense array element: the range bridge already
        // takes a signal id, so the computed element id slots straight in.
        NbaAssignArrayRange(arr, idx_reg, hi_reg, lo_reg, val_reg) => {
            let ArrayOperand::Dense {
                first_id, lo, hi, ..
            } = arr.as_ref()
            else {
                return None;
            };
            // A dense array's BASE id is mapped like any other signal id: the
            // element address is base + (index - lo), so mapping the base
            // relocates the whole array for this block's instance.
            let first_id = sref(*first_id as u32);
            let _ = writeln!(
                w,
                "let eff = (r{idx_reg}v & !r{idx_reg}x) as i64;\n\
                 if eff >= {lo} && eff <= {hi} {{\n\
                   (br().nba_range)(sim, ({first_id} as i64 + (eff - {lo})) as u32, r{hi_reg}v, r{lo_reg}v, r{val_reg}v, r{val_reg}x);\n\
                 }}"
            );
        }
        BranchIfFalse(cond, target) => {
            let t = jump_pc(*target as usize, n);
            let _ = writeln!(
                w,
                "if (r{cond}v & !r{cond}x) == 0 {{ pc = {t}; continue 'sm; }}"
            );
        }
        // Fused LogNot+BranchIfFalse: jump unless DEFINITE zero (X jumps),
        // the exact composition the interpreter and cranelift implement.
        BranchUnlessZero(cond, target) => {
            let t = jump_pc(*target as usize, n);
            let _ = writeln!(
                w,
                "if (r{cond}v | r{cond}x) != 0 {{ pc = {t}; continue 'sm; }}"
            );
        }
        CmpBranchConst(kind, src, k, tmp, target) => {
            // Reuse the constant comparison emitter, preserving its planes
            // and signed-width rules, before the fused branch.
            if k.is_fill || k.is_real || k.width > 64 {
                return None;
            }
            let cmp = BinOpConst(*tmp, *src, k.clone(), *kind);
            emit_insn_rust(w, tables, ntab, &cmp, i, n, meta, sig_w, sig_signed, sigmap)?;
            let t = jump_pc(*target as usize, n);
            let _ = writeln!(
                w,
                "if (r{tmp}v & !r{tmp}x) == 0 {{ pc = {t}; continue 'sm; }}"
            );
        }
        CmpBranch(kind, l, r, tmp, target) => {
            use crate::compiler::bytecode::CmpKind as CK;
            let t = jump_pc(*target as usize, n);
            match kind {
                CK::CaseEq => {
                    let both = rs(*l) && rs(*r);
                    let (lw, rw_) = (rw(*l), rw(*r));
                    let _ = writeln!(
                        w,
                        "let t = caseeq4(r{l}v, r{l}x, r{r}v, r{r}x, {both}, {lw}, {rw_}); r{tmp}v = t.0; r{tmp}x = t.1;"
                    );
                }
                _ => {
                    let op = match kind {
                        CK::Eq => 0,
                        CK::Neq => 1,
                        CK::Lt => 2,
                        CK::Leq => 3,
                        CK::Gt => 4,
                        CK::Geq => 5,
                        CK::CaseEq => unreachable!(),
                    };
                    let bs = rs(*l) && rs(*r);
                    let (lw, rw_) = (rw(*l), rw(*r));
                    let _ = writeln!(
                        w,
                        "let t = cmp4(r{l}v, r{l}x, r{r}v, r{r}x, {bs}, {lw}, {rw_}, {op}); r{tmp}v = t.0; r{tmp}x = t.1;"
                    );
                }
            }
            let _ = writeln!(
                w,
                "if (r{tmp}v & !r{tmp}x) == 0 {{ pc = {t}; continue 'sm; }}"
            );
        }
        Jump(target) => {
            let t = jump_pc(*target as usize, n);
            let _ = writeln!(w, "pc = {t}; continue 'sm;");
        }
        BranchIfSignalFalse(sig, target, bit) => {
            let t = jump_pc(*target as usize, n);
            let sig = sref(*sig);
            if *bit == u32::MAX {
                let _ = writeln!(
                    w,
                    "let t = ld(sim, {sig}); if (t.0 & !t.1) == 0 {{ pc = {t_pc}; continue 'sm; }}",
                    t_pc = t
                );
            } else if *bit >= 64 {
                // Wide-signal bit: slice bridge, then branch on the lsb.
                let _ = writeln!(
                    w,
                    "let bv = (br().load_slice)(sim, {sig}, {bit}, 1); let bx = (br().load_slice_xz)(sim, {sig}, {bit}, 1); if (bv & !bx) & 1 == 0 {{ pc = {t_pc}; continue 'sm; }}",
                    t_pc = t
                );
            } else {
                let _ = writeln!(
                    w,
                    "let t = ld(sim, {sig}); if ((t.0 & !t.1) >> {bit}) & 1 == 0 {{ pc = {t_pc}; continue 'sm; }}",
                    t_pc = t
                );
            }
        }
        CaseJump(src, cj) => {
            let tn = *ntab;
            *ntab += 1;
            let items: Vec<String> = cj
                .table
                .iter()
                .map(|&t| jump_pc(t as usize, n).to_string())
                .collect();
            let _ = writeln!(
                tables,
                "static T{tn}: [u32; {}] = [{}];",
                items.len(),
                items.join(", ")
            );
            let def = jump_pc(cj.default as usize, n);
            let _ = writeln!(
                w,
                "pc = if r{src}x != 0 {{ {def} }} else {{ *T{tn}.get(r{src}v as usize).unwrap_or(&{def}) }}; continue 'sm;"
            );
        }
        _ => return None,
    }
    let _ = i;
    Some(())
}

fn mask_c(w: u32) -> u64 {
    if w >= 64 { !0 } else { (1u64 << w) - 1 }
}

/// An out-of-range jump target falls off the end (interpreter behavior);
/// map it to a pc value the state machine's `_ =>` arm turns into return.
fn jump_pc(t: usize, n: usize) -> usize {
    if t <= n { t } else { n }
}

/// Compile the generated crate and dlopen it. Returns the dlopen handle
/// (leaked for process lifetime) plus a symbol resolver.
pub struct AotLib {
    handle: *mut libc::c_void,
}

impl AotLib {
    /// Step 15: the single-symbol function table. Returns a resolver
    /// closure over `xezim_native_api`.
    pub fn api(&self) -> Option<impl Fn(u32) -> *mut u8 + '_> {
        let c = std::ffi::CString::new("xezim_native_api").ok()?;
        let p = unsafe { libc::dlsym(self.handle, c.as_ptr()) };
        if p.is_null() {
            return None;
        }
        let f: unsafe extern "C" fn(u32) -> *mut u8 = unsafe { std::mem::transmute(p) };
        Some(move |idx: u32| unsafe { f(idx) })
    }

    pub fn sym_fsm(&self, name: &str) -> Option<AotFsmFn> {
        let c = std::ffi::CString::new(name).ok()?;
        let p = unsafe { libc::dlsym(self.handle, c.as_ptr()) };
        if p.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut libc::c_void, AotFsmFn>(p) })
        }
    }

    pub fn sym(&self, name: &str) -> Option<JitFn> {
        let c = std::ffi::CString::new(name).ok()?;
        let p = unsafe { libc::dlsym(self.handle, c.as_ptr()) };
        if p.is_null() {
            None
        } else {
            Some(unsafe { std::mem::transmute::<*mut libc::c_void, JitFn>(p) })
        }
    }
}

pub fn compile_and_load(source: &str, verbose: bool, planes: (u64, u32)) -> Option<AotLib> {
    // Unique per COMPILE, not per process: several Simulators in one
    // process (cargo test threads) would otherwise overwrite each other's
    // crate — and dlopen caches by PATH, silently handing a test another
    // design's library.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("xezim_aot_{}_{}", std::process::id(), seq));
    std::fs::create_dir_all(&dir).ok()?;
    let rs = dir.join("xezim_aot.rs");
    let so = dir.join("libxezim_aot.so");
    // Roadmap step 16: persistent native cache. The key hashes the FULL
    // generated source (which transitively encodes the design: signal ids,
    // widths, constants), the opt level, target-cpu mode and this xezim
    // build — so any change to the design OR the generator misses cleanly.
    // A hit skips rustc entirely; dlopen still goes through the unique temp
    // path's COPY so per-process libraries never collide.
    let opt_env = std::env::var("XEZIM_AOT_OPT").unwrap_or_else(|_| "2".to_string());
    let cache_so: Option<std::path::PathBuf> =
        if std::env::var_os("XEZIM_NO_NATIVE_CACHE").is_none() {
            let mut h: u64 = 0xcbf29ce484222325;
            let mut feed = |b: &[u8]| {
                for &x in b {
                    h ^= x as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            };
            feed(source.as_bytes());
            feed(opt_env.as_bytes());
            // "nolto": ThinLTO was measured at 57s of the C910 comb
            // crate's 213s compile — cross-function inlining analysis over
            // bodies that are deliberately independent (template dedup
            // REQUIRES them not to inline into each other). -C lto=off
            // -C embed-bitcode=no: 213.1s -> 166.9s (-22%), .so 14 -> 13MB.
            // The marker keeps ThinLTO-built cache entries from being reused.
            feed(b"native-nolto");
            feed(
                option_env!("XEZIM_GIT_HASH")
                    .unwrap_or(env!("CARGO_PKG_VERSION"))
                    .as_bytes(),
            );
            let base = std::env::var_os("XEZIM_CACHE_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from))
                .or_else(|| {
                    std::env::var_os("HOME").map(|hm| std::path::PathBuf::from(hm).join(".cache"))
                });
            base.map(|b| {
                let d = b.join("xezim").join("native");
                let _ = std::fs::create_dir_all(&d);
                d.join(format!("{h:016x}.so"))
            })
        } else {
            None
        };
    if let Some(cp) = &cache_so {
        if cp.exists() && std::fs::copy(cp, &so).is_ok() {
            if let Some(lib) = dlopen_and_bind(&so, planes) {
                if verbose {
                    eprintln!("[AOT] native cache hit ({})", cp.display());
                }
                return Some(lib);
            }
        }
    }
    std::fs::write(&rs, source).ok()?;
    let t0 = std::time::Instant::now();
    // XEZIM_AOT_OPT overrides the opt level (default 2; 3 measured neutral
    // on ibex but may differ on larger generated crates). target-cpu=native
    // lets LLVM use the host's vector/bit ops in the plane algebra.
    let opt = opt_env;
    let opt_arg = format!(
        "opt-level={}",
        if matches!(opt.as_str(), "0" | "1" | "2" | "3") {
            opt.as_str()
        } else {
            "2"
        }
    );
    let out = std::process::Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "cdylib",
            "-C",
            opt_arg.as_str(),
            "-C",
            "target-cpu=native",
            "-C",
            "panic=abort",
            "-C",
            "debuginfo=0",
            "-C",
            "lto=off",
            "-C",
            "embed-bitcode=no",
            "-o",
        ])
        .arg(&so)
        .arg(&rs)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "[AOT] rustc failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    if verbose {
        eprintln!(
            "[AOT] rustc compiled {} bytes of source in {:.1}s",
            source.len(),
            t0.elapsed().as_secs_f64()
        );
    }
    if let Some(cp) = &cache_so {
        // Atomic publish: write beside, rename over.
        let tmp = cp.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::copy(&so, &tmp).is_ok() {
            let _ = std::fs::rename(&tmp, cp);
        }
    }
    dlopen_and_bind(&so, planes)
}

/// dlopen the compiled dylib and bind the runtime bridge table (which
/// carries the SoA plane base — NativeCtx step 1). Shared by the fresh
/// compile path and the step-16 cache-hit path.
fn dlopen_and_bind(so: &std::path::Path, planes: (u64, u32)) -> Option<AotLib> {
    let c = std::ffi::CString::new(so.to_string_lossy().into_owned()).ok()?;
    let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW) };
    if handle.is_null() {
        eprintln!("[AOT] dlopen failed");
        return None;
    }
    let lib = AotLib { handle };
    let bind = lib.sym("xezim_aot_bind")?;
    let bridge = AotBridge {
        load: super::jit::xezim_jit_load_signal,
        load_xz: super::jit::xezim_jit_load_signal_xz,
        store4s: super::jit::xezim_jit_store_signal_4s,
        nba4s: super::jit::xezim_jit_schedule_nba_4s,
        nba_range: super::jit::xezim_jit_schedule_nba_range_dyn,
        nba_bit: super::jit::xezim_jit_schedule_nba_bit_dyn,
        blk_range: super::jit::xezim_jit_blocking_assign_range_dyn,
        load_slice: super::jit::xezim_jit_load_signal_slice,
        load_slice_xz: super::jit::xezim_jit_load_signal_slice_xz,
        planes: planes.0,
        planes_len: planes.1,
    };
    // xezim_aot_bind has the JitFn ABI shape only by accident; cast through
    // the real signature.
    let bind_fn: unsafe extern "C" fn(*const AotBridge) =
        unsafe { std::mem::transmute::<JitFn, unsafe extern "C" fn(*const AotBridge)>(bind) };
    unsafe { bind_fn(&bridge as *const AotBridge) };
    Some(lib)
}

/// Assemble the full crate source from per-block fns.
pub fn module_source(block_fns: &[String]) -> String {
    module_source_named(block_fns, &[])
}

/// Roadmap step 15: alongside the per-fn symbols, export ONE
/// `xezim_native_api(idx) -> *mut u8` table over `names` (in order), so the
/// loader resolves a single symbol instead of one dlsym per block.
pub fn module_source_named(block_fns: &[String], names: &[String]) -> String {
    let mut s =
        String::with_capacity(PRELUDE.len() + block_fns.iter().map(|b| b.len()).sum::<usize>());
    s.push_str(PRELUDE);
    for b in block_fns {
        s.push_str(b);
        s.push('\n');
    }
    if !names.is_empty() {
        s.push_str("#[no_mangle]\npub unsafe extern \"C\" fn xezim_native_api(idx: u32) -> *mut u8 {\nmatch idx {\n");
        for (i, n) in names.iter().enumerate() {
            let _ = writeln!(s, "{i} => {n} as usize as *mut u8,");
        }
        s.push_str("_ => core::ptr::null_mut(),\n} }\n");
    }
    s
}

/// Canonical shape of a compiled block, for template dedup.
///
/// Walks the SAME id-bearing fields the emitter routes through `sref`, in the
/// same order, assigning each distinct signal id a first-use ordinal. Returns
/// the shape key plus the ordinal->id mapping, so the caller can emit ONE body
/// per key and a per-block mapping array. The key MUST be computed from the
/// same map the emitter uses, or two blocks sharing a body would resolve an
/// ordinal to different signals.
///
/// The key folds in each mapped signal's declared width and signedness,
/// because the emitter bakes those into masks, resize widths and sign scrubs —
/// same-shaped blocks over differently-sized signals need different code.
pub fn canon_shape(
    insns: &[Insn],
    sig_w: &[u32],
    sig_signed: &[bool],
) -> (u64, Vec<u32>, std::collections::HashMap<u32, u32>) {
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();
    let mut canon = |id: u32, h: &mut std::collections::hash_map::DefaultHasher| {
        let next = order.len() as u32;
        let ord = *map.entry(id).or_insert_with(|| {
            order.push(id);
            next
        });
        ord.hash(h);
    };
    for insn in insns {
        super::bytecode::insn_opcode_name(insn).hash(&mut h);
        match insn {
            Insn::LoadSignal(d, s) | Insn::LoadSignalSigned(d, s) => {
                d.hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::LoadSignalBit(d, s, b) => {
                (d, b).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::LoadSignalRange(d, s, l, r) => {
                (d, l, r).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::BranchIfSignalFalse(s, t, b) => {
                (t, b).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::BlockingAssign(s, r, w) | Insn::NbaAssign(s, r, w) => {
                (r, w).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::NbaAssignConst(s, v, w) => {
                let (vb, xb) = v.raw_bits();
                (w, vb, xb, v.width, v.is_signed).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::BlockingAssignRange(s, hi, lo, r) | Insn::NbaAssignRange(s, hi, lo, r) => {
                (hi, lo, r).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::BlockingAssignRangeDyn(s, a, b, r) | Insn::NbaAssignRangeDyn(s, a, b, r) => {
                (a, b, r).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::BlockingAssignBitDyn(s, i, r) | Insn::NbaAssignBitDyn(s, i, r) => {
                (i, r).hash(&mut h);
                canon(*s, &mut h);
            }
            Insn::LoadArrayElem(d, arr, i) => {
                d.hash(&mut h);
                i.hash(&mut h);
                match arr.as_ref() {
                    ArrayOperand::Dense {
                        first_id, lo, hi, ..
                    } => {
                        (lo, hi).hash(&mut h);
                        canon(*first_id as u32, &mut h);
                    }
                    // Name-keyed arrays are not emitted natively; refuse.
                    ArrayOperand::Named(_) => return (u64::MAX, Vec::new(), HashMap::new()),
                }
            }
            Insn::BlockingAssignArray(arr, i, r, w) | Insn::NbaAssignArray(arr, i, r, w) => {
                (i, r, w).hash(&mut h);
                match arr.as_ref() {
                    ArrayOperand::Dense {
                        first_id, lo, hi, ..
                    } => {
                        (lo, hi).hash(&mut h);
                        canon(*first_id as u32, &mut h);
                    }
                    ArrayOperand::Named(_) => return (u64::MAX, Vec::new(), HashMap::new()),
                }
            }
            Insn::NbaAssignArrayRead(dst, arr, idx, w) => {
                w.hash(&mut h);
                canon(*dst, &mut h);
                canon(*idx, &mut h);
                match arr.as_ref() {
                    ArrayOperand::Dense {
                        first_id, lo, hi, ..
                    } => {
                        (lo, hi).hash(&mut h);
                        canon(*first_id as u32, &mut h);
                    }
                    ArrayOperand::Named(_) => return (u64::MAX, Vec::new(), HashMap::new()),
                }
            }
            // Everything else carries no signal identity: its Debug form IS
            // its canonical form.
            other => format!("{:?}", other).hash(&mut h),
        }
    }
    for &id in &order {
        let w = sig_w.get(id as usize).copied().unwrap_or(0);
        let sg = sig_signed.get(id as usize).copied().unwrap_or(false);
        (w, sg).hash(&mut h);
    }
    (h.finish(), order, map)
}

/// A template body's runtime signature: `(sim, map_ptr) -> rc`, where
/// `map_ptr` is the calling block's ordinal->signal-id array (host-owned).
pub type AotTplFn = unsafe extern "C" fn(*mut u8, *const u32) -> u32;

/// Result of grouping blocks into shared templates.
pub struct TemplateBuild {
    /// Generated template bodies, in `tpl_names` order.
    pub fns: Vec<String>,
    /// Exported body names, in API-table order.
    pub tpl_names: Vec<String>,
    /// Per enrolled block: `(block index, position in tpl_names, mapping)`.
    /// The mapping is the block's first-use-ordinal -> signal-id array; the
    /// HOST stores it and passes a pointer at call time. Keeping the maps
    /// host-side is the point: the generated crate contains ONLY the bodies
    /// plus the api table, so its item count equals the template count.
    pub blocks: Vec<(usize, u32, Vec<u32>)>,
    pub n_templates: usize,
    pub n_blocks: usize,
}

/// Group blocks by canonical shape and emit ONE exported body per shape.
///
/// Measured on C910: 108,248 comb blocks collapse to 8,172 shapes (13.2x).
/// A shape whose representative fails to generate is dropped WHOLE — every
/// block in it falls back to the interpreter — because the blocks in a shape
/// share one body by construction.
pub fn gen_templated_blocks(
    items: &[(usize, Vec<Insn>, u32)],
    sig_w: &[u32],
    sig_signed: &[bool],
) -> TemplateBuild {
    use std::collections::HashMap;
    let mut groups: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut shapes: Vec<(u64, Vec<u32>, HashMap<u32, u32>)> = Vec::with_capacity(items.len());
    for (i, (_, insns, _)) in items.iter().enumerate() {
        let (key, order, map) = canon_shape(insns, sig_w, sig_signed);
        if key == u64::MAX {
            shapes.push((u64::MAX, Vec::new(), HashMap::new()));
            continue;
        }
        groups.entry(key).or_default().push(i);
        shapes.push((key, order, map));
    }
    let mut fns: Vec<String> = Vec::new();
    let mut tpl_names: Vec<String> = Vec::new();
    let mut blocks: Vec<(usize, u32, Vec<u32>)> = Vec::new();
    // Deterministic order: generated source must not depend on hash iteration
    // (the persistent native cache keys on the source text).
    let mut keys: Vec<u64> = groups.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        let members = &groups[&key];
        let rep = members[0];
        let tpl = format!("xezim_aot_tpl_{key:016x}");
        let (_, _, rep_map) = &shapes[rep];
        let Some(body) = gen_block_fn_mapped(
            &tpl,
            &items[rep].1,
            items[rep].2,
            sig_w,
            sig_signed,
            Some(rep_map),
        ) else {
            continue; // whole shape unsupported; its blocks stay interpreted
        };
        let tpl_pos = tpl_names.len() as u32;
        fns.push(body);
        tpl_names.push(tpl);
        for &m in members {
            blocks.push((items[m].0, tpl_pos, shapes[m].1.clone()));
        }
    }
    let (n_templates, n_blocks) = (tpl_names.len(), blocks.len());
    TemplateBuild {
        fns,
        tpl_names,
        blocks,
        n_templates,
        n_blocks,
    }
}
