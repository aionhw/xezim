//! Structure of Arrays (SOA) optimization for signal table.
//!
//! This module provides an optional Structure of Arrays representation for the
//! signal table. The simulator does not yet use these types; it continues to
//! store signals in `Vec<Value>` and its existing inline-bit cache.
//!
//! # Problem Background (Observation #2)
//!
//! The legacy simulator maintains both Array of Structures (`signal_table: Vec<Value>`)
//! and partial SoA layouts (`signal_inline_bits: Vec<[u64; 2]>`). This dual-view setup
//! requires synchronization between layouts. This module supports experiments
//! with a unified store; performance measurements here do not imply a simulator speedup.
//!
//! # Architecture & Incremental Migration Strategy
//!
//! 1. **Primary Storage for <= 64-bit signals**: [`InlineSignalArray`] stores `val_bits`,
//!    `xz_bits`, `widths`, and `flags` as contiguous parallel vectors. This gives:
//!    - Zero heap allocations per signal access
//!    - Ideal cache line locality (accessing only value bits during 2-state settle passes)
//!    - Direct SIMD-friendly vector layouts
//!
//! 2. **Packed Wide Signal Storage (> 64-bit)**: [`PackedWideSignalArray`] stores wide signals
//!    packed at 2 bits per logical bit (4 logical bits per byte). The core's current
//!    `Value` also uses two packed planes for wide values, so this does not provide
//!    a fourfold reduction over the current representation.
//!
//! 3. **Unified Storage & O(1) Router**: [`SoaSignalStore`] provides $O(1)$ signal categorization
//!    and lookup using a dense index LUT (`wide_map`), eliminating linear scans.
//!
//! 4. **Transient View Semantics**: Full `Value` objects are created on demand as transient
//!    views via [`SoaSignalStore::read_transient_value`] and [`SoaSignalStore::with_transient_value`].
//!    This allows legacy paths expecting `Value` to function without permanently storing
//!    duplicate AoS structures in memory.
//!
//! 5. **Coherence & Change Tracking**: In-place writes (`write_bits`, `write_value`) return boolean
//!    change flags, allowing delta-cycle worklists to skip dirty propagation when writes produce
//!    identical bit patterns. Selective synchronization (`sync_dirty_to_aos`) minimizes cross-model
//!    traffic during incremental migration.

use xezim_core::value::LogicBit;
use xezim_core::value::Value;

/// Flag indicating the signal is signed (IEEE 1800 §6.11).
pub const FLAG_SIGNED: u8 = 1 << 0;
/// Flag indicating the signal is two-state (e.g. `bit`, `byte`, `int`), where X/Z collapse to 0.
/// This is declaration metadata, independent of the signal's current value (§6.11.2).
pub const FLAG_TWO_STATE: u8 = 1 << 1;
/// Flag indicating the signal represents a floating-point real type.
pub const FLAG_REAL: u8 = 1 << 2;

/// Sentinel index in `wide_map` indicating an inline (<= 64-bit) signal.
pub const INLINE_SENTINEL: u32 = u32::MAX;

/// Structure of Arrays representation for inline (<= 64 bit) signals.
#[derive(Debug, Clone, Default)]
pub struct InlineSignalArray {
    /// Value bits for each signal (LSB = bit 0).
    pub val_bits: Vec<u64>,
    /// X/Z plane bits for each signal (1 indicates X or Z at that bit position).
    pub xz_bits: Vec<u64>,
    /// Declared bit width of each signal (1..=64).
    pub widths: Vec<u32>,
    /// Type and behavior flags (`FLAG_SIGNED`, `FLAG_TWO_STATE`, `FLAG_REAL`).
    pub flags: Vec<u8>,
    /// Number of signals stored.
    pub len: usize,
}

