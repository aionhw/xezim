//! Word-level helpers of the bytecode VM: wide-word shifts and replication, out-of-range
//! selects, the vector fast paths and the X/Z-drop and real-to-int conversions.

use super::*;

/// §11.5.1: the effective index of a DYNAMIC bit-select. `None` when the index
/// contains x or z, in which case a WRITE must be discarded ("no bits shall be
/// modified") and a read yields x.
///
/// `Value::to_u64` masks x bits to ZERO and returns `Some(0)` rather than
/// `None`, so it cannot answer this question. Every dynamic bit-store used
/// `to_u64().unwrap_or(0)` and therefore wrote BIT 0 whenever the index was
/// unknown. Out-of-range indices need no help here: `Value::set_bit` already
/// drops them. Same defect, and same fix, as the `LoadArrayElem` x-index
/// sentinel on the read side.

/// §11.5.1 part-select whose physical range `[hi:lo]` may leave `[0, w)`.
/// Out-of-range positions read x, and the in-range positions read their
/// bits (IEEE 1800 §11.5.1; differential-verified against a reference
/// simulator in `tests/misc/operators_11_select_reduce.rs`). Some tools
/// read the WHOLE select x instead when any position is out of range;
/// `XEZIM_OOB_SELECT=whole` selects that form.
/// `XEZIM_OOB_SELECT=whole`: a partially out-of-range select reads x as a
/// whole and a partially out-of-range WRITE is discarded (some tools);
/// default is the §11.5.1 per-bit form for both.
// ---- Wide two-state register word helpers (little-endian `[u64; N]`). ----

/// Top word index of a `w`-bit value and the mask of its live bits.
#[inline(always)]
pub(super) fn wtop(w: u16) -> (usize, u64) {
    let top = ((w as usize).max(1) - 1) >> 6;
    let r = w as u32 - (top as u32) * 64;
    (top, if r >= 64 { u64::MAX } else { (1u64 << r) - 1 })
}

/// Truncate `x` to `w` bits in place.
#[inline(always)]
pub(super) fn wmask_top<const N: usize>(x: &mut [u64; N], w: u16) {
    let (top, m) = wtop(w);
    x[top] &= m;
    for i in top + 1..N {
        x[i] = 0;
    }
}

#[inline(always)]
fn repeated_word(part: u64, width: u8) -> Option<u64> {
    match width {
        1 => Some(0u64.wrapping_sub(part & 1)),
        2 => Some((part & 0x3).wrapping_mul(0x5555_5555_5555_5555)),
        4 => Some((part & 0xf).wrapping_mul(0x1111_1111_1111_1111)),
        8 => Some((part & 0xff).wrapping_mul(0x0101_0101_0101_0101)),
        16 => Some((part & 0xffff).wrapping_mul(0x0001_0001_0001_0001)),
        32 => Some((part & 0xffff_ffff).wrapping_mul(0x0000_0001_0000_0001)),
        64 => Some(part),
        _ => None,
    }
}

#[inline(always)]
pub(super) fn replicate_narrow(part: u64, width: u8, count: u8) -> u64 {
    if count == 0 {
        return 0;
    }
    if count == 1 {
        return part;
    }
    let total = width as u16 * count as u16;
    if let Some(word) = repeated_word(part, width) {
        return if total >= 64 {
            word
        } else {
            word & ((1u64 << total) - 1)
        };
    }

    let mut acc = 0u64;
    let mut chunk = part;
    let mut chunk_width = width as u32;
    let mut copies = count;
    while copies != 0 {
        if copies & 1 != 0 {
            acc = (acc << chunk_width) | chunk;
        }
        copies >>= 1;
        if copies != 0 {
            chunk |= chunk << chunk_width;
            chunk_width *= 2;
        }
    }
    acc
}

