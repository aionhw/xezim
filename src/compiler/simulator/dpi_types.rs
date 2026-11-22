//! DPI argument and return kinds, open arrays and bindings.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum DpiRetKind {
    Void,
    Int32,
    /// §35.5.5 / H.7.4: a scalar `logic` result is an `svLogic` — an
    /// `unsigned char` holding sv_0 / sv_1 / sv_z (2) / sv_x (3).
    Logic,
    Int64,
    Real32,
    Real64,
    Chandle,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum DpiArgKind {
    Int32In,
    Int32Out,
    /// §35.5.6.1 / H.7.4: a scalar 4-state formal (`logic`, `reg`) is an
    /// `svLogic` (`unsigned char`, sv_z = 2, sv_x = 3) by value, or an
    /// `svLogic*` for output and inout.
    LogicIn,
    LogicOut,
    /// §35.5.6.1: a scalar two-state `bit` is an `svBit` (`unsigned char`)
    /// by value, or an `svBit*` for output and inout. Unlike `int`/`byte`/
    /// `shortint` (pass-by-value i32 whose low byte the C side reads) the
    /// value must be reduced to its single meaningful bit (0/1) before
    /// crossing the boundary — a 1-bit formal is not the low 8 bits.
    BitIn,
    BitOut,
    Int64In,
    Int64Out,
    Real32In,
    Real32Out,
    Real64In,
    Real64Out,
    ChandleIn,
    ChandleOut,
    StringIn,
    StringOut,
    /// An open-array formal (`int a[]`, `bit [7:0] a[]`): passed as an
    /// Annex H open-array handle (see `DpiOpenArrayHeader`).
    OpenArrayIn(DpiOaElem),
    OpenArrayOut(DpiOaElem),
    VecLogicIn(u32),
    VecLogicOut(u32),
    /// 2-state packed vector (`bit [N]`) passed as plain `uint32_t*`
    /// (svBitVecVal*). ABI differs from VecLogicIn: no bval field.
    VecBitIn(u32),
    VecBitOut(u32),
    /// §35.5.6 / H.7.6: an unpacked struct (or a fixed-size array of
    /// them), passed as a pointer to the C struct of its members; its C
    /// layout comes from the formal's declared type (`dpi_c_type`).
    StructIn,
    StructOut,
}

/// The canonical C element of a DPI open array (IEEE 1800-2017 H.7):
/// `char`, `short`, `int`, `long long`, `float`, `double`, or a packed
/// vector as `svBitVecVal` / `svLogicVecVal` words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum DpiOaElem {
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bit(u32),
    Logic(u32),
    /// A scalar `bit` / `logic` element: one `svBit` / `svLogic` byte
    /// (H.7.4), not a one-word vector.
    SBit,
    SLogic,
}

impl DpiOaElem {
    pub(super) fn bytes(self) -> usize {
        match self {
            DpiOaElem::I8 | DpiOaElem::SBit | DpiOaElem::SLogic => 1,
            DpiOaElem::I16 => 2,
            DpiOaElem::I32 | DpiOaElem::F32 => 4,
            DpiOaElem::I64 | DpiOaElem::F64 => 8,
            DpiOaElem::Bit(w) => 4 * w.div_ceil(32).max(1) as usize,
            DpiOaElem::Logic(w) => 8 * w.div_ceil(32).max(1) as usize,
        }
    }

    /// The SV width of one element.
    pub(super) fn width(self) -> u32 {
        match self {
            DpiOaElem::I8 => 8,
            DpiOaElem::I16 => 16,
            DpiOaElem::I32 | DpiOaElem::F32 => 32,
            DpiOaElem::I64 | DpiOaElem::F64 => 64,
            DpiOaElem::Bit(w) | DpiOaElem::Logic(w) => w.max(1),
            DpiOaElem::SBit | DpiOaElem::SLogic => 1,
        }
    }
}

/// The header placed immediately BEFORE the element data of a DPI open
/// array; the handle C receives points at the data, so code indexing it as
/// a plain C array keeps working while `svSize`, `svLeft`,
/// `svGetArrElemPtr1`, ... (src/svdpi_shim.c, which mirrors this layout as
/// `xz_oa_hdr`) read the shape from here.
#[repr(C)]
#[derive(Clone, Copy)]
struct DpiOpenArrayHeader {
    magic: u32,
    elem_bytes: u32,
    ndims: i32,
    packed: i32,
    packed_left: i32,
    packed_right: i32,
    left: [i32; 4],
    right: [i32; 4],
    total_bytes: u32,
    elem_count: u32,
}

const DPI_OA_MAGIC: u32 = 0x414f_5a58; // "XZOA"

const DPI_OA_HDR_BYTES: usize = std::mem::size_of::<DpiOpenArrayHeader>();

const _: () = assert!(DPI_OA_HDR_BYTES == 64);

/// One open-array argument: its buffer (header + data, 8-byte aligned) and
/// shape, kept alive for the call and decoded again for an output.
pub(super) struct DpiOpenArrayArg {
    buf: Vec<u64>,
    elem: DpiOaElem,
    count: usize,
}