impl InlineSignalArray {
    /// Create a new empty `InlineSignalArray`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an `InlineSignalArray` with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            val_bits: Vec::with_capacity(capacity),
            xz_bits: Vec::with_capacity(capacity),
            widths: Vec::with_capacity(capacity),
            flags: Vec::with_capacity(capacity),
            len: 0,
        }
    }

    /// Construct from a slice of `Value` objects (recording inline signals).
    /// Values do not carry two-state declaration metadata, so signals default to four-state.
    pub fn from_values(values: &[Value]) -> Self {
        let mut arr = Self::with_capacity(values.len());
        for v in values {
            let width = v.width;
            let mut f = 0u8;
            if v.is_signed {
                f |= FLAG_SIGNED;
            }
            if v.is_real {
                f |= FLAG_REAL;
            }

            if width <= 64 {
                let (vb, xb) = v.raw_bits();
                let mask = if width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                arr.val_bits.push(vb & mask);
                arr.xz_bits.push(xb & mask);
                arr.widths.push(width);
                arr.flags.push(f);
            } else {
                // Wide fallback placeholder in inline plane
                arr.val_bits.push(0);
                arr.xz_bits.push(0);
                arr.widths.push(width);
                arr.flags.push(f);
            }
        }
        arr.len = values.len();
        arr
    }

    /// Number of signals.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the array is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get value and X/Z bits for signal at `idx`.
    #[inline(always)]
    pub fn get_bits(&self, idx: usize) -> (u64, u64) {
        (self.val_bits[idx], self.xz_bits[idx])
    }

    /// Set value and X/Z bits unconditionally.
    #[inline(always)]
    pub fn set_bits(&mut self, idx: usize, val_bits: u64, xz_bits: u64) {
        self.val_bits[idx] = val_bits;
        self.xz_bits[idx] = xz_bits;
    }

    /// Write bits with width masking, two-state folding, and change detection.
    /// Returns `true` if the signal value actually changed.
    #[inline]
    pub fn write_bits(&mut self, idx: usize, val: u64, xz: u64) -> bool {
        let mask = self.mask_of(idx);
        let flags = self.flags[idx];
        // §6.11.2: both X (v=0,x=1) and Z (v=1,x=1) convert to zero.
        let (new_val, new_xz) = if (flags & FLAG_TWO_STATE) != 0 {
            (val & !xz & mask, 0)
        } else {
            (val & mask, xz & mask)
        };

        if self.val_bits[idx] == new_val && self.xz_bits[idx] == new_xz {
            return false;
        }

        self.val_bits[idx] = new_val;
        self.xz_bits[idx] = new_xz;
        true
    }

    /// Write from a `Value` reference. Returns `true` if changed.
    #[inline]
    pub fn write_transient_value(&mut self, idx: usize, val: &Value) -> bool {
        let (vb, xb) = val.raw_bits();
        self.write_bits(idx, vb, xb)
    }

    /// Reconstruct a transient `Value` view from SoA storage on demand.
    #[inline]
    pub fn to_transient_value(&self, idx: usize) -> Value {
        let width = self.widths[idx];
        let val = self.val_bits[idx];
        let xz = self.xz_bits[idx];
        let mut v = Value::from_inline(val, xz, width);
        v.is_signed = self.is_signed(idx);
        v.is_real = self.is_real(idx);
        v
    }

    /// Execute a closure borrowing a transient `Value` view.
    #[inline]
    pub fn with_transient_value<R>(&self, idx: usize, f: impl FnOnce(&Value) -> R) -> R {
        let v = self.to_transient_value(idx);
        f(&v)
    }

    /// Width of signal at `idx`.
    #[inline(always)]
    pub fn get_width(&self, idx: usize) -> u32 {
        self.widths[idx]
    }

    /// Bitmask for signal width.
    #[inline(always)]
    pub fn mask_of(&self, idx: usize) -> u64 {
        let width = self.widths[idx];
        if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        }
    }

    /// Mask value to the signal's declared width.
    #[inline(always)]
    pub fn mask_val(&self, idx: usize, val: u64) -> u64 {
        val & self.mask_of(idx)
    }

    /// Check if signal has X or Z bits set.
    #[inline(always)]
    pub fn has_xz(&self, idx: usize) -> bool {
        self.xz_bits[idx] != 0
    }

    /// Whether signal is declared signed.
    #[inline(always)]
    pub fn is_signed(&self, idx: usize) -> bool {
        (self.flags[idx] & FLAG_SIGNED) != 0
    }

    /// Whether signal is a two-state type.
    #[inline(always)]
    pub fn is_two_state(&self, idx: usize) -> bool {
        (self.flags[idx] & FLAG_TWO_STATE) != 0
    }

    /// Whether signal is a floating-point real type.
    #[inline(always)]
    pub fn is_real(&self, idx: usize) -> bool {
        (self.flags[idx] & FLAG_REAL) != 0
    }

    /// Extract a bit from a signal: returns 0=0, 1=1, 2=x, 3=z.
    #[inline]
    pub fn get_bit(&self, idx: usize, bit_pos: u8) -> u8 {
        if bit_pos >= 64 || bit_pos >= self.widths[idx] as u8 {
            return 0;
        }
        let val = self.val_bits[idx];
        let xz = self.xz_bits[idx];
        let v = ((val >> bit_pos) & 1) as u8;
        let z = ((xz >> bit_pos) & 1) as u8;
        (z << 1) | v
    }
}