#[inline(always)]
pub(super) fn replicate_wide<const N: usize>(part: u64, width: u8, count: u16) -> [u64; N] {
    if count == 0 {
        return [0u64; N];
    }
    if count == 1 {
        let mut acc = [0u64; N];
        acc[0] = part;
        return acc;
    }
    let total = width as u16 * count;
    if let Some(word) = repeated_word(part, width) {
        let mut acc = [word; N];
        wmask_top(&mut acc, total);
        return acc;
    }

    let mut acc = [0u64; N];
    let mut chunk = [0u64; N];
    chunk[0] = part;
    let mut chunk_width = width as u32;
    let mut copies = count;
    while copies != 0 {
        if copies & 1 != 0 {
            acc = wshl(acc, chunk_width);
            for i in 0..N {
                acc[i] |= chunk[i];
            }
        }
        copies >>= 1;
        if copies != 0 {
            let shifted = wshl(chunk, chunk_width);
            for i in 0..N {
                chunk[i] |= shifted[i];
            }
            chunk_width *= 2;
        }
    }
    acc
}

/// `x << sh` across N words (bits shifted past the top word are dropped).
#[inline(always)]
pub(super) fn wshl<const N: usize>(x: [u64; N], sh: u32) -> [u64; N] {
    if sh == 0 {
        return x;
    }
    let ws = (sh >> 6) as usize;
    let bs = sh & 63;
    let mut r = [0u64; N];
    for i in ws..N {
        let mut v = x[i - ws] << bs;
        if bs != 0 && i > ws {
            v |= x[i - ws - 1] >> (64 - bs);
        }
        r[i] = v;
    }
    r
}

/// `x >> sh` across N words.
#[inline(always)]
pub(super) fn wshr<const N: usize>(x: [u64; N], sh: u32) -> [u64; N] {
    if sh == 0 {
        return x;
    }
    let ws = (sh >> 6) as usize;
    let bs = sh & 63;
    let mut r = [0u64; N];
    for i in 0..N.saturating_sub(ws) {
        let mut v = x[i + ws] >> bs;
        if bs != 0 && i + ws + 1 < N {
            v |= x[i + ws + 1] << (64 - bs);
        }
        r[i] = v;
    }
    r
}

/// The 64 bits of `x` starting at bit `lo`.
#[inline(always)]
pub(super) fn wget64<const N: usize>(x: &[u64; N], lo: u32) -> u64 {
    let ws = (lo >> 6) as usize;
    let bs = lo & 63;
    if ws >= N {
        return 0;
    }
    let mut v = x[ws] >> bs;
    if bs != 0 && ws + 1 < N {
        v |= x[ws + 1] << (64 - bs);
    }
    v
}

pub(crate) fn oob_select_whole() -> bool {
    static WHOLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WHOLE.get_or_init(|| std::env::var("XEZIM_OOB_SELECT").ok().as_deref() == Some("whole"))
}

/// §11.5.1 partial-write rule: `true` when a write through a select whose
/// physical range `[hi:lo]` leaves `[0, w)` must be discarded entirely
/// (`XEZIM_OOB_SELECT=whole`); by default only the in-range bits are stored.
#[inline]
pub(crate) fn oob_write_discard(hi: i64, lo: i64, w: u32) -> bool {
    oob_select_whole() && (lo < 0 || hi >= w as i64)
}

/// Outcome of a two-state dynamic-offset slice read.
pub(super) enum TsDyn {
    Ok(u64),
    Oob,
    X,
}

#[inline]
pub(crate) fn oob_range_select(base: &Value, hi: i64, lo: i64) -> Value {
    let per_bit = !oob_select_whole();
    if hi < lo {
        return Value::new(1);
    }
    let width = (hi - lo + 1) as u32;
    let w = base.width as i64;
    if lo >= 0 && hi < w {
        return base.range_select(hi as usize, lo as usize);
    }
    if !per_bit && !base.is_fill {
        return Value::new(width);
    }
    base.range_select_signed(hi, lo)
}

#[inline]
pub(super) fn dyn_bit_index(v: &Value) -> Option<usize> {
    if v.has_xz() {
        return None;
    }
    Some(v.to_u64().unwrap_or(0) as usize)
}

