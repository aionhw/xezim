//! IEEE 1800 §35.5.4: exported subroutines called from C.
//!
//! A user DPI library calls an exported subroutine by its C name, an
//! undefined symbol to the library. Before the libraries load, a small C
//! trampoline library defines one function per exported name: it packs the
//! arguments into 64-bit slots (integers by value, reals by their bits,
//! everything passed by reference as the address) and forwards to
//! `__xezim_dpi_export_dispatch`, which runs the subroutine (`run_dpi_export`).
//!
//! The C side of each formal follows §35.5.6 and Annex H (H.7, H.8, H.10),
//! as the reference simulator's generated header declares it:
//! - `byte`, `shortint`, `int`, `longint`, `real`, `shortreal` by value as
//!   `char`, `short`, `int`, `int64_t`, `double`, `float`; scalar `bit` /
//!   `logic` as `svBit` / `svLogic`; `chandle` as `void*`; `string` as
//!   `const char*`;
//! - a packed vector wider than 64 bits by pointer to the canonical
//!   `svLogicVecVal[]` (4-state) or `svBitVecVal[]` (2-state). xezim passes a
//!   packed vector of up to 64 bits by value, as `int` or `long long` (where
//!   the standard passes it by pointer too);
//! - an unpacked struct or fixed-size unpacked array by pointer to its C
//!   layout: the C struct of its members, or its elements in a row;
//! - an `output` or `inout` formal by pointer to its C type.
//!
//! A function result is a small value (§35.5.5): a packed vector wider than
//! 64 bits or an aggregate is not a legal result, and the run stops with an
//! error, as in the reference simulator. Any other type DPI does not carry
//! (an `event`, a class handle, a dynamic array, ...) leaves the subroutine
//! not callable from C: the startup message says so, and a call from C
//! reports an error and returns 0 (#291).

use super::*;
use std::ffi::CString;

/// A C scalar of the export ABI (Annex H.7.3, Table H.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DpiCScalar {
    I8,
    I16,
    I32,
    I64,
    /// `svBit`: an `unsigned char` holding 0 or 1.
    Bit,
    /// `svLogic`: an `unsigned char` holding `sv_0`, `sv_1`, `sv_z`, `sv_x`.
    Logic,
    F32,
    F64,
    /// `chandle`: `void*`.
    Chandle,
    /// `string`: `const char*`.
    Str,
}

impl DpiCScalar {
    fn c_type(self) -> &'static str {
        match self {
            DpiCScalar::I8 => "char",
            DpiCScalar::I16 => "short",
            DpiCScalar::I32 => "int",
            DpiCScalar::I64 => "long long",
            DpiCScalar::Bit | DpiCScalar::Logic => "unsigned char",
            DpiCScalar::F32 => "float",
            DpiCScalar::F64 => "double",
            DpiCScalar::Chandle => "void*",
            DpiCScalar::Str => "const char*",
        }
    }

    fn size(self) -> usize {
        match self {
            DpiCScalar::I8 | DpiCScalar::Bit | DpiCScalar::Logic => 1,
            DpiCScalar::I16 => 2,
            DpiCScalar::I32 | DpiCScalar::F32 => 4,
            DpiCScalar::I64 | DpiCScalar::F64 => 8,
            DpiCScalar::Chandle | DpiCScalar::Str => std::mem::size_of::<usize>(),
        }
    }

    fn is_real(self) -> bool {
        matches!(self, DpiCScalar::F32 | DpiCScalar::F64)
    }

    /// The SV value of a 64-bit slot holding this scalar.
    fn slot_to_value(self, slot: i64) -> Value {
        match self {
            DpiCScalar::F32 | DpiCScalar::F64 => Value::from_f64(f64::from_bits(slot as u64)),
            DpiCScalar::Str => Value::from_string(&c_string_at(slot as usize)),
            DpiCScalar::Chandle => Value::from_u64(slot as u64, 64),
            DpiCScalar::Bit => Value::from_u64(slot as u64 & 1, 1),
            DpiCScalar::Logic => {
                let mut v = Value::zero(1);
                v.set_bit(
                    0,
                    match slot & 3 {
                        0 => LogicBit::Zero,
                        1 => LogicBit::One,
                        2 => LogicBit::Z,
                        _ => LogicBit::X,
                    },
                );
                v
            }
            DpiCScalar::I8 | DpiCScalar::I16 | DpiCScalar::I32 | DpiCScalar::I64 => {
                let w = (self.size() * 8) as u32;
                let mut v = Value::from_u64(slot as u64, 64).resize(w);
                v.is_signed = true;
                v
            }
        }
    }

    /// A value as this scalar's 64-bit slot. A string is kept in `keep`,
    /// which the caller holds until the next call of the export.
    fn value_to_slot(self, v: &Value, keep: &mut Vec<CString>) -> i64 {
        // §35.5.6 / §6.12.2: a real value bound for an integral C type
        // converts by value (rounding), never as its f64 bit pattern.
        let rounded;
        let v = if v.is_real && !matches!(self, DpiCScalar::F32 | DpiCScalar::F64 | DpiCScalar::Str)
        {
            rounded = Simulator::real_to_int(v.to_f64(), 64);
            &rounded
        } else {
            v
        };
        match self {
            DpiCScalar::F32 | DpiCScalar::F64 => v.to_f64().to_bits() as i64,
            DpiCScalar::Str => {
                let c = CString::new(v.to_sv_string().replace('\0', "")).unwrap_or_default();
                let p = c.as_ptr() as usize as i64;
                keep.push(c);
                p
            }
            DpiCScalar::Logic => match v.get_bit(0) {
                LogicBit::Zero => 0,
                LogicBit::One => 1,
                LogicBit::Z => 2,
                LogicBit::X => 3,
            },
            DpiCScalar::Bit => (v.get_bit(0) == LogicBit::One) as i64,
            _ => {
                let mut v = v.clone();
                v.is_real = false;
                v.to_u64().map_or(0, |u| u as i64)
            }
        }
    }

    /// Read one scalar from C memory, as a slot.
    ///
    /// # Safety
    /// `p` points at a valid, aligned object of this type.
    unsafe fn load(self, p: *const u8) -> i64 {
        unsafe {
            match self {
                DpiCScalar::I8 => *(p as *const i8) as i64,
                DpiCScalar::Bit | DpiCScalar::Logic => *p as i64,
                DpiCScalar::I16 => *(p as *const i16) as i64,
                DpiCScalar::I32 => *(p as *const i32) as i64,
                DpiCScalar::I64 => *(p as *const i64),
                DpiCScalar::F32 => (*(p as *const f32) as f64).to_bits() as i64,
                DpiCScalar::F64 => (*(p as *const f64)).to_bits() as i64,
                DpiCScalar::Chandle | DpiCScalar::Str => *(p as *const usize) as i64,
            }
        }
    }

    /// Write one scalar slot to C memory.
    ///
    /// # Safety
    /// `p` points at a valid, aligned, writable object of this type.
    unsafe fn store(self, p: *mut u8, slot: i64) {
        unsafe {
            match self {
                DpiCScalar::I8 | DpiCScalar::Bit | DpiCScalar::Logic => *p = slot as u8,
                DpiCScalar::I16 => *(p as *mut i16) = slot as i16,
                DpiCScalar::I32 => *(p as *mut i32) = slot as i32,
                DpiCScalar::I64 => *(p as *mut i64) = slot,
                DpiCScalar::F32 => *(p as *mut f32) = f64::from_bits(slot as u64) as f32,
                DpiCScalar::F64 => *(p as *mut f64) = f64::from_bits(slot as u64),
                DpiCScalar::Chandle | DpiCScalar::Str => *(p as *mut usize) = slot as usize,
            }
        }
    }
}