/// Conversion helper for converting between `LogicBit` and 2-bit packed codes.
trait PackedBitConvert {
    fn to_packed(&self) -> u8;
    fn from_packed(packed: u8) -> Self;
}

impl PackedBitConvert for LogicBit {
    #[inline(always)]
    fn to_packed(&self) -> u8 {
        match self {
            Self::Zero => 0b00,
            Self::One => 0b01,
            Self::X => 0b10,
            Self::Z => 0b11,
        }
    }

    #[inline(always)]
    fn from_packed(packed: u8) -> Self {
        match packed & 0b11 {
            0b00 => Self::Zero,
            0b01 => Self::One,
            0b10 => Self::X,
            _ => Self::Z,
        }
    }
}

/// Convert slice of `LogicBit` into 2-bit-per-bit packed bytes.
pub fn logic_bits_to_packed(bits: &[LogicBit]) -> Vec<u8> {
    let num_bytes = (bits.len() + 3) / 4;
    let mut packed = vec![0u8; num_bytes];
    for (i, &bit) in bits.iter().enumerate() {
        let byte_idx = i / 4;
        let shift = (i % 4) * 2;
        let code = bit.to_packed();
        packed[byte_idx] |= code << shift;
    }
    packed
}

/// Unpack 2-bit-per-bit packed bytes back into `Vec<LogicBit>`.
pub fn packed_to_logic_bits(packed: &[u8], len: usize) -> Vec<LogicBit> {
    let mut bits = Vec::with_capacity(len);
    for i in 0..len {
        let byte_idx = i / 4;
        let shift = (i % 4) * 2;
        let code = (packed[byte_idx] >> shift) & 0b11;
        bits.push(LogicBit::from_packed(code));
    }
    bits
}

/// Decode the low 64 bits from 2-bit-per-bit packed bytes in $O(1)$ time.
#[inline]
pub fn decode_packed_low_bits(data: &[u8], width: u32) -> (u64, u64) {
    let bits_to_read = width.min(64) as usize;
    let mut val: u64 = 0;
    let mut xz: u64 = 0;
    let bytes_to_read = (bits_to_read + 3) / 4;
    for (b_idx, &byte) in data.iter().take(bytes_to_read).enumerate() {
        for bit_in_byte in 0..4 {
            let bit_idx = b_idx * 4 + bit_in_byte;
            if bit_idx >= bits_to_read {
                break;
            }
            let shift = bit_in_byte * 2;
            let code = (byte >> shift) & 0b11;
            let v = (code & 1) as u64;
            let x = ((code >> 1) & 1) as u64;
            val |= v << bit_idx;
            xz |= x << bit_idx;
        }
    }
    (val, xz)
}

/// Structure of Arrays representation for packed wide signals (> 64 bits).
#[derive(Debug, Clone, Default)]
pub struct PackedWideSignalArray {
    /// 2-bit packed logical bits for each wide signal.
    pub data: Vec<Vec<u8>>,
    /// Width of each wide signal in bits.
    pub widths: Vec<u32>,
    /// Flags (`FLAG_SIGNED`, `FLAG_TWO_STATE`, `FLAG_REAL`).
    pub flags: Vec<u8>,
    /// Number of wide signals stored.
    pub len: usize,
}