// ===========================================================================
// Inline (<=64-bit) fast paths for the bytecode VM's hot ALU arms.
//
// `exec_insns` used to write EVERY result as
//     self.vm_regs[d] = self.vm_regs[l].op(&self.vm_regs[r]);
// which is a call that CONSTRUCTS a fresh 32-byte `Value`, a 32-byte move
// into the destination register, and a `Drop` of the destination's previous
// contents (a branch on the storage discriminant plus `free` when it was
// `Wide`). For the overwhelmingly common case — both operands in `Inline`
// storage — the whole result is two `u64`s plus a width and a sign flag, so
// the destination can simply be overwritten in place.
//
// Each `vm_*` helper below mirrors the corresponding `Value` method EXACTLY
// for the operand shapes it accepts, and returns `None` for every other shape
// so the caller falls back to that same method. That keeps the 4-state
// (0/1/X/Z) rules in one place: anything not reproduced bit-for-bit here is
// simply not handled here. `vm_fastpath_tests` below checks the equivalence
// exhaustively over all 4-state patterns of widths 1..=5.
// ===========================================================================

/// `Value::mask` — bit mask for the valid bits of an inline value.
#[inline(always)]
pub(super) fn vm_mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

/// `(val_bits, xz_bits, width, is_signed)` for a value eligible for an inline
/// fast path: `Inline` storage, not a real, not a §5.7.1 fill, width <= 64.
/// Rejecting a shape is always safe — the caller then runs the original
/// `Value` method, which is where every rejected shape was handled before.
#[inline(always)]
fn vm_hdr(v: &Value) -> Option<(u64, u64, u32, bool)> {
    if v.is_real || v.is_fill || v.width > 64 {
        return None;
    }
    match v.inline_bits() {
        Some((val, xz)) => Some((val, xz, v.width, v.is_signed)),
        None => None,
    }
}

/// Overwrite `dst` in place with an inline result — the whole point of this
/// module. Only a destination that currently holds `Wide` storage needs the
/// old construct/move/drop, and that path also frees the heap buffer.
#[inline(always)]
pub(super) fn vm_store(dst: &mut Value, val: u64, xz: u64, width: u32, signed: bool) {
    if width > 64 {
        // Inline storage holds 64 bits by invariant. An element wider than
        // that reached here through the x/z-index arm of LoadArrayElem (the
        // 140-bit store-buffer entry read through an x pointer at time 0):
        // the inline form then read past its planes, and in release the
        // shift wrapped and returned bit-pattern garbage instead of x.
        let mut v = Value::new(width);
        for i in 0..64usize {
            let b = match ((val >> i) & 1, (xz >> i) & 1) {
                (0, 0) => LogicBit::Zero,
                (1, 0) => LogicBit::One,
                (0, 1) => LogicBit::X,
                _ => LogicBit::Z,
            };
            v.set_bit(i, b);
        }
        v.is_signed = signed;
        *dst = v;
        return;
    }
    if dst.set_inline_bits(val, xz) {
        dst.width = width;
        dst.is_signed = signed;
        dst.is_real = false;
        dst.is_fill = false;
    } else {
        let mut v = Value::from_inline(val, xz, width);
        v.is_signed = signed;
        *dst = v;
    }
}

/// `Value::to_u64().unwrap_or(0)` on inline bits.
#[inline(always)]
fn vm_to_u64(val: u64, xz: u64) -> u64 {
    val & !xz
}

/// `Value::to_i64().unwrap_or(0) as u64` on inline bits.
#[inline(always)]
fn vm_to_i64(val: u64, xz: u64, width: u32, signed: bool) -> u64 {
    let raw = val & !xz;
    if signed && width > 0 && width < 64 && (raw >> (width - 1)) & 1 == 1 {
        raw | !vm_mask(width)
    } else {
        raw
    }
}

/// `Value::operand_bits_u64`.
#[inline(always)]
fn vm_operand_bits(
    val: u64,
    xz: u64,
    w_self: u32,
    signed_self: bool,
    signed_expr: bool,
    w: u32,
) -> u64 {
    if signed_expr && w_self < w && w_self < 64 {
        vm_to_i64(val, xz, w_self, signed_self)
    } else {
        vm_to_u64(val, xz)
    }
}

