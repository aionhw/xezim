//! Packed signal memory: segment ids, segments and bit splicing.

use super::*;

/// Packed word storage for large memories (on by default;
/// `XEZIM_PACKED_MEM=0` turns it off).
///
/// c906 census: 35.06 M of its 35.11 M signals are array cells, and the big
/// ones are all BYTE-wide RAMs — 16 separate 8-bit `ram*.mem` arrays standing
/// in for one 128-bit word. As ordinary signals each cell costs a 32 B
/// `Value` plus ~24 B of per-signal side tables (width/signed/real/
/// inline_bits/has_xz/dirty) to carry 8 bits. In the arena a cell is its
/// value plane plus its x/z plane, each rounded up to 1, 2, 4 or 8 bytes and
/// stored side by side, so one access touches one cache line; a two-state
/// element has no x/z plane at all. Declared element metadata (width,
/// signedness, two-state) is kept once per ARRAY, not per cell.
///
/// ADMISSION: an integral element of at most 64 bits in an array big enough
/// to have skipped per-element name registration (see `packed_admissible`).
///
/// IDS: a cell id is `PACKED_BASE | segment << SEG_SHIFT | offset`. Within
/// one array the ids are consecutive, so the `first_id + (idx - lo)` cell
/// arithmetic every caller already performs stays correct with no change.
/// Every id at or above `PACKED_BASE` indexes far past the end of
/// `signal_table`, so a site that still indexes the table with one fails
/// loudly (out of bounds) rather than silently reading the wrong cell.
pub(crate) const PACKED_BASE: usize = 1usize << 48;

const PACKED_SEG_SHIFT: u32 = 32;

const PACKED_OFF_MASK: usize = (1usize << PACKED_SEG_SHIFT) - 1;

/// Segment numbers fit in the 16 bits between the offset and `PACKED_BASE`.
const PACKED_MAX_SEGS: usize = 1 << (48 - PACKED_SEG_SHIFT);

#[inline]
pub(crate) fn is_packed_id(id: usize) -> bool {
    id >= PACKED_BASE
}

pub(super) fn packed_mem_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("XEZIM_PACKED_MEM")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// One admitted array.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct PackedSeg {
    /// Byte offset of cell 0 in `PackedMem::data`.
    base: usize,
    /// Number of cells.
    count: usize,
    /// Element-width mask of the value plane.
    vmask: u64,
    /// Mask of the x/z plane: `vmask`, or 0 for a two-state element (which
    /// stores no x/z plane; its "x/z" load reads a neighbour and is masked
    /// to nothing).
    pub(super) xmask: u64,
    /// The bytes of one plane (`plane` bytes of ones), for the 8-byte
    /// read-modify-write stores.
    pmask: u64,
    /// Bytes per plane: 1, 2, 4 or 8.
    plane: u8,
    /// Bytes per cell: `plane` (two-state) or `2 * plane` (value + x/z).
    cell: u8,
    /// Declared element width in bits (1..=64).
    pub(super) width: u8,
    /// `SEG_SIGNED` / `SEG_TWO_STATE` / `SEG_FORCED`.
    flags: u8,
}

const SEG_SIGNED: u8 = 1;

const SEG_TWO_STATE: u8 = 2;

/// At least one cell of the segment is under `force` (§10.6.2); stores then
/// consult `PackedMem::forced` before landing.
const SEG_FORCED: u8 = 4;

/// Zero bytes after every segment, so an 8-byte load or read-modify-write
/// store at any plane start stays inside `data`.
pub(super) const PACKED_PAD: usize = 16;

/// Byte arena backing every admitted array (see `PACKED_BASE`).
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct PackedMem {
    data: Vec<u8>,
    segs: Vec<PackedSeg>,
    /// Cells currently under `force` (only consulted for a segment carrying
    /// `SEG_FORCED`).
    forced: HashSet<usize>,
}