/// The text of a C string (empty for NULL).
fn c_string_at(p: usize) -> String {
    if p == 0 {
        return String::new();
    }
    // SAFETY: the C caller passes a NUL-terminated string (§35.5.6).
    unsafe { std::ffi::CStr::from_ptr(p as *const libc::c_char) }
        .to_string_lossy()
        .into_owned()
}

/// The C type of an exported subroutine's formal, as it lies in memory.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum DpiCType {
    Scalar(DpiCScalar),
    /// A packed vector: `svLogicVecVal[n]` (4-state, `{aval, bval}` pairs)
    /// or `svBitVecVal[n]` (2-state), n = ceil(width / 32).
    Vec {
        width: u32,
        four: bool,
    },
    /// A fixed-size unpacked array: its elements in a row (H.7.5), the
    /// first dimension outermost; `dims` holds each `(left, right)`, and C
    /// index 0 of a dimension is its lowest index.
    Array {
        elem: Box<DpiCType>,
        dims: Vec<(i64, i64)>,
    },
    /// An unpacked struct: its members as the members of a C struct.
    Struct(Vec<(String, DpiCType)>),
}

impl DpiCType {
    pub(super) fn size_align(&self) -> (usize, usize) {
        match self {
            DpiCType::Scalar(s) => (s.size(), s.size()),
            DpiCType::Vec { width, four } => {
                let n = width.div_ceil(32).max(1) as usize;
                (n * if *four { 8 } else { 4 }, 4)
            }
            DpiCType::Array { elem, dims } => {
                let (s, a) = elem.size_align();
                (s * dims_count(dims), a)
            }
            DpiCType::Struct(members) => {
                let (mut off, mut align) = (0usize, 1usize);
                for (_, m) in members {
                    let (s, a) = m.size_align();
                    off = off.next_multiple_of(a);
                    off += s;
                    align = align.max(a);
                }
                (off.next_multiple_of(align).max(1), align)
            }
        }
    }

    /// The width of a leaf's SV value (0 for a string or a real).
    fn leaf_width(&self) -> u32 {
        match self {
            DpiCType::Scalar(s) => match s {
                DpiCScalar::I8 => 8,
                DpiCScalar::I16 => 16,
                DpiCScalar::I32 => 32,
                DpiCScalar::I64 | DpiCScalar::Chandle => 64,
                DpiCScalar::Bit | DpiCScalar::Logic => 1,
                _ => 0,
            },
            DpiCType::Vec { width, .. } => *width,
            _ => 0,
        }
    }
}

fn dims_count(dims: &[(i64, i64)]) -> usize {
    dims.iter()
        .map(|&(l, r)| (l - r).unsigned_abs() as usize + 1)
        .product()
}

/// The SV index of C index `k` in a dimension `(left, right)`: C index 0
/// is the lowest index, whichever way the range runs (as the reference
/// simulator lays out `int a[5:2]`: `a[2]` first).
fn dim_index((l, r): (i64, i64), k: usize) -> i64 {
    l.min(r) + k as i64
}

/// How the trampoline passes one formal.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum DpiExpArg {
    /// An input scalar, by value.
    Val(DpiCScalar),
    /// By pointer: an `output`/`inout` of any type, or an input packed
    /// vector or aggregate (`const T*`). `inp`: the pointee is read before
    /// the call; `out`: the formal's final value is stored back.
    Ptr { ty: DpiCType, inp: bool, out: bool },
}

/// What a call from C runs, per exported subroutine.
#[derive(Debug, Clone)]
pub(super) struct DpiExpSig {
    /// The C result: `None` for `void`.
    ret: Option<DpiCScalar>,
    args: Vec<DpiExpArg>,
    /// Why C cannot call it: the formal and its type.
    bad: Option<String>,
    /// The result type is not a legal DPI result (§35.5.5).
    bad_ret: Option<String>,
    /// Each formal's declared type.
    dts: Vec<DataType>,
}

/// A formal not modelled: its description for the messages.
type Unmodelled = String;