/// `Value::resize`'s inline head, as raw bits. `None` for the shapes that
/// head sends to `resize_slow`.
#[inline(always)]
pub(super) fn vm_resize_bits(
    val: u64,
    xz: u64,
    width: u32,
    signed: bool,
    target: u32,
) -> Option<(u64, u64)> {
    if target == 0 || target > 64 {
        return None;
    }
    if target == width {
        return Some((val, xz));
    }
    let mask = vm_mask(target);
    if target < width {
        return Some((val & mask, xz & mask));
    }
    // Widen: sign-extend only for a signed source whose MSB is a KNOWN 1
    // (an X/Z MSB does not replicate here — that is `resize_for_assign`'s job).
    let mut v = val;
    if signed
        && width > 0
        && width <= 64
        && (xz >> (width - 1)) & 1 == 0
        && (val >> (width - 1)) & 1 == 1
    {
        v |= mask & !vm_mask(width);
    }
    Some((v, xz))
}

/// `Value::add` (`sub == false`) / `Value::sub` (`sub == true`).
#[inline(always)]
pub(super) fn vm_add_sub(a: &Value, b: &Value, sub: bool) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    let (bv, bx, bw, bsg) = vm_hdr(b)?;
    let w = aw.max(bw);
    if ax != 0 || bx != 0 {
        // `Value::new(w)`: all-X, unsigned.
        return Some((0, vm_mask(w), w, false));
    }
    let signed = asg && bsg;
    let x = vm_operand_bits(av, ax, aw, asg, signed, w);
    let y = vm_operand_bits(bv, bx, bw, bsg, signed, w);
    let r = if sub {
        x.wrapping_sub(y)
    } else {
        x.wrapping_add(y)
    };
    Some((r & vm_mask(w), 0, w, signed))
}

/// `Value::bitwise_and`.
#[inline(always)]
pub(super) fn vm_bitand(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, _) = vm_hdr(a)?;
    let (bv, bx, bw, _) = vm_hdr(b)?;
    let w = aw.max(bw);
    if ax == 0 && bx == 0 {
        Some((av & bv, 0, w, false))
    } else {
        let any_xz = ax | bx;
        let result_val = av & bv & !any_xz;
        let result_xz = any_xz & !((!av & !ax) | (!bv & !bx)); // known 0 kills X
        Some((result_val, result_xz & vm_mask(w), w, false))
    }
}

/// `Value::bitwise_or`.
#[inline(always)]
pub(super) fn vm_bitor(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, _) = vm_hdr(a)?;
    let (bv, bx, bw, _) = vm_hdr(b)?;
    let w = aw.max(bw);
    if ax == 0 && bx == 0 {
        Some((av | bv, 0, w, false))
    } else {
        let any_xz = ax | bx;
        let result_val = (av | bv) & !any_xz;
        let known_one = (av & !ax) | (bv & !bx);
        let result_xz = any_xz & !known_one; // known 1 kills X
        Some((result_val | known_one, result_xz & vm_mask(w), w, false))
    }
}

/// `Value::bitwise_xor`.
#[inline(always)]
pub(super) fn vm_bitxor(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, _) = vm_hdr(a)?;
    let (bv, bx, bw, _) = vm_hdr(b)?;
    let w = aw.max(bw);
    let any_xz = ax | bx;
    Some(((av ^ bv) & !any_xz, any_xz & vm_mask(w), w, false))
}

/// `Value::bitwise_not` (keeps `is_signed`, unlike the binary bitwise ops).
#[inline(always)]
pub(super) fn vm_bitnot(a: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    Some(((!av & !ax) & vm_mask(aw), ax, aw, asg))
}

/// `Value::bitwise_xor(..).bitwise_not()` — the `BitXnor` arm's composition.
#[inline(always)]
pub(super) fn vm_bitxnor(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (v, x, w, s) = vm_bitxor(a, b)?;
    Some(((!v & !x) & vm_mask(w), x, w, s))
}