impl DpiOpenArrayArg {
    /// `shape` is each unpacked dimension's `(left, right)`, outermost
    /// first; `values` holds the elements row-major, every dimension in
    /// ascending index order (the order `svGetArrElemPtr*` addresses).
    pub(super) fn new(elem: DpiOaElem, values: &[Value], shape: &[(i64, i64)]) -> Self {
        let count = values.len();
        let mut left = [0i32; 4];
        let mut right = [0i32; 4];
        for (k, &(l, r)) in shape.iter().take(4).enumerate() {
            left[k] = l as i32;
            right[k] = r as i32;
        }
        let eb = elem.bytes();
        let data_bytes = eb * count;
        let mut buf = vec![0u64; (DPI_OA_HDR_BYTES + data_bytes).div_ceil(8)];
        let packed = matches!(elem, DpiOaElem::Bit(_) | DpiOaElem::Logic(_));
        let hdr = DpiOpenArrayHeader {
            magic: DPI_OA_MAGIC,
            elem_bytes: eb as u32,
            ndims: shape.len().clamp(1, 4) as i32,
            packed: packed as i32,
            packed_left: if packed { elem.width() as i32 - 1 } else { 0 },
            packed_right: 0,
            left,
            right,
            total_bytes: data_bytes as u32,
            elem_count: count as u32,
        };
        let bytes: &mut [u8] =
            unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
        bytes[..DPI_OA_HDR_BYTES].copy_from_slice(unsafe {
            std::slice::from_raw_parts(
                (&hdr as *const DpiOpenArrayHeader).cast::<u8>(),
                DPI_OA_HDR_BYTES,
            )
        });
        for (i, v) in values.iter().enumerate() {
            let at = DPI_OA_HDR_BYTES + i * eb;
            let dst = &mut bytes[at..at + eb];
            match elem {
                DpiOaElem::I8 | DpiOaElem::I16 | DpiOaElem::I32 | DpiOaElem::I64 => {
                    let x = if v.is_real {
                        v.to_f64() as i64 as u64
                    } else {
                        v.to_u64().unwrap_or(0)
                    };
                    dst.copy_from_slice(&x.to_le_bytes()[..eb]);
                }
                DpiOaElem::F32 => dst.copy_from_slice(&(Self::real_of(v) as f32).to_le_bytes()),
                DpiOaElem::F64 => dst.copy_from_slice(&Self::real_of(v).to_le_bytes()),
                DpiOaElem::SBit => dst[0] = (v.get_bit(0) == LogicBit::One) as u8,
                DpiOaElem::SLogic => dst[0] = Simulator::dpi_svlogic_code(v),
                DpiOaElem::Bit(w) => {
                    let (aval, _) = Simulator::dpi_value_to_logic_words(v, w);
                    for (k, word) in aval.iter().enumerate().take(eb / 4) {
                        dst[k * 4..k * 4 + 4].copy_from_slice(&word.to_le_bytes());
                    }
                }
                DpiOaElem::Logic(w) => {
                    let il = Simulator::dpi_value_to_logic_interleaved(v, w);
                    for (k, word) in il.iter().enumerate().take(eb / 4) {
                        dst[k * 4..k * 4 + 4].copy_from_slice(&word.to_le_bytes());
                    }
                }
            }
        }
        DpiOpenArrayArg { buf, elem, count }
    }

    fn real_of(v: &Value) -> f64 {
        if v.is_real {
            v.to_f64()
        } else {
            v.to_u64().unwrap_or(0) as i64 as f64
        }
    }

    /// The handle C receives: the start of the element data.
    pub(super) fn handle(&mut self) -> *mut std::ffi::c_void {
        unsafe {
            self.buf
                .as_mut_ptr()
                .cast::<u8>()
                .add(DPI_OA_HDR_BYTES)
                .cast()
        }
    }

    /// The elements as C left them.
    pub(super) fn values(&self) -> Vec<Value> {
        let eb = self.elem.bytes();
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.buf.len() * 8)
        };
        (0..self.count)
            .map(|i| {
                let at = DPI_OA_HDR_BYTES + i * eb;
                let src = &bytes[at..at + eb];
                let word = |k: usize| u32::from_le_bytes(src[k * 4..k * 4 + 4].try_into().unwrap());
                match self.elem {
                    DpiOaElem::I8 => Value::from_u64(src[0] as i8 as i64 as u64, 8),
                    DpiOaElem::I16 => {
                        Value::from_u64(i16::from_le_bytes([src[0], src[1]]) as i64 as u64, 16)
                    }
                    DpiOaElem::I32 => Value::from_u64(word(0) as u64, 32),
                    DpiOaElem::I64 => {
                        Value::from_u64(u64::from_le_bytes(src.try_into().unwrap()), 64)
                    }
                    DpiOaElem::F32 => {
                        Value::from_f64(f32::from_le_bytes(src.try_into().unwrap()) as f64)
                    }
                    DpiOaElem::F64 => Value::from_f64(f64::from_le_bytes(src.try_into().unwrap())),
                    DpiOaElem::SBit => Value::from_u64((src[0] & 1) as u64, 1),
                    DpiOaElem::SLogic => Simulator::dpi_svlogic_value(src[0]),
                    DpiOaElem::Bit(w) => {
                        let aval: Vec<u32> = (0..eb / 4).map(word).collect();
                        Simulator::dpi_logic_words_to_value(&aval, &[], w)
                    }
                    DpiOaElem::Logic(w) => {
                        let il: Vec<u32> = (0..eb / 4).map(word).collect();
                        Simulator::dpi_logic_interleaved_to_value(&il, w)
                    }
                }
            })
            .collect()
    }
}

pub(super) struct DpiBinding {
    pub(super) ret: DpiRetKind,
    pub(super) arg_kinds: Vec<DpiArgKind>,
    pub(super) cif: Cif,
    pub(super) fn_ptr: CodePtr,
}