impl Simulator {
    /// The unpacked dimensions of a formal: its own, then its typedef's.
    fn dpi_formal_dims(
        &self,
        dt: &DataType,
        dims: &[crate::ast::types::UnpackedDimension],
    ) -> Result<Vec<(i64, i64)>, Unmodelled> {
        use crate::ast::types::UnpackedDimension as UD;
        let mut all: Vec<UD> = dims.to_vec();
        let mut cur = dt;
        for _ in 0..64 {
            let DataType::TypeReference { name, .. } = cur else {
                break;
            };
            if let Some(d) = self.module.typedef_unpacked_dims.get(&name.name.name) {
                all.extend(d.iter().cloned());
            }
            match self.module.typedef_types.get(&name.name.name) {
                Some(next) => cur = next,
                None => break,
            }
        }
        let norm = crate::compiler::elaborate::normalize_unpacked_dims(
            &all,
            &self.module.parameters,
            &self.module.typedef_types,
        );
        let params = Some(&self.module.parameters);
        norm.iter()
            .map(|d| match d {
                UD::Range { left, right, .. } => {
                    let l = crate::compiler::elaborate::const_eval_i64_with_params(left, params);
                    let r = crate::compiler::elaborate::const_eval_i64_with_params(right, params);
                    l.zip(r)
                        .ok_or_else(|| "an array with non-constant bounds".to_string())
                }
                UD::Expression { expr, .. } => {
                    match crate::compiler::elaborate::const_eval_i64_with_params(expr, params) {
                        Some(n) if n > 0 => Ok((0, n - 1)),
                        _ => Err("an array with non-constant bounds".to_string()),
                    }
                }
                _ => Err(
                    "a dynamic array, queue or associative array, which an exported \
                     subroutine cannot take"
                        .to_string(),
                ),
            })
            .collect()
    }