/// `Value::is_equal` restricted to X-free operands. The X case returns `None`
/// so the LRM's bit-by-bit ambiguity rule stays in the original method.
#[inline(always)]
pub(super) fn vm_is_equal(a: &Value, b: &Value) -> Option<bool> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    let (bv, bx, bw, bsg) = vm_hdr(b)?;
    if ax != 0 || bx != 0 {
        return None;
    }
    // §11.6.1/§11.8.2: the comparison is SIGNED only when BOTH operands are
    // signed; if either is unsigned the propagated type is unsigned and the
    // narrower operand is ZERO-extended. Mirrors `Value::is_equal` — this fast
    // path is asserted equal to it by `binary_ops_match_value_methods`, which
    // is what caught the two implementations drifting apart.
    if aw != bw {
        let w = aw.max(bw);
        let both_signed = asg && bsg;
        let (ra, rax) = vm_resize_bits(av, ax, aw, both_signed, w)?;
        let (rb, rbx) = vm_resize_bits(bv, bx, bw, both_signed, w)?;
        Some(vm_to_u64(ra, rax) == vm_to_u64(rb, rbx))
    } else {
        Some(vm_to_u64(av, ax) == vm_to_u64(bv, bx))
    }
}

/// `Value::case_eq`'s own inline fast path, reproduced verbatim so the result
/// can be stored in place instead of materialised as a `Value`.
#[inline(always)]
pub(super) fn vm_case_eq(a: &Value, b: &Value) -> Option<bool> {
    if a.is_fill || b.is_fill {
        return None;
    }
    let (mut av, mut ax) = a.inline_bits()?;
    let (mut bv, mut bx) = b.inline_bits()?;
    let (aw, bw) = (a.width, b.width);
    let w = aw.max(bw);
    let mask = vm_mask(w);
    if a.is_signed && b.is_signed {
        if aw > 0 && aw < w {
            let ext = mask & !vm_mask(aw);
            let sign = 1u64 << (aw - 1);
            if av & sign != 0 {
                av |= ext;
            }
            if ax & sign != 0 {
                ax |= ext;
            }
        }
        if bw > 0 && bw < w {
            let ext = mask & !vm_mask(bw);
            let sign = 1u64 << (bw - 1);
            if bv & sign != 0 {
                bv |= ext;
            }
            if bx & sign != 0 {
                bx |= ext;
            }
        }
    }
    Some((av & mask) == (bv & mask) && (ax & mask) == (bx & mask))
}

/// `Value::less_than` (`or_equal == false`) / `Value::less_equal`.
/// Returns the (val_bits, xz_bits) of the 1-bit result.
#[inline(always)]
pub(super) fn vm_less(a: &Value, b: &Value, or_equal: bool) -> Option<(u64, u64)> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    let (bv, bx, bw, bsg) = vm_hdr(b)?;
    if ax != 0 || bx != 0 {
        return Some((0, 1)); // `Value::new(1)`
    }
    // §5.5.1: signed compare only when BOTH operands are signed.
    let r = if asg && bsg {
        let x = vm_to_i64(av, ax, aw, asg) as i64;
        let y = vm_to_i64(bv, bx, bw, bsg) as i64;
        if or_equal { x <= y } else { x < y }
    } else {
        let x = vm_to_u64(av, ax);
        let y = vm_to_u64(bv, bx);
        if or_equal { x <= y } else { x < y }
    };
    Some((r as u64, 0))
}

/// `Value::is_nonzero` on inline bits: 0 = false, 1 = true, 2 = unknown.
#[inline(always)]
fn vm_nonzero(val: u64, xz: u64) -> u8 {
    if val & !xz != 0 {
        1
    } else if xz != 0 {
        2
    } else {
        0
    }
}