impl PackedWideSignalArray {
    /// Create a new empty `PackedWideSignalArray`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from a slice of wide `Value` instances (> 64 bits).
    /// Signals default to four-state; two-state status requires declaration metadata.
    pub fn from_wide_values(values: &[Value]) -> Self {
        let mut data = Vec::with_capacity(values.len());
        let mut widths = Vec::with_capacity(values.len());
        let mut flags = Vec::with_capacity(values.len());

        for v in values {
            if v.width > 64 {
                let w = v.width as usize;
                let mut bits = Vec::with_capacity(w);
                for i in 0..w {
                    bits.push(v.get_bit(i));
                }
                data.push(logic_bits_to_packed(&bits));
                widths.push(v.width);
                let mut f = 0u8;
                if v.is_signed {
                    f |= FLAG_SIGNED;
                }
                if v.is_real {
                    f |= FLAG_REAL;
                }
                flags.push(f);
            }
        }

        let len = data.len();
        Self {
            data,
            widths,
            flags,
            len,
        }
    }

    /// Number of wide signals stored.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Extract low 64 bits of value and X/Z in $O(1)$.
    #[inline]
    pub fn get_low_bits(&self, wide_idx: usize) -> (u64, u64) {
        if wide_idx >= self.len {
            return (0, 0);
        }
        decode_packed_low_bits(&self.data[wide_idx], self.widths[wide_idx])
    }

    /// Reconstruct a full transient `Value` from packed wide storage.
    pub fn to_transient_value(&self, wide_idx: usize) -> Value {
        let width = self.widths[wide_idx];
        let bits = packed_to_logic_bits(&self.data[wide_idx], width as usize);
        let mut val = Value::new(width);
        val.is_signed = (self.flags[wide_idx] & FLAG_SIGNED) != 0;
        val.is_real = (self.flags[wide_idx] & FLAG_REAL) != 0;
        for (i, &b) in bits.iter().enumerate() {
            val.set_bit(i, b);
        }
        val
    }

    /// Write a new `Value` into packed storage. Returns `true` if changed.
    pub fn write_wide_value(&mut self, wide_idx: usize, val: &Value) -> bool {
        let w = self.widths[wide_idx] as usize;
        let mut bits = Vec::with_capacity(w);
        for i in 0..w {
            bits.push(val.get_bit(i));
        }
        let mut new_bytes = logic_bits_to_packed(&bits);
        if (self.flags[wide_idx] & FLAG_TWO_STATE) != 0 {
            // §6.11.2: retain only known ones in each packed pair (X/Z -> 0).
            for byte in &mut new_bytes {
                *byte = (*byte & !(*byte >> 1)) & 0x55;
            }
        }
        if self.data[wide_idx] == new_bytes {
            return false;
        }
        self.data[wide_idx] = new_bytes;
        true
    }

    /// Check if wide signal has X or Z bits set.
    /// Fast check: each packed byte stores four 2-bit codes; if any code's
    /// high bit is 1 (`0b10` = X, `0b11` = Z), `byte & 0b10101010 != 0`.
    #[inline]
    pub fn has_xz(&self, wide_idx: usize) -> bool {
        self.data[wide_idx].iter().any(|&b| (b & 0xAA) != 0)
    }
}

/// Unified Structure of Arrays signal store with $O(1)$ routing.
///
/// Makes SoA the primary store for <= 64-bit signals, while offering seamless
/// transient `Value` views and bidirectional AoS synchronization.
#[derive(Debug, Clone)]
pub struct SoaSignalStore {
    /// Inline signals (<= 64 bits) stored as parallel arrays.
    pub inline: InlineSignalArray,
    /// Wide signals (> 64 bits) stored as packed 2-bit vectors.
    pub wide: PackedWideSignalArray,
    /// $O(1)$ lookup table: `wide_map[signal_id]`.
    /// Equals `INLINE_SENTINEL` if signal is inline, otherwise index into `wide`.
    wide_map: Vec<u32>,
    /// Total number of signals in the design.
    pub total_len: usize,
    /// Monotonically increasing generation counter bumped on modifications.
    pub generation: u64,
}