    /// The C type of `dt` with unpacked `dims`. `top`: a formal itself, where
    /// a packed vector of up to 64 bits passes by value as `int` or `long
    /// long`; inside an aggregate every packed vector is `svLogicVecVal[]` /
    /// `svBitVecVal[]`, as Annex H lays it out.
    pub(super) fn dpi_c_type(
        &self,
        dt: &DataType,
        dims: &[crate::ast::types::UnpackedDimension],
        top: bool,
    ) -> Result<DpiCType, Unmodelled> {
        use crate::ast::types::{IntegerAtomType as IA, IntegerVectorType, RealType, SimpleType};
        let dims = self.dpi_formal_dims(dt, dims)?;
        if !dims.is_empty() {
            let elem = self.dpi_c_type(dt, &[], false)?;
            if let DpiCType::Array { .. } = elem {
                return Err("an array of arrays".to_string());
            }
            return Ok(DpiCType::Array {
                elem: Box::new(elem),
                dims,
            });
        }
        let rdt = crate::compiler::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let width = || {
            crate::compiler::elaborate::resolve_type_width(
                rdt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
            .max(1)
        };
        let packed = |w: u32, four: bool| {
            if top && w <= 32 {
                DpiCType::Scalar(DpiCScalar::I32)
            } else if top && w <= 64 {
                DpiCType::Scalar(DpiCScalar::I64)
            } else {
                DpiCType::Vec { width: w, four }
            }
        };
        Ok(match rdt {
            DataType::Real { kind, .. } => DpiCType::Scalar(match kind {
                RealType::ShortReal => DpiCScalar::F32,
                _ => DpiCScalar::F64,
            }),
            DataType::Simple { kind, .. } => DpiCType::Scalar(match kind {
                SimpleType::String => DpiCScalar::Str,
                SimpleType::Chandle => DpiCScalar::Chandle,
                SimpleType::Event => {
                    return Err("event, which DPI does not carry".to_string());
                }
            }),
            DataType::IntegerAtom { kind, .. } => match kind {
                IA::Byte => DpiCType::Scalar(DpiCScalar::I8),
                IA::ShortInt => DpiCType::Scalar(DpiCScalar::I16),
                IA::Int => DpiCType::Scalar(DpiCScalar::I32),
                IA::LongInt => DpiCType::Scalar(DpiCScalar::I64),
                // 4-state packed vectors (§6.11).
                IA::Integer => packed(32, true),
                IA::Time => packed(64, true),
            },
            DataType::IntegerVector {
                kind, dimensions, ..
            } => {
                let two = matches!(kind, IntegerVectorType::Bit);
                if dimensions.is_empty() {
                    DpiCType::Scalar(if two {
                        DpiCScalar::Bit
                    } else {
                        DpiCScalar::Logic
                    })
                } else {
                    packed(width(), !two)
                }
            }
            DataType::Implicit { dimensions, .. } => {
                if dimensions.is_empty() {
                    DpiCType::Scalar(DpiCScalar::Logic)
                } else {
                    packed(width(), true)
                }
            }
            DataType::Struct(su) if su.packed => {
                packed(width(), !self.packed_aggregate_two_state(su))
            }
            DataType::Struct(su) => {
                if !matches!(su.kind, crate::ast::types::StructUnionKind::Struct) {
                    return Err("an unpacked union".to_string());
                }
                let mut members = Vec::new();
                for m in &su.members {
                    for d in &m.declarators {
                        let t = self.dpi_c_type(&m.data_type, &d.dimensions, false)?;
                        members.push((d.name.name.clone(), t));
                    }
                }
                if members.is_empty() {
                    return Err("an empty struct".to_string());
                }
                DpiCType::Struct(members)
            }
            DataType::Enum(e) => match e.base_type.as_deref() {
                Some(bt) => return self.dpi_c_type(bt, &[], top),
                None => DpiCType::Scalar(DpiCScalar::I32),
            },
            DataType::TypeReference { name, .. } => {
                return Err(format!(
                    "'{}', a class handle or a type DPI does not carry",
                    name.name.name
                ));
            }
            DataType::Interface { .. } => {
                return Err("a virtual interface, which DPI does not carry".to_string());
            }
            DataType::Void(_) => return Err("void".to_string()),
        })
    }

    /// The C side of every formal and of the result of exported subroutine
    /// `name`; None if it is neither a function nor a task.
    pub(super) fn dpi_export_signature(&self, name: &str) -> Option<DpiExpSig> {
        let (ret_dt, ports, task) = if let Some(fd) = self.module.functions.get(name) {
            (Some(&fd.return_type), &fd.ports, false)
        } else if let Some(td) = self.module.tasks.get(name) {
            (None, &td.ports, true)
        } else {
            return None;
        };
        let mut sig = DpiExpSig {
            ret: None,
            args: Vec::new(),
            bad: None,
            bad_ret: None,
            dts: ports.iter().map(|p| p.data_type.clone()).collect(),
        };
        if task {
            // §35.9: the C function for an exported task returns int — 1
            // when it returned because of a disable, else 0.
            sig.ret = Some(DpiCScalar::I32);
        } else if let Some(rt) = ret_dt.filter(|t| !matches!(t, DataType::Void(_))) {
            // §35.5.5: a result is a small value — never an aggregate or a
            // packed vector the C ABI cannot return in a register.
            match self.dpi_c_type(rt, &[], true) {
                Ok(DpiCType::Scalar(s)) => sig.ret = Some(s),
                Ok(DpiCType::Vec { width, .. }) => {
                    sig.bad_ret = Some(format!(
                        "a packed vector of {} bits (a DPI function result is at most 64 bits \
                         wide; pass it through an output argument)",
                        width
                    ))
                }
                Ok(_) => {
                    sig.bad_ret = Some(
                        "an unpacked aggregate (pass it through an output argument)".to_string(),
                    )
                }
                Err(why) => sig.bad_ret = Some(why),
            }
        }
        for p in ports {
            let ty = match self.dpi_c_type(&p.data_type, &p.dimensions, true) {
                Ok(t) => t,
                Err(why) => {
                    if sig.bad.is_none() {
                        sig.bad = Some(format!("formal '{}' has type {}", p.name.name, why));
                    }
                    sig.args.push(DpiExpArg::Val(DpiCScalar::I64));
                    continue;
                }
            };
            let (inp, out) = match p.direction {
                PortDirection::Output => (false, true),
                PortDirection::Inout | PortDirection::Ref => (true, true),
                _ => (true, false),
            };
            sig.args.push(match ty {
                DpiCType::Scalar(s) if !out => DpiExpArg::Val(s),
                ty => DpiExpArg::Ptr { ty, inp, out },
            });
        }
        Some(sig)
    }

    /// Build and load the export trampoline; false when it was needed but
    /// could not be built.
    pub(super) fn load_dpi_export_trampoline(&mut self) -> bool {
        let exports = self.module.dpi_exports.clone();
        if exports.is_empty() {
            return true;
        }
        // Exported functions are only reachable from a loaded DPI library; with
        // none configured, nothing can call them from C — skip the compile.
        if configured_dpi_libs().is_empty() {
            return true;
        }
        let mut body = String::new();
        body.push_str("#include <string.h>\n");
        body.push_str(
            "extern long long __xezim_dpi_export_dispatch(long long id, long long n, const long long* a);\n",
        );
        let mut emitted = 0usize;
        let c_names = self.module.dpi_export_c_names.clone();
        // §35.5.3: every instance of a module that exports a subroutine
        // shares its C symbol; one entry point serves them all and the
        // dispatch picks the instance by scope (`dpi_export_for_scope`).
        let mut seen_c: HashSet<&str> = HashSet::default();
        for (id, name) in exports.iter().enumerate() {
            // The symbol the C side links against: the export's alias when
            // one was declared, else the SV name.
            let c_name: &str = c_names.get(id).map(|s| s.as_str()).unwrap_or(name);
            if !seen_c.insert(c_name) {
                continue;
            }
            let Some(sig) = self.dpi_export_signature(name) else {
                continue;
            };
            if let Some(why) = &sig.bad_ret {
                // §35.5.5: as the reference simulator, refuse to run.
                self.compile_errors.push(format!(
                    "exported function '{}' returns {}, which is not a legal DPI result type \
                     (IEEE 1800 clause 35.5.5)",
                    name, why
                ));
                continue;
            }
            let params: Vec<String> = sig
                .args
                .iter()
                .enumerate()
                .map(|(i, a)| match a {
                    DpiExpArg::Val(s) => format!("{} a{}", s.c_type(), i),
                    DpiExpArg::Ptr { .. } => format!("void* a{}", i),
                })
                .collect();
            let param_list = if params.is_empty() {
                "void".to_string()
            } else {
                params.join(", ")
            };
            let ret_c = sig.ret.map_or("void", |s| s.c_type());
            if let Some(why) = &sig.bad {
                eprintln!(
                    "[DPI] exported subroutine '{}' is NOT callable from C: {}; a call from C \
                     reports an error and returns 0",
                    name, why
                );
                // Keeps the symbol (the user library still loads); a call
                // reports the error (`run_dpi_export_unmodelled`).
                let call = format!("__xezim_dpi_export_dispatch({}, -1, 0)", id);
                body.push_str(&match sig.ret {
                    None => format!("void {}({}) {{ (void){}; }}\n", c_name, param_list, call),
                    Some(_) => format!(
                        "{} {}({}) {{ (void){}; return 0; }}\n",
                        ret_c, c_name, param_list, call
                    ),
                });
                emitted += 1;
                continue;
            }
            // Pack each argument into a 64-bit slot: integers by value, reals by
            // their IEEE-754 bit pattern (so the single integer dispatch carries
            // both), pointers by their address.
            let mut pack = format!("    long long __a[{}];\n", sig.args.len().max(1));
            if sig.args.is_empty() {
                pack.push_str("    (void)__a;\n");
            }
            for (i, a) in sig.args.iter().enumerate() {
                pack.push_str(&match a {
                    DpiExpArg::Val(s) if s.is_real() => {
                        format!("    {{ double __t = a{i}; memcpy(&__a[{i}], &__t, 8); }}\n")
                    }
                    DpiExpArg::Val(DpiCScalar::Chandle | DpiCScalar::Str)
                    | DpiExpArg::Ptr { .. } => {
                        format!("    __a[{i}] = (long long)(unsigned long)a{i};\n")
                    }
                    DpiExpArg::Val(_) => format!("    __a[{i}] = (long long)a{i};\n"),
                });
            }
            let call = format!(
                "__xezim_dpi_export_dispatch({}, {}, __a)",
                id,
                sig.args.len()
            );
            body.push_str(&match sig.ret {
                None => format!(
                    "void {}({}) {{\n{}    (void){};\n}}\n",
                    c_name, param_list, pack, call
                ),
                Some(s) if s.is_real() => format!(
                    "{ret_c} {c_name}({param_list}) {{\n{pack}    long long __r = {call};\n    double __d; memcpy(&__d, &__r, 8); return ({ret_c})__d;\n}}\n"
                ),
                Some(DpiCScalar::Chandle | DpiCScalar::Str) => format!(
                    "{ret_c} {c_name}({param_list}) {{\n{pack}    return ({ret_c})(unsigned long){call};\n}}\n"
                ),
                Some(_) => format!(
                    "{ret_c} {c_name}({param_list}) {{\n{pack}    return ({ret_c}){call};\n}}\n"
                ),
            });
            emitted += 1;
        }
        if emitted == 0 {
            return true;
        }
        let dir = std::env::temp_dir();
        let stem = format!("xezim_dpi_exports_{}", std::process::id());
        let cpath = dir.join(format!("{}.c", stem));
        let sopath = dir.join(format!("{}.so", stem));
        if let Err(e) = std::fs::write(&cpath, &body) {
            eprintln!("[DPI] could not write export trampoline source: {}", e);
            return false;
        }
        let cmd = dpi_trampoline_cc_command(std::env::var("CC").ok().as_deref(), &sopath, &cpath);
        let status = std::process::Command::new(&cmd[0]).args(&cmd[1..]).status();
        match status {
            Ok(st) if st.success() => {}
            Ok(st) => {
                eprintln!(
                    "[DPI] export trampoline compile failed (exit {:?}); C callbacks into \
                     exported SV functions will be unresolved",
                    st.code()
                );
                return false;
            }
            Err(e) => {
                eprintln!(
                    "[DPI] could not run the C compiler `{}` for the export trampoline ({}); \
                     set $CC or put `cc` on PATH",
                    cmd[0].to_string_lossy(),
                    e
                );
                return false;
            }
        }
        use libloading::os::unix::{Library as UnixLibrary, RTLD_GLOBAL, RTLD_NOW};
        let loaded = match unsafe { UnixLibrary::open(Some(&sopath), RTLD_NOW | RTLD_GLOBAL) } {
            Ok(l) => {
                self.dpi_libraries.push(Library::from(l));
                true
            }
            Err(e) => {
                eprintln!("[DPI] failed to load export trampoline: {}", e);
                false
            }
        };
        let _ = std::fs::remove_file(&cpath);
        let _ = std::fs::remove_file(&sopath);
        loaded
    }

    /// A call from C of an export that is not callable from C: an error at
    /// the call site, counted, instead of a silent 0.
    pub(super) fn run_dpi_export_unmodelled(&mut self, id: usize) -> i64 {
        let id = self.dpi_export_for_scope(id).unwrap_or(id);
        let Some(name) = self.module.dpi_exports.get(id).cloned() else {
            return 0;
        };
        let c = self
            .module
            .dpi_export_c_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| name.clone());
        let why = self
            .dpi_export_signature(&name)
            .and_then(|s| s.bad)
            .unwrap_or_else(|| "a type DPI does not carry".to_string());
        let m = format!(
            "[xezim][error] DPI export '{}' cannot be called from C: {} (IEEE 1800 clause \
             35.5.6); the call returns 0 (t={})",
            c, why, self.time
        );
        self.error_count = self.error_count.saturating_add(1);
        self.record_output(m.clone());
        self.stdout_writeln(&m);
        0
    }