/// `Value::logic_and` / `Value::logic_or`, as the 1-bit result's raw bits.
#[inline(always)]
pub(super) fn vm_logic2(a: &Value, b: &Value, is_or: bool) -> Option<(u64, u64)> {
    let (av, ax, _, _) = vm_hdr(a)?;
    let (bv, bx, _, _) = vm_hdr(b)?;
    let p = vm_nonzero(av, ax);
    let q = vm_nonzero(bv, bx);
    Some(if is_or {
        if p == 1 || q == 1 {
            (1, 0)
        } else if p == 0 && q == 0 {
            (0, 0)
        } else {
            (0, 1)
        }
    } else {
        if p == 1 && q == 1 {
            (1, 0)
        } else if p == 0 || q == 0 {
            (0, 0)
        } else {
            (0, 1)
        }
    })
}

/// `Value::logic_not`, as the 1-bit result's raw bits.
#[inline(always)]
pub(super) fn vm_logic_not(a: &Value) -> Option<(u64, u64)> {
    let (av, ax, _, _) = vm_hdr(a)?;
    Some(match vm_nonzero(av, ax) {
        1 => (0, 0),
        0 => (1, 0),
        _ => (0, 1),
    })
}

/// `Value::bit_select`'s inline head, as the 1-bit result's raw bits.
/// Deliberately does NOT test `is_fill`/`is_real`: neither does that head,
/// and a divergence there would change §5.7.1 fill behaviour.
#[inline(always)]
pub(super) fn vm_bit_select(v: &Value, index: usize) -> Option<(u64, u64)> {
    let (val, xz) = v.inline_bits()?;
    if index < v.width as usize && index < 64 {
        Some(((val >> index) & 1, (xz >> index) & 1))
    } else {
        None
    }
}

/// `Value::concat_refs(repeat_n(src, n))` restricted to a result that fits
/// inline. Every part is the SAME value, so the width sum `concat_refs` walks
/// its iterator to compute is one multiply here, and the fold needs no
/// iterator. `None` (result wider than 64 bits, or a width multiply that
/// overflows) falls back to `concat_refs` itself.
#[inline(always)]
pub(super) fn vm_replicate(src: &Value, n: usize) -> Option<(u64, u64, u32)> {
    let w = src.width;
    let total = w.checked_mul(n as u32)?;
    if total > 64 {
        return None;
    }
    let (v, x) = src.raw_bits();
    let mask = vm_mask(w);
    let (mut ov, mut ox, mut off) = (0u64, 0u64, 0u32);
    // A zero-width part is SKIPPED by `concat_refs`, leaving the accumulator
    // untouched — so `total == 0` yields a zero-width, all-clear value.
    if w != 0 {
        for _ in 0..n {
            ov |= (v & mask) << off;
            ox |= (x & mask) << off;
            off += w;
        }
    }
    Some((ov, ox, total))
}

/// `Value::range_select`'s inline head, as (val_bits, xz_bits, width).
#[inline(always)]
pub(super) fn vm_range_select(v: &Value, left: usize, right: usize) -> Option<(u64, u64, u32)> {
    if v.is_fill {
        return None;
    }
    let (val, xz) = v.inline_bits()?;
    let (lo, hi) = if left >= right {
        (right, left)
    } else {
        (left, right)
    };
    if hi < 64 && hi < v.width as usize {
        let width = hi - lo + 1;
        let mask = if width == 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        Some((((val >> lo) & mask), ((xz >> lo) & mask), width as u32))
    } else {
        None
    }
}

#[cfg(test)]
mod vm_fastpath_tests {
    use super::*;

    /// §9.6.2: terminating a parked process releases its captured value.
    #[test]
    fn canceled_property_waits_release_values() {
        let sim = crate::simulate(
            r#"
class watch_cell; bit flag; endclass
module top;
  watch_cell watched = new();
  initial begin
    repeat (16) begin
      fork begin @(watched.flag); $fatal(1, "unexpected wake"); end join_none
      #1;
      disable fork;
    end
    $finish;
  end
endmodule
"#,
            100,
        )
        .expect("simulate");
        assert!(sim.vc_saved.is_empty(), "canceled value captures remain");
    }