impl SoaSignalStore {
    /// Construct a new `SoaSignalStore` from a design's initial `Vec<Value>`.
    /// All signals default to four-state because `Value` does not encode that type property.
    pub fn from_values(values: &[Value]) -> Self {
        let inline = InlineSignalArray::from_values(values);

        let mut wide_values = Vec::new();
        let mut wide_map = Vec::with_capacity(values.len());

        for v in values {
            if v.width > 64 {
                let w_idx = wide_values.len() as u32;
                wide_values.push(v.clone());
                wide_map.push(w_idx);
            } else {
                wide_map.push(INLINE_SENTINEL);
            }
        }

        let wide = PackedWideSignalArray::from_wide_values(&wide_values);
        let total_len = values.len();

        Self {
            inline,
            wide,
            wide_map,
            total_len,
            generation: 1,
        }
    }

    /// Construct with two-state declaration metadata indexed by signal ID.
    /// Missing entries default to four-state; entries beyond `values` are ignored.
    /// Initial X/Z bits of two-state signals are converted to zero (§6.11.2).
    pub fn from_values_with_two_state(values: &[Value], two_state: &[bool]) -> Self {
        let mut store = Self::from_values(values);
        for (id, &is_two_state) in two_state.iter().take(values.len()).enumerate() {
            if is_two_state {
                store.inline.flags[id] |= FLAG_TWO_STATE;
                let wide_idx = store.wide_map[id];
                if wide_idx != INLINE_SENTINEL {
                    store.wide.flags[wide_idx as usize] |= FLAG_TWO_STATE;
                }
                store.write_value(id, &values[id]);
            }
        }
        // Initial type conversion is part of construction, not a subsequent write.
        store.generation = 1;
        store
    }

    /// Number of signals in design.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.total_len
    }

    /// Whether store is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    /// Check in $O(1)$ whether signal `id` is a wide signal (> 64 bits).
    #[inline(always)]
    pub fn is_wide(&self, id: usize) -> bool {
        self.wide_map.get(id).copied().unwrap_or(INLINE_SENTINEL) != INLINE_SENTINEL
    }

    /// Get value and X/Z bits as `(val_bits, xz_bits)`.
    /// For inline signals, returns exact bits. For wide signals, returns low 64 bits in $O(1)$.
    #[inline]
    pub fn get_inline_bits(&self, id: usize) -> (u64, u64) {
        let w_idx = self.wide_map.get(id).copied().unwrap_or(INLINE_SENTINEL);
        if w_idx != INLINE_SENTINEL {
            self.wide.get_low_bits(w_idx as usize)
        } else {
            self.inline.get_bits(id)
        }
    }

    /// Check if signal has any X or Z bits in $O(1)$.
    #[inline]
    pub fn has_xz(&self, id: usize) -> bool {
        let w_idx = self.wide_map.get(id).copied().unwrap_or(INLINE_SENTINEL);
        if w_idx != INLINE_SENTINEL {
            self.wide.has_xz(w_idx as usize)
        } else {
            self.inline.has_xz(id)
        }
    }

    /// Get width of signal in $O(1)$.
    #[inline(always)]
    pub fn get_width(&self, id: usize) -> u32 {
        self.inline.widths[id]
    }

    /// Check if signal is declared signed in $O(1)$.
    #[inline(always)]
    pub fn is_signed(&self, id: usize) -> bool {
        self.inline.is_signed(id)
    }

    /// Write bits directly to inline signal. Returns `true` if value changed.
    #[inline]
    pub fn write_bits(&mut self, id: usize, val: u64, xz: u64) -> bool {
        let changed = self.inline.write_bits(id, val, xz);
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        changed
    }

    /// Write any `Value` (inline or wide) into SoA storage. Returns `true` if changed.
    pub fn write_value(&mut self, id: usize, val: &Value) -> bool {
        let w_idx = self.wide_map.get(id).copied().unwrap_or(INLINE_SENTINEL);
        let changed = if w_idx != INLINE_SENTINEL {
            self.wide.write_wide_value(w_idx as usize, val)
        } else {
            self.inline.write_transient_value(id, val)
        };
        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        changed
    }

    /// Reconstruct a transient `Value` on demand from SoA storage.
    pub fn read_transient_value(&self, id: usize) -> Value {
        let w_idx = self.wide_map.get(id).copied().unwrap_or(INLINE_SENTINEL);
        if w_idx != INLINE_SENTINEL {
            self.wide.to_transient_value(w_idx as usize)
        } else {
            self.inline.to_transient_value(id)
        }
    }

    /// Execute a closure with a borrowed transient `Value`.
    pub fn with_transient_value<R>(&self, id: usize, f: impl FnOnce(&Value) -> R) -> R {
        let v = self.read_transient_value(id);
        f(&v)
    }

    // ── Incremental Migration & Synchronization Layer ──────────────────

    /// Bulk synchronize the entire SoA store from a legacy `Vec<Value>` AoS table.
    pub fn sync_from_aos(&mut self, aos: &[Value]) {
        for (id, val) in aos.iter().enumerate().take(self.total_len) {
            self.write_value(id, val);
        }
    }

    /// Flush the SoA store state into an external legacy AoS table.
    pub fn sync_to_aos(&self, aos: &mut [Value]) {
        for id in 0..self.total_len.min(aos.len()) {
            aos[id] = self.read_transient_value(id);
        }
    }

    /// Selectively synchronize only modified signals back into an AoS table.
    /// Minimizes cache traffic and allocator overhead during hybrid execution.
    pub fn sync_dirty_to_aos(&self, dirty_ids: &[usize], aos: &mut [Value]) {
        for &id in dirty_ids {
            if id < self.total_len && id < aos.len() {
                aos[id] = self.read_transient_value(id);
            }
        }
    }

    /// Synchronize a single signal updated by legacy AoS code into SoA store.
    pub fn sync_signal_from_aos(&mut self, id: usize, val: &Value) -> bool {
        self.write_value(id, val)
    }
}