    /// The SV value of a packed vector at `p` (`svLogicVecVal[]` or
    /// `svBitVecVal[]`).
    ///
    /// # Safety
    /// `p` points at `ceil(width / 32)` elements.
    unsafe fn dpi_load_vec(p: *const u8, width: u32, four: bool) -> Value {
        let n = width.div_ceil(32).max(1) as usize;
        unsafe {
            if four {
                let words = std::slice::from_raw_parts(p as *const u32, 2 * n);
                Self::dpi_logic_interleaved_to_value(words, width)
            } else {
                let words = std::slice::from_raw_parts(p as *const u32, n);
                Self::dpi_logic_words_to_value(words, &[], width)
            }
        }
    }

    /// Store `v` as a packed vector at `p`.
    ///
    /// # Safety
    /// `p` points at `ceil(width / 32)` writable elements.
    unsafe fn dpi_store_vec(p: *mut u8, width: u32, four: bool, v: &Value) {
        let v = if v.is_real {
            Value::from_u64(v.to_f64() as i64 as u64, 64).resize(width)
        } else {
            v.resize(width)
        };
        let n = width.div_ceil(32).max(1) as usize;
        unsafe {
            if four {
                let words = Self::dpi_value_to_logic_interleaved(&v, width);
                std::ptr::copy_nonoverlapping(words.as_ptr(), p as *mut u32, 2 * n);
            } else {
                // 2-state: X and Z read as 0 (§35.5.6).
                let (aval, bval) = Self::dpi_value_to_logic_words(&v, width);
                for i in 0..n {
                    *(p as *mut u32).add(i) = aval[i] & !bval[i];
                }
            }
        }
    }