impl PackedMem {
    /// Admit an array of `count` cells of `width` bits, every cell holding
    /// `init`. Returns the id of cell 0, or None when the arena cannot take
    /// it (width 0 or over 64, or out of segment numbers).
    pub(super) fn admit(
        &mut self,
        count: usize,
        width: u32,
        signed: bool,
        two_state: bool,
        init: (u64, u64),
    ) -> Option<usize> {
        if width == 0 || width > 64 || count == 0 || self.segs.len() >= PACKED_MAX_SEGS {
            return None;
        }
        let plane: u8 = match width {
            1..=8 => 1,
            9..=16 => 2,
            17..=32 => 4,
            _ => 8,
        };
        let cell = if two_state { plane } else { 2 * plane };
        let vmask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let seg = PackedSeg {
            base: self.data.len(),
            count,
            vmask,
            xmask: if two_state { 0 } else { vmask },
            pmask: if plane == 8 {
                u64::MAX
            } else {
                (1u64 << (8 * plane as u32)) - 1
            },
            plane,
            cell,
            width: width as u8,
            flags: if signed { SEG_SIGNED } else { 0 } | if two_state { SEG_TWO_STATE } else { 0 },
        };
        let (iv, ix) = (init.0 & seg.vmask, init.1 & seg.xmask);
        let bytes = count.checked_mul(cell as usize)?;
        if iv == 0 && ix == 0 {
            self.data.resize(self.data.len() + bytes, 0);
        } else {
            let mut pattern = [0u8; 16];
            pattern[..plane as usize].copy_from_slice(&iv.to_le_bytes()[..plane as usize]);
            if !two_state {
                pattern[plane as usize..cell as usize]
                    .copy_from_slice(&ix.to_le_bytes()[..plane as usize]);
            }
            self.data.reserve(bytes + PACKED_PAD);
            for _ in 0..count {
                self.data.extend_from_slice(&pattern[..cell as usize]);
            }
        }
        self.data.resize(self.data.len() + PACKED_PAD, 0);
        let first = PACKED_BASE | (self.segs.len() << PACKED_SEG_SHIFT);
        self.segs.push(seg);
        Some(first)
    }

    pub(super) fn reserve(&mut self, bytes: usize) {
        self.data.reserve(bytes);
    }

    /// Bytes held by the arena.
    pub(super) fn bytes(&self) -> usize {
        self.data.len()
    }

    /// The cell bytes (for huge-page advice).
    pub(super) fn data(&self) -> &[u8] {
        &self.data
    }

    #[inline(always)]
    pub(super) fn locate(&self, id: usize) -> Option<(&PackedSeg, usize)> {
        let seg = self.segs.get((id - PACKED_BASE) >> PACKED_SEG_SHIFT)?;
        let off = id & PACKED_OFF_MASK;
        if off >= seg.count {
            return None;
        }
        Some((seg, seg.base + off * seg.cell as usize))
    }

    #[inline(always)]
    fn load8(&self, at: usize) -> u64 {
        u64::from_le_bytes(self.data[at..at + 8].try_into().unwrap())
    }

    /// Raw cell contents — `(val_bits, xz_bits, width)` with NO `Value`
    /// constructed. The RAM hot path moves whole words between the arena and
    /// a VM register; building a `Value` per access showed up as ~4% in
    /// `cell_read` plus ~3.5% in the allocator on a c906 cmark profile.
    #[inline]
    pub(crate) fn raw(&self, id: usize) -> (u64, u64, u32) {
        match self.locate(id) {
            Some((seg, at)) => (
                self.load8(at) & seg.vmask,
                self.load8(at + seg.plane as usize) & seg.xmask,
                seg.width as u32,
            ),
            None => (0, 1, 1),
        }
    }

    /// `raw` plus the element's declared signedness.
    #[inline]
    pub(crate) fn raw_signed(&self, id: usize) -> (u64, u64, u32, bool) {
        match self.locate(id) {
            Some((seg, at)) => (
                self.load8(at) & seg.vmask,
                self.load8(at + seg.plane as usize) & seg.xmask,
                seg.width as u32,
                seg.flags & SEG_SIGNED != 0,
            ),
            None => (0, 1, 1, false),
        }
    }