/// Backward-compatible wrapper preserving the `SoaSignalTable` API.
#[derive(Debug, Clone)]
pub struct SoaSignalTable {
    /// Internal unified store.
    pub store: SoaSignalStore,
    /// Indices of wide signals in the original signal table (for backward compatibility).
    pub wide_indices: Vec<usize>,
    /// Total number of signals.
    pub total_len: usize,
}

impl SoaSignalTable {
    /// Create a new `SoaSignalTable` from a slice of `Value`s.
    pub fn from_values(values: &[Value]) -> Self {
        let mut wide_indices = Vec::new();
        for (idx, v) in values.iter().enumerate() {
            if v.width > 64 {
                wide_indices.push(idx);
            }
        }
        let total_len = values.len();
        let store = SoaSignalStore::from_values(values);

        Self {
            store,
            wide_indices,
            total_len,
        }
    }

    /// Get inline bits in $O(1)$ without linear scanning.
    #[inline(always)]
    pub fn get_inline_bits(&self, idx: usize) -> (u64, u64) {
        self.store.get_inline_bits(idx)
    }

    /// Number of signals.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.total_len
    }

    /// Whether empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }
}

// ── Tests & Benchmarks ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Reference probe: widths 1, 8, 63, 64, 65, 128 and 129; all 0, all 1,
    // all X, all Z, and repeating ZX10. §6.11.2 preserves X/Z in logic and
    // converts them to 0 in bit, leaving known ones unchanged.
    fn reference_pattern(width: u32, pattern: usize, two_state: bool) -> Value {
        let mut value = Value::from_u64(0, width);
        for bit_index in 0..width as usize {
            let bit = match pattern {
                0 => LogicBit::Zero,
                1 => LogicBit::One,
                2 => LogicBit::X,
                3 => LogicBit::Z,
                _ => [LogicBit::Zero, LogicBit::One, LogicBit::X, LogicBit::Z][bit_index % 4],
            };
            value.set_bit(
                bit_index,
                if two_state && matches!(bit, LogicBit::X | LogicBit::Z) {
                    LogicBit::Zero
                } else {
                    bit
                },
            );
        }
        value
    }

    /// §6.11.2: a known initial value does not make a logic signal two-state.
    #[test]
    fn test_known_four_state_signals_accept_xz() {
        for width in [1, 8, 63, 64, 65, 128, 129] {
            let initial = reference_pattern(width, 0, false);
            let mut store = SoaSignalStore::from_values(&[initial.clone()]);
            assert!(!store.inline.is_two_state(0));
            if width > 64 {
                assert_eq!(store.wide.flags[0] & FLAG_TWO_STATE, 0);
            }
            let mut aos = vec![initial];
            for pattern in 1..5 {
                let value = reference_pattern(width, pattern, false);
                let previous = store.read_transient_value(0);
                let generation = store.generation;
                let changed = value != previous;
                assert_eq!(store.write_value(0, &value), changed);
                assert_eq!(store.generation, generation + u64::from(changed));
                assert_eq!(store.read_transient_value(0), value);
                assert_eq!(store.get_inline_bits(0), value.raw_bits());
                assert_eq!(store.has_xz(0), value.has_xz());
                assert!(!store.write_value(0, &value));
                store.sync_dirty_to_aos(&[0], &mut aos);
                assert_eq!(aos[0], value);
            }
        }
    }

    /// §6.11.2: both X and Z convert to zero, for inline and wide bit vectors.
    #[test]
    fn test_two_state_writes_fold_x_and_z() {
        for width in [1, 8, 63, 64, 65, 128, 129] {
            let initial = reference_pattern(width, 0, false);
            let mut store = SoaSignalStore::from_values_with_two_state(&[initial], &[true]);
            assert!(store.inline.is_two_state(0));
            if width > 64 {
                assert_ne!(store.wide.flags[0] & FLAG_TWO_STATE, 0);
            }
            for pattern in 0..5 {
                let source = reference_pattern(width, pattern, false);
                let expected = reference_pattern(width, pattern, true);
                let changed = expected != store.read_transient_value(0);
                let generation = store.generation;
                assert_eq!(store.write_value(0, &source), changed);
                assert_eq!(store.generation, generation + u64::from(changed));
                assert_eq!(store.read_transient_value(0), expected);
                assert!(!store.has_xz(0));
                assert!(!store.write_value(0, &source));
                if width <= 64 {
                    let (val, xz) = source.raw_bits();
                    assert!(!store.write_bits(0, val, xz));
                    assert_eq!(store.get_inline_bits(0), expected.raw_bits());
                }
            }
        }
    }

    /// §6.11.2: declaration metadata controls initial conversion and AoS sync.
    #[test]
    fn test_two_state_metadata_and_aos_sync() {
        let values: Vec<_> = [1, 8, 63, 64, 65, 128, 129]
            .into_iter()
            .map(|width| reference_pattern(width, 4, false))
            .collect();
        let metadata = [true, false, true, false, true, false, true];
        let mut store = SoaSignalStore::from_values_with_two_state(&values, &metadata);
        let expected: Vec<_> = values
            .iter()
            .zip(metadata)
            .map(|(value, two_state)| reference_pattern(value.width, 4, two_state))
            .collect();
        let mut aos = values.clone();
        store.sync_to_aos(&mut aos);
        assert_eq!(aos, expected);
        assert_eq!(store.generation, 1);
        store.sync_from_aos(&values);
        store.sync_to_aos(&mut aos);
        assert_eq!(aos, expected);

        // A partial declaration mask defaults the remaining signals to four-state.
        let partial = SoaSignalStore::from_values_with_two_state(&values, &[true]);
        for (id, value) in values.iter().enumerate().skip(1) {
            assert_eq!(partial.read_transient_value(id), *value);
            assert!(!partial.inline.is_two_state(id));
        }
    }

    #[test]
    fn test_packed_conversion() {
        let bits = [
            LogicBit::Zero,
            LogicBit::One,
            LogicBit::X,
            LogicBit::Z,
            LogicBit::One,
            LogicBit::Zero,
        ];
        assert_eq!(
            packed_to_logic_bits(&logic_bits_to_packed(&bits), bits.len()),
            bits
        );
    }

    #[test]
    fn test_inline_array_operations() {
        let mut values = Vec::new();
        for i in 0..100 {
            let mut v = Value::from_u64(i as u64, 16);
            if i % 2 == 1 {
                v.is_signed = true;
            }
            values.push(v);
        }

        let mut arr = InlineSignalArray::from_values(&values);
        assert_eq!(arr.len(), 100);

        // Verify values and signedness
        for i in 0..100 {
            let (val, xz) = arr.get_bits(i);
            assert_eq!(val, i as u64);
            assert_eq!(xz, 0);
            assert_eq!(arr.is_signed(i), i % 2 == 1);
            assert_eq!(arr.get_width(i), 16);
        }

        // Test in-place write with change detection
        let changed = arr.write_bits(10, 42, 0);
        assert!(changed);
        assert_eq!(arr.get_bits(10), (42, 0));

        // Unchanged write returns false
        let changed_again = arr.write_bits(10, 42, 0);
        assert!(!changed_again);

        // Test transient value view reification
        let transient = arr.to_transient_value(10);
        assert_eq!(transient.to_u64(), Some(42));
        assert_eq!(transient.width, 16);
    }

    #[test]
    fn test_wide_array_operations() {
        let width = 128;
        let mut wide_val = Value::new(width);
        wide_val.is_signed = true;
        wide_val.set_bit(0, LogicBit::One);
        wide_val.set_bit(1, LogicBit::Zero);
        wide_val.set_bit(2, LogicBit::X);
        wide_val.set_bit(3, LogicBit::Z);
        wide_val.set_bit(65, LogicBit::One);

        let wide_arr = PackedWideSignalArray::from_wide_values(&[wide_val.clone()]);
        assert_eq!(wide_arr.len(), 1);
        assert!(wide_arr.has_xz(0));

        // Low 64 bits decoding
        let (val_low, xz_low) = wide_arr.get_low_bits(0);
        assert_eq!(val_low & 0b1111, 0b1001); // bits 0=1, 1=0, 2=0, 3=1
        assert_eq!(xz_low & 0b1111, 0b1100); // bits 2=X, 3=Z

        // Transient view roundtrip
        let reconstructed = wide_arr.to_transient_value(0);
        assert_eq!(reconstructed.width, width);
        assert_eq!(reconstructed.is_signed, true);
        assert_eq!(reconstructed.get_bit(0), LogicBit::One);
        assert_eq!(reconstructed.get_bit(1), LogicBit::Zero);
        assert_eq!(reconstructed.get_bit(2), LogicBit::X);
        assert_eq!(reconstructed.get_bit(3), LogicBit::Z);
        assert_eq!(reconstructed.get_bit(65), LogicBit::One);
    }

    #[test]
    fn test_soa_signal_store_unified_and_sync() {
        let mut initial_aos = Vec::new();
        // 50 inline signals
        for i in 0..50 {
            initial_aos.push(Value::from_u64(i as u64, 32));
        }
        // 10 wide signals
        for i in 0..10 {
            let mut w = Value::new(128);
            w.set_bit(0, LogicBit::One);
            w.set_bit(i, LogicBit::One);
            initial_aos.push(w);
        }

        let mut store = SoaSignalStore::from_values(&initial_aos);
        assert_eq!(store.len(), 60);

        // O(1) wide checks
        assert!(!store.is_wide(10));
        assert!(store.is_wide(50));
        assert!(store.is_wide(59));

        // Inline bits extraction for both inline and wide
        assert_eq!(store.get_inline_bits(10), (10, 0));
        let (w_val, _) = store.get_inline_bits(50);
        assert_eq!(w_val & 1, 1);

        // In-place mutation
        let gen_before = store.generation;
        let changed = store.write_bits(5, 0xABCD, 0);
        assert!(changed);
        assert_eq!(store.get_inline_bits(5), (0xABCD, 0));
        assert!(store.generation > gen_before);

        // Selective dirty sync back to AoS
        let mut legacy_aos = initial_aos.clone();
        store.sync_dirty_to_aos(&[5], &mut legacy_aos);
        assert_eq!(legacy_aos[5].to_u64(), Some(0xABCD));
        // Other entries untouched
        assert_eq!(legacy_aos[4].to_u64(), Some(4));
    }

    #[test]
    fn test_soa_vs_aos_memory_efficiency() {
        let num_signals = 10000;
        let width = 64;

        let aos: Vec<Value> = (0..num_signals)
            .map(|_| Value::from_u64(0xAAAA_AAAA_AAAA_AAAA, width))
            .collect();

        let soa = InlineSignalArray::from_values(&aos);

        let aos_size = aos.capacity() * std::mem::size_of::<Value>();
        let soa_size = soa.val_bits.capacity() * 8
            + soa.xz_bits.capacity() * 8
            + soa.widths.capacity() * 4
            + soa.flags.capacity();

        assert!(
            soa_size < aos_size,
            "SoA memory ({soa_size} B) should be more compact than AoS ({aos_size} B)"
        );
    }
}