    /// The leaves of a value of C type `ty` at `p`: `(suffix, value)` with
    /// the suffix naming the leaf below the variable (`""`, `[2]`, `.a`,
    /// `[1].x`), as xezim stores aggregates.
    ///
    /// # Safety
    /// `p` points at a valid object of type `ty`.
    pub(super) unsafe fn dpi_load_leaves(
        ty: &DpiCType,
        p: *const u8,
        sfx: &str,
        out: &mut Vec<(String, Value)>,
    ) {
        unsafe {
            match ty {
                DpiCType::Scalar(s) => out.push((sfx.to_string(), s.slot_to_value(s.load(p)))),
                DpiCType::Vec { width, four } => {
                    out.push((sfx.to_string(), Self::dpi_load_vec(p, *width, *four)))
                }
                DpiCType::Array { elem, dims } => {
                    let (es, _) = elem.size_align();
                    for (k, isfx) in dpi_array_suffixes(dims).iter().enumerate() {
                        Self::dpi_load_leaves(elem, p.add(k * es), &format!("{sfx}{isfx}"), out);
                    }
                }
                DpiCType::Struct(members) => {
                    let mut off = 0usize;
                    for (m, t) in members {
                        let (s, a) = t.size_align();
                        off = off.next_multiple_of(a);
                        Self::dpi_load_leaves(t, p.add(off), &format!("{sfx}.{m}"), out);
                        off += s;
                    }
                }
            }
        }
    }

    /// Store the leaves `get` returns into a C object of type `ty` at `p`.
    ///
    /// # Safety
    /// `p` points at a valid, writable object of type `ty`.
    pub(super) unsafe fn dpi_store_leaves(
        ty: &DpiCType,
        p: *mut u8,
        sfx: &str,
        get: &mut dyn FnMut(&str) -> Option<Value>,
        keep: &mut Vec<CString>,
    ) {
        unsafe {
            match ty {
                DpiCType::Scalar(s) => {
                    if let Some(v) = get(sfx) {
                        s.store(p, s.value_to_slot(&v, keep));
                    }
                }
                DpiCType::Vec { width, four } => {
                    if let Some(v) = get(sfx) {
                        Self::dpi_store_vec(p, *width, *four, &v);
                    }
                }
                DpiCType::Array { elem, dims } => {
                    let (es, _) = elem.size_align();
                    for (k, isfx) in dpi_array_suffixes(dims).iter().enumerate() {
                        Self::dpi_store_leaves(
                            elem,
                            p.add(k * es),
                            &format!("{sfx}{isfx}"),
                            get,
                            keep,
                        );
                    }
                }
                DpiCType::Struct(members) => {
                    let mut off = 0usize;
                    for (m, t) in members {
                        let (s, a) = t.size_align();
                        off = off.next_multiple_of(a);
                        Self::dpi_store_leaves(t, p.add(off), &format!("{sfx}.{m}"), get, keep);
                        off += s;
                    }
                }
            }
        }
    }