    /// Every 4-state pattern of `width` bits, as (val_bits, xz_bits).
    fn patterns(width: u32) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let n = width as usize;
        for code in 0..4usize.pow(n as u32) {
            let (mut v, mut x) = (0u64, 0u64);
            for i in 0..n {
                match (code >> (2 * i)) & 3 {
                    0 => {}           // 0
                    1 => v |= 1 << i, // 1
                    2 => x |= 1 << i, // X
                    _ => {
                        v |= 1 << i;
                        x |= 1 << i;
                    } // Z
                }
            }
            out.push((v, x));
        }
        out
    }

    fn mk(v: u64, x: u64, w: u32, signed: bool) -> Value {
        let mut val = Value::from_inline(v, x, w);
        val.is_signed = signed;
        val
    }

    /// All (value, signedness) operands over widths 1..=5.
    fn operands() -> Vec<Value> {
        let mut out = Vec::new();
        for w in 1..=5u32 {
            for (v, x) in patterns(w) {
                out.push(mk(v, x, w, false));
                out.push(mk(v, x, w, true));
            }
        }
        out
    }

    /// A fast-path result and the reference `Value` must agree on the bits AND
    /// on every header field the VM later reads back.
    fn same(fast: (u64, u64, u32, bool), reference: &Value) {
        let (fv, fx, fw, fs) = fast;
        let (rv, rx) = reference.raw_bits();
        let m = vm_mask(reference.width);
        assert_eq!(fw, reference.width, "width");
        assert_eq!(fs, reference.is_signed, "is_signed");
        assert!(!reference.is_real && !reference.is_fill);
        assert_eq!(fv & m, rv & m, "val_bits");
        assert_eq!(fx & m, rx & m, "xz_bits");
        // The unmasked words must match too — `to_u64`/`is_nonzero`/`has_xz`
        // all read them without masking.
        assert_eq!(fv, rv, "val_bits (unmasked)");
        assert_eq!(fx, rx, "xz_bits (unmasked)");
    }

    #[test]
    fn binary_ops_match_value_methods() {
        let ops = operands();
        for a in &ops {
            for b in &ops {
                if let Some(r) = vm_add_sub(a, b, false) {
                    same(r, &a.add(b));
                }
                if let Some(r) = vm_add_sub(a, b, true) {
                    same(r, &a.sub(b));
                }
                if let Some(r) = vm_bitand(a, b) {
                    same(r, &a.bitwise_and(b));
                }
                if let Some(r) = vm_bitor(a, b) {
                    same(r, &a.bitwise_or(b));
                }
                if let Some(r) = vm_bitxor(a, b) {
                    same(r, &a.bitwise_xor(b));
                }
                if let Some(r) = vm_bitxnor(a, b) {
                    same(r, &a.bitwise_xor(b).bitwise_not());
                }
                if let Some(e) = vm_is_equal(a, b) {
                    same((e as u64, 0, 1, false), &a.is_equal(b));
                    same((!e as u64, 0, 1, false), &a.is_not_equal(b));
                }
                if let Some(e) = vm_case_eq(a, b) {
                    same((e as u64, 0, 1, false), &a.case_eq(b));
                }
                if let Some((v, x)) = vm_less(a, b, false) {
                    same((v, x, 1, false), &a.less_than(b));
                    same((v, x, 1, false), &b.greater_than(a));
                }
                if let Some((v, x)) = vm_less(a, b, true) {
                    same((v, x, 1, false), &a.less_equal(b));
                    same((v, x, 1, false), &b.greater_equal(a));
                }
                if let Some((v, x)) = vm_logic2(a, b, false) {
                    same((v, x, 1, false), &a.logic_and(b));
                }
                if let Some((v, x)) = vm_logic2(a, b, true) {
                    same((v, x, 1, false), &a.logic_or(b));
                }
            }
        }
    }

    #[test]
    fn unary_ops_and_resize_match_value_methods() {
        for a in operands() {
            if let Some(r) = vm_bitnot(&a) {
                same(r, &a.bitwise_not());
            }
            if let Some((v, x)) = vm_logic_not(&a) {
                same((v, x, 1, false), &a.logic_not());
            }
            let (av, ax, aw, asg) = vm_hdr(&a).unwrap();
            for target in 0..=9u32 {
                if let Some((rv, rx)) = vm_resize_bits(av, ax, aw, asg, target) {
                    let reference = a.resize(target);
                    same((rv, rx, target, a.is_signed), &reference);
                }
            }
        }
    }

    #[test]
    fn selects_match_value_methods() {
        for a in operands() {
            for i in 0..8usize {
                if let Some((v, x)) = vm_bit_select(&a, i) {
                    same((v, x, 1, false), &a.bit_select(i));
                    // The `BranchIfSignalFalse` bit path replaces
                    // `bit_select(..).is_true()` with this test.
                    assert_eq!(v & !x != 0, a.bit_select(i).is_true());
                }
                for j in 0..8usize {
                    if let Some((v, x, w)) = vm_range_select(&a, i, j) {
                        same((v, x, w, false), &a.range_select(i, j));
                    }
                }
            }
        }
    }

    #[test]
    fn replicate_matches_concat_refs() {
        for a in operands() {
            for n in 2..=9usize {
                if let Some((v, x, w)) = vm_replicate(&a, n) {
                    let reference = Value::concat_refs(std::iter::repeat_n(&a, n));
                    same((v, x, w, false), &reference);
                }
            }
        }
    }

    #[test]
    fn repetition_helpers_match_shift_reference() {
        for width in 1u8..=64 {
            let mask = if width == 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            let part = 0xd6a5_39c7_b84e_102f & mask;

            for count in 0u8..=64 / width {
                let mut reference = 0u64;
                if width == 64 {
                    if count != 0 {
                        reference = part;
                    }
                } else {
                    for _ in 0..count {
                        reference = (reference << width) | part;
                    }
                }
                assert_eq!(
                    replicate_narrow(part, width, count),
                    reference,
                    "narrow width={width} count={count}"
                );
            }

            let max_count = 512u16 / width as u16;
            for count in [0u16, 1, 2, 3, max_count] {
                if count > max_count {
                    continue;
                }
                let mut reference = [0u64; 8];
                for _ in 0..count {
                    reference = wshl(reference, width as u32);
                    reference[0] |= part;
                }
                assert_eq!(
                    replicate_wide::<8>(part, width, count),
                    reference,
                    "wide width={width} count={count}"
                );
            }
        }
    }

    /// `vm_store` must leave the register in exactly the state a full
    /// assignment would have, for both an `Inline` and a `Wide` destination.
    #[test]
    fn store_overwrites_in_place() {
        for &(v, x, w, s) in &[(0u64, 0u64, 1u32, false), (0xff, 0x0f, 8, true)] {
            let mut dst = Value::from_inline(0xdead, 0xbeef, 3);
            dst.is_signed = true;
            vm_store(&mut dst, v, x, w, s);
            assert_eq!(dst.raw_bits(), (v, x));
            assert_eq!(dst.width, w);
            assert_eq!(dst.is_signed, s);
            assert!(!dst.is_real && !dst.is_fill);

            let mut wide = Value::zero(96); // Wide storage
            vm_store(&mut wide, v, x, w, s);
            assert_eq!(wide.raw_bits(), (v, x));
            assert_eq!(wide.width, w);
            assert_eq!(wide.is_signed, s);
        }
    }
}

/// `Insn::DropXz`: map a register's X/Z bits to 0 (§6.11.1). Out of line so
/// the VM dispatch loops carry one small arm for it.
#[cold]
#[inline(never)]
pub(super) fn drop_xz(r: &mut Value) {
    if r.has_xz() {
        *r = r.to_two_state();
    }
}

/// `Insn::RealToInt`: §6.12.1 — a real rounds to a 64-bit integer, ties
/// away from zero (a target narrower than 64 bits wraps it at the `Resize`
/// that follows); any other value is left as it is.
#[inline]
pub(super) fn real_to_int_reg(r: &mut Value) {
    if r.is_real {
        *r = Value::from_real_wrapped(r.to_f64(), 64);
    }
}