    /// Store raw bits into a cell (masked to the element width; a two-state
    /// element drops x/z to 0). A cell under `force` keeps its forced value.
    /// True when the cell changed.
    #[inline]
    pub(crate) fn set_raw(&mut self, id: usize, v: u64, x: u64) -> bool {
        let Some((seg, at)) = self.locate(id) else {
            return false;
        };
        let seg = *seg;
        if seg.flags & SEG_FORCED != 0 && self.forced.contains(&id) {
            return false;
        }
        self.store_at(&seg, at, v, x)
    }

    #[inline(always)]
    fn store_at(&mut self, seg: &PackedSeg, at: usize, v: u64, x: u64) -> bool {
        let nv = v & seg.vmask;
        let nx = x & seg.xmask;
        let xat = at + seg.plane as usize;
        let ov = self.load8(at);
        let ox = self.load8(xat);
        if ov & seg.vmask == nv && ox & seg.xmask == nx {
            return false;
        }
        // Read-modify-write of whole 8-byte words: the bytes past this
        // plane are written back unchanged, so `ox` stays current.
        self.data[at..at + 8].copy_from_slice(&((ov & !seg.pmask) | nv).to_le_bytes());
        if seg.xmask != 0 {
            self.data[xat..xat + 8].copy_from_slice(&((ox & !seg.pmask) | nx).to_le_bytes());
        }
        true
    }

    /// Land `(v, x)` in the cell even if it is forced, leaving the force as
    /// it is: for a swap undone before anything else reads the cell (a
    /// concurrent assertion's sampled values, §16.5.1).
    pub(super) fn put_raw(&mut self, id: usize, v: u64, x: u64) {
        if let Some((seg, at)) = self.locate(id) {
            let seg = *seg;
            self.store_at(&seg, at, v, x);
        }
    }

    /// §10.6.2 `force`: land `(v, x)` in the cell even if it is already
    /// forced, and pin it there until `release_forced`.
    pub(crate) fn force_raw(&mut self, id: usize, v: u64, x: u64) -> bool {
        let Some((seg, at)) = self.locate(id) else {
            return false;
        };
        let seg_idx = (id - PACKED_BASE) >> PACKED_SEG_SHIFT;
        let seg_copy = *seg;
        let changed = self.store_at(&seg_copy, at, v, x);
        self.segs[seg_idx].flags |= SEG_FORCED;
        self.forced.insert(id);
        changed
    }

    /// §10.6.2 `release`: the cell keeps its value (a variable) and takes
    /// ordinary stores again.
    pub(crate) fn release_forced(&mut self, id: usize) {
        self.forced.remove(&id);
    }

    #[inline]
    pub(crate) fn width(&self, id: usize) -> u32 {
        self.locate(id)
            .map(|(seg, _)| seg.width as u32)
            .unwrap_or(1)
    }
    #[inline]
    pub(crate) fn is_signed(&self, id: usize) -> bool {
        self.locate(id)
            .is_some_and(|(seg, _)| seg.flags & SEG_SIGNED != 0)
    }
    #[inline]
    pub(crate) fn read(&self, id: usize) -> Value {
        if self.locate(id).is_none() {
            return Value::new(1);
        }
        let (v, x, w, signed) = self.raw_signed(id);
        let mut value = Value::from_inline(v, x, w);
        value.is_signed = signed;
        value
    }
    /// Returns true when the cell actually changed.
    #[inline]
    pub(crate) fn write(&mut self, id: usize, val: &Value) -> bool {
        let w = self.width(id);
        let (v, x) = if val.width == w && !val.is_real && !val.is_fill {
            val.raw_bits()
        } else {
            val.resize(w).raw_bits()
        };
        self.set_raw(id, v, x)
    }
}

/// Bits `lo..=hi` of `dst` replaced by the low bits of `src` (a part-select
/// store into a packed-arena cell; `hi` < 64, empty when `lo > hi`).
#[inline(always)]
pub(super) fn splice_bits(dst: u64, src: u64, lo: u32, hi: u32) -> u64 {
    if lo > hi {
        return dst;
    }
    let len = hi - lo + 1;
    let m = if len >= 64 {
        u64::MAX
    } else {
        ((1u64 << len) - 1) << lo
    };
    (dst & !m) | ((src << lo) & m)
}