    /// §35.5.4: run an exported SV subroutine identified by its declaration-order
    /// id, called from C through the generated trampoline. `args` points at
    /// `nargs` 64-bit slots (integers by value, reals as IEEE-754 bits,
    /// pointers as addresses). The return is a 64-bit slot: an integer value,
    /// a real's bit pattern, or an address.
    pub(super) fn run_dpi_export(&mut self, id: usize, nargs: usize, args: *const i64) -> i64 {
        // §35.9 d): an imported subroutine in the disabled state may not call
        // an export any more.
        if self.dpi_unwinding {
            self.dpi_protocol_fatal(
                "an exported subroutine was called after its imported caller was disabled",
            );
            return 1;
        }
        if self.dpi_cur_task.is_some() && self.dpi_stack_exhausted() {
            return 0;
        }
        let Some(id) = self.dpi_export_for_scope(id) else {
            let (c, scope) = (
                self.module
                    .dpi_export_c_names
                    .get(id)
                    .cloned()
                    .unwrap_or_default(),
                dpi_task::dpi_scope_name(ACTIVE_SCOPE.with(|cell| cell.get())),
            );
            self.emit_severity_text(
                "Fatal",
                &format!(
                    "DPI export '{}' is not declared in the calling scope '{}' or a scope above it (IEEE 1800 clause 35.5.3)",
                    c, scope
                ),
            );
            self.fatal_finish_number = Some(1);
            self.finished = true;
            return 0;
        };
        let Some(name) = self.module.dpi_exports.get(id).cloned() else {
            return 0;
        };
        let Some(sig) = self.dpi_export_signature(&name) else {
            return 0;
        };
        if sig.bad.is_some() {
            return self.run_dpi_export_unmodelled(id);
        }
        let raw: Vec<i64> = (0..nargs).map(|i| unsafe { *args.add(i) }).collect();
        // Every formal C passes by pointer binds to a variable of this
        // dispatch holding the pointee: a frame local for a scalar or a
        // packed vector, a set of hidden leaves for an aggregate. Its final
        // value is stored back through the pointer after the call (§35.5.6).
        let local = |i: usize| format!("__dpi_export_arg{}", i);
        self.dpi_export_seq += 1;
        let agg_base = |i: usize, seq: u64| format!("__xz_dpi_export{}_arg{}", seq, i);
        let seq = self.dpi_export_seq;
        let mut frame: HashMap<String, Value> = HashMap::default();
        let mut aggs: Vec<(usize, String, Vec<String>)> = Vec::new();
        let mut arg_exprs: Vec<Expression> = Vec::with_capacity(raw.len());
        let ident = |n: String| {
            Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: None,
                    path: vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: n,
                            span: crate::ast::Span::dummy(),
                        },
                        selects: Vec::new(),
                    }],
                    span: crate::ast::Span::dummy(),
                    cached_signal_id: crate::ast::cache::SignalCache::new(None),
                    cached_resolved_name: crate::ast::cache::NameCache::new(),
                    bound: crate::ast::binding::Bound::UNSET,
                }),
                crate::ast::Span::dummy(),
            )
        };
        for (i, &slot) in raw.iter().enumerate() {
            match sig.args.get(i) {
                Some(DpiExpArg::Val(s @ (DpiCScalar::Bit | DpiCScalar::Logic))) => {
                    frame.insert(local(i), s.slot_to_value(slot));
                    arg_exprs.push(ident(local(i)));
                }
                Some(DpiExpArg::Val(s)) => {
                    let e = match s {
                        DpiCScalar::F32 | DpiCScalar::F64 => {
                            ExprKind::Number(NumberLiteral::Real(f64::from_bits(slot as u64)))
                        }
                        DpiCScalar::Str => ExprKind::StringLiteral(c_string_at(slot as usize)),
                        _ => ExprKind::Number(NumberLiteral::Integer {
                            size: Some(64),
                            signed: true,
                            base: NumberBase::Decimal,
                            value: slot.to_string(),
                            cached_val: crate::ast::cache::LiteralCache::new(Some((slot as u64, 0u64, 64u32))),
                        }),
                    };
                    arg_exprs.push(Expression::new(e, crate::ast::Span::dummy()));
                }
                Some(DpiExpArg::Ptr { ty, inp, .. }) => {
                    // An output's pointee is not read: it starts from its
                    // type's default (§35.5.6).
                    let mut leaves = Vec::new();
                    if *inp && slot != 0 {
                        // SAFETY: C passes a pointer to an object of this
                        // type (§35.5.6, Annex H.8).
                        unsafe {
                            Self::dpi_load_leaves(ty, slot as usize as *const u8, "", &mut leaves)
                        };
                    } else {
                        dpi_default_leaves(ty, "", &mut leaves);
                    }
                    if let DpiCType::Scalar(_) | DpiCType::Vec { .. } = ty {
                        let v = leaves
                            .pop()
                            .map(|(_, v)| v)
                            .unwrap_or_else(|| Value::zero(32));
                        frame.insert(local(i), v);
                        arg_exprs.push(ident(local(i)));
                    } else {
                        let base = agg_base(i, seq);
                        let dt = sig
                            .dts
                            .get(i)
                            .cloned()
                            .unwrap_or(DataType::Void(crate::ast::Span::dummy()));
                        let names = self.dpi_export_bind_aggregate(&base, &dt, ty, leaves);
                        aggs.push((i, base.clone(), names));
                        arg_exprs.push(ident(base));
                    }
                }
                None => arg_exprs.push(ident(local(i))),
            }
        }
        let has_frame = !frame.is_empty();
        if has_frame {
            self.push_local_frame(frame);
            // §35.5.6: a C string pointer is an unbounded string variable,
            // not an integral local sized by its current character count.
            // Preserve that kind when export copy-out grows the value.
            for (i, arg) in sig.args.iter().enumerate() {
                if matches!(
                    arg,
                    DpiExpArg::Ptr {
                        ty: DpiCType::Scalar(DpiCScalar::Str),
                        ..
                    }
                ) {
                    self.record_frame_string_kind(&local(i), true);
                }
            }
        }
        let mut ret_val: Option<Value> = None;
        // An instance's export runs in that instance, as a call through its
        // hierarchical name does (`$time`, `%m` and bare names included).
        let inst: Option<String> = name
            .rsplit_once('.')
            .filter(|_| !name.contains("::"))
            .map(|(p, _)| p.to_string());
        let saved_scope = inst.as_ref().map(|sc| {
            let hint = self.name_resolve_hint.replace(Some(sc.clone()));
            let ts = self.timescale_scope_override.replace(sc.clone());
            (hint, ts)
        });
        let on_fiber = self.dpi_note_export(&name, true);
        let mut is_task = false;
        if let Some(fd) = self.fn_decl_rc(&name) {
            self.dpi_export_depth += 1;
            let r = self.exec_function_call(&fd, &arg_exprs);
            self.dpi_export_depth -= 1;
            ret_val = Some(r);
        } else if let Some(td) = self.task_decl_rc(&name) {
            self.dpi_export_depth += 1;
            self.exec_task_call(&td, &arg_exprs);
            self.dpi_export_depth -= 1;
            is_task = true;
        }
        if on_fiber {
            self.dpi_note_export(&name, false);
        }
        if let Some((hint, ts)) = saved_scope {
            *self.name_resolve_hint.borrow_mut() = hint;
            self.timescale_scope_override = ts;
        }
        // Strings handed to C live until the next call of this export.
        let mut keep: Vec<CString> = Vec::new();
        let frame = if has_frame {
            self.pop_local_frame_take().unwrap_or_default()
        } else {
            HashMap::default()
        };
        for (i, a) in sig.args.iter().enumerate() {
            let DpiExpArg::Ptr { ty, out: true, .. } = a else {
                continue;
            };
            let Some(&slot) = raw.get(i) else { continue };
            if slot == 0 || self.dpi_unwinding {
                continue;
            }
            let p = slot as usize as *mut u8;
            let vals: HashMap<String, Value> =
                if let Some((_, base, _)) = aggs.iter().find(|(j, ..)| *j == i) {
                    let mut sfxs = Vec::new();
                    dpi_default_leaves(ty, "", &mut sfxs);
                    sfxs.into_iter()
                        .filter_map(|(sfx, _)| {
                            let v = self.get_signal_value_by_name(&format!("{}{}", base, sfx))?;
                            Some((sfx, v))
                        })
                        .collect()
                } else {
                    frame
                        .get(&local(i))
                        .map(|v| (String::new(), v.clone()))
                        .into_iter()
                        .collect()
                };
            let mut get = |sfx: &str| vals.get(sfx).cloned();
            // SAFETY: `p` is the caller's writable object of this type
            // (§35.5.6, Annex H.8).
            unsafe { Self::dpi_store_leaves(ty, p, "", &mut get, &mut keep) };
        }
        for (_, base, names) in aggs {
            self.dpi_export_unbind_aggregate(&base, &names);
        }
        let ret = if is_task {
            // §35.9 a): an exported task returns 1 when it returns because
            // its imported caller was disabled, else 0.
            self.dpi_unwinding as i64
        } else {
            match (sig.ret, ret_val) {
                (Some(s), Some(v)) => s.value_to_slot(&v, &mut keep),
                _ => 0,
            }
        };
        if keep.is_empty() {
            self.dpi_export_cstrings.remove(&id);
        } else {
            self.dpi_export_cstrings.insert(id, keep);
        }
        ret
    }
}

impl Simulator {
    /// Make the leaves of an aggregate C passed in a hidden variable `base`
    /// of the formal's type, as a module variable of that type is stored:
    /// its leaf signals, its array shape, its declared type. Returns the
    /// leaf names, for `dpi_export_unbind_aggregate`.
    fn dpi_export_bind_aggregate(
        &mut self,
        base: &str,
        dt: &DataType,
        ty: &DpiCType,
        leaves: Vec<(String, Value)>,
    ) -> Vec<String> {
        let mut names = Vec::with_capacity(leaves.len());
        for (sfx, v) in leaves {
            let n = format!("{}{}", base, sfx);
            self.signals.insert(n.clone(), v);
            names.push(n);
        }
        // A string leaf is stored as text of any length (§6.16), not cut
        // to the width its first value had.
        let mut strs = Vec::new();
        dpi_string_leaves(ty, base, &mut strs);
        // A struct's string member is stored as wide as the string type
        // (`resolve_type_width`), which a formal bound from it keeps: stored
        // at the length of its first text, a longer one written in the
        // subroutine was cut to it.
        let str_w = crate::compiler::elaborate::resolve_type_width(
            &DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                span: crate::ast::Span::dummy(),
            },
            None,
            None,
        );
        for n in strs {
            if let Some(v) = self.signals.get(&n) {
                if v.width < str_w {
                    let wide = v.resize(str_w);
                    self.signals.insert(n.clone(), wide);
                }
            }
            self.module.string_signals.insert(n.clone());
            self.string_signals.insert(n.clone());
            names.push(n);
        }
        self.module
            .var_decl_types
            .insert(base.to_string(), dt.clone());
        match ty {
            DpiCType::Array { elem, dims } => {
                let w = elem.leaf_width().max(1);
                match dims.as_slice() {
                    [(l, r)] => {
                        self.module
                            .arrays
                            .insert(base.to_string(), (*l.min(r), *l.max(r), w));
                        if l > r {
                            self.module.descending_arrays.insert(base.to_string());
                        }
                    }
                    [a, b] => {
                        self.module.arrays_2d.insert(base.to_string(), (*a, *b, w));
                        self.multi_dim_array_names.insert(base.to_string());
                    }
                    _ => {
                        self.module
                            .arrays_nd
                            .insert(base.to_string(), (dims.clone(), w));
                        self.multi_dim_array_names.insert(base.to_string());
                    }
                }
                if matches!(**elem, DpiCType::Struct(_)) {
                    self.note_struct_root(base);
                }
            }
            DpiCType::Struct(_) => self.note_struct_root(base),
            _ => {}
        }
        names
    }

    /// Remove what `dpi_export_bind_aggregate` made.
    fn dpi_export_unbind_aggregate(&mut self, base: &str, names: &[String]) {
        for n in names {
            self.signals.remove(n);
            self.module.string_signals.remove(n);
            self.string_signals.remove(n);
        }
        self.module.var_decl_types.remove(base);
        self.module.arrays.remove(base);
        self.module.arrays_2d.remove(base);
        self.module.arrays_nd.remove(base);
        self.module.descending_arrays.remove(base);
        self.multi_dim_array_names.remove(base);
    }
}

/// The leaves of a value of C type `ty`, each holding its type's default
/// (§6.8: x for a 4-state type, 0 for a 2-state one, "" for a string).
fn dpi_default_leaves(ty: &DpiCType, sfx: &str, out: &mut Vec<(String, Value)>) {
    match ty {
        DpiCType::Scalar(s) => out.push((
            sfx.to_string(),
            match s {
                DpiCScalar::F32 | DpiCScalar::F64 => Value::from_f64(0.0),
                DpiCScalar::Str => Value::from_string(""),
                DpiCScalar::Logic => Value::new(1),
                _ => s.slot_to_value(0),
            },
        )),
        DpiCType::Vec { width, four } => out.push((
            sfx.to_string(),
            if *four {
                Value::new(*width)
            } else {
                Value::zero(*width)
            },
        )),
        DpiCType::Array { elem, dims } => {
            for isfx in dpi_array_suffixes(dims) {
                dpi_default_leaves(elem, &format!("{sfx}{isfx}"), out);
            }
        }
        DpiCType::Struct(members) => {
            for (m, t) in members {
                dpi_default_leaves(t, &format!("{sfx}.{m}"), out);
            }
        }
    }
}

/// The names of the string leaves of a value of C type `ty` named `name`.
fn dpi_string_leaves(ty: &DpiCType, name: &str, out: &mut Vec<String>) {
    match ty {
        DpiCType::Scalar(DpiCScalar::Str) => out.push(name.to_string()),
        DpiCType::Array { elem, dims } => {
            if matches!(
                **elem,
                DpiCType::Scalar(DpiCScalar::Str) | DpiCType::Struct(_)
            ) {
                for isfx in dpi_array_suffixes(dims) {
                    dpi_string_leaves(elem, &format!("{name}{isfx}"), out);
                }
            }
        }
        DpiCType::Struct(members) => {
            for (m, t) in members {
                dpi_string_leaves(t, &format!("{name}.{m}"), out);
            }
        }
        _ => {}
    }
}

/// The index suffixes of a fixed array's elements in C order (row-major,
/// C index 0 of each dimension at its lowest index).
fn dpi_array_suffixes(dims: &[(i64, i64)]) -> Vec<String> {
    let mut out = vec![String::new()];
    for &d in dims {
        let n = (d.0 - d.1).unsigned_abs() as usize + 1;
        let mut next = Vec::with_capacity(out.len() * n);
        for pre in &out {
            for k in 0..n {
                next.push(format!("{}[{}]", pre, dim_index(d, k)));
            }
        }
        out = next;
    }
    out
}
