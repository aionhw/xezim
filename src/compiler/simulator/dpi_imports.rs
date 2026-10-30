//! Simulator dpi imports.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn load_dpi_libraries(&mut self) {
        // §35.5.4 DPI EXPORT: a user DPI library references each exported SV
        // function as an undefined C symbol (`extern int sv_add(int,int);`).
        // Provide those symbols BEFORE loading the user libs by generating a
        // tiny trampoline shared object — one C function per exported name that
        // forwards into `__xezim_dpi_export_dispatch`, which runs the SV
        // subroutine. Loaded RTLD_GLOBAL so the user libs' RTLD_NOW resolution
        // of the exported names finds them.
        let trampoline_failed = !self.load_dpi_export_trampoline();
        // Each library is opened RTLD_NOW | RTLD_GLOBAL, so its symbols join
        // the global scope and a later --dpi-lib can link against it (#202).
        // A library that needs one listed after it fails its first attempt;
        // the failures are retried while a pass still loads something, so the
        // command-line order does not matter. Symbol lookup keeps that order.
        let mut pending: Vec<(usize, String, String)> = configured_dpi_libs()
            .into_iter()
            .enumerate()
            .map(|(i, p)| (i, p, String::new()))
            .collect();
        let mut loaded: Vec<(usize, Library)> = Vec::new();
        while !pending.is_empty() {
            let before = pending.len();
            let mut failed = Vec::new();
            for (i, path, _) in pending {
                match open_dpi_library(&path) {
                    Ok(lib) => loaded.push((i, lib)),
                    Err(e) => failed.push((i, path, e.to_string())),
                }
            }
            pending = failed;
            if pending.len() == before {
                break;
            }
        }
        loaded.sort_by_key(|(i, _)| *i);
        self.dpi_libraries
            .extend(loaded.into_iter().map(|(_, lib)| lib));
        // A library that cannot be loaded stops the run before it starts, as
        // in the reference simulator (#201): continuing would turn every call
        // into it into a silent no-op.
        for (_, path, e) in pending {
            // A library that calls an exported SV subroutine needs the
            // trampoline; when that failed to build, say so — the loader's
            // own message only names the missing symbol.
            let hint = if trampoline_failed {
                " (the DPI export trampoline could not be built, so exported SV \
                 subroutines are unresolved; see the [DPI] message above)"
            } else {
                ""
            };
            self.compile_errors.push(format!(
                "--dpi-lib '{}' could not be loaded: {}{}",
                path, e, hint
            ));
        }
    }

    pub(super) fn bind_all_dpi_imports(&mut self) {
        for (sv_name, spec) in self.module.dpi_imports.clone() {
            self.try_bind_dpi(&sv_name, &spec);
        }
    }

    /// Internal typedef-chain resolver for DataType (cloned).
    /// Follows TypeReference chains to get the underlying type.
    /// `resolve_type_ref` as a borrow (same chain and bound).
    pub(super) fn resolve_type_ref_borrowed<'a>(
        dt: &'a DataType,
        typedef_types: &'a HashMap<String, DataType>,
    ) -> &'a DataType {
        let mut cur = dt;
        for _ in 0..64 {
            match cur {
                DataType::TypeReference { name, .. } => {
                    if let Some(next) = typedef_types.get(&name.name.name) {
                        cur = next;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        cur
    }

    pub(super) fn resolve_type_ref(
        dt: &DataType,
        typedef_types: &HashMap<String, DataType>,
    ) -> DataType {
        let mut cur = dt.clone();
        for _ in 0..64 {
            match &cur {
                DataType::TypeReference { name, .. } => {
                    if let Some(next) = typedef_types.get(&name.name.name) {
                        cur = next.clone();
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        cur
    }

    /// Returns `Some((width, signed))` when `dt` — either directly or through a
    /// typedef/type-parameter chain — denotes a SCALAR INTEGRAL type whose
    /// width differs from the caller's actual (e.g. `input int` into a formal
    /// declared as the 64-bit typedef `uvm_integral_t`). Returns `None` for
    /// real / string / chandle / struct / class / assoc-array / void so those
    /// formals keep the caller value untouched (mirrors the direct integral
    /// formal handling that a raw `TypeReference` port bypassed).
    pub(super) fn scalar_formal_integral(&self, dt: &DataType) -> Option<(u32, bool)> {
        use crate::ast::types::DataType as DT;
        // Enum and real stay out: enum formals were never width-adapted and a
        // real typedef must round (not resize) — keep the prior behavior.
        let resolved = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let is_resolved_int =
            matches!(&resolved, DT::IntegerVector { .. } | DT::IntegerAtom { .. })
                || matches!(
                    &resolved,
                    DT::Implicit { dimensions, .. } if !dimensions.is_empty()
                );
        if !is_resolved_int {
            return None;
        }
        // §7.4.1/§6.18: the named typedef width includes packed dimensions
        // on every alias in the chain, not just its terminal element type.
        let named_width = matches!(dt, DT::TypeReference { name, dimensions, .. }
            if !dimensions.is_empty()
                || super::elaborate::lookup_scoped_type(name, &self.module.typedefs)
                    .is_some_and(|width| *width > 0));
        let width_type = if named_width { dt } else { resolved };
        let pw = super::elaborate::resolve_type_width(
            width_type,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        if pw == 0 {
            return None;
        }
        let signed =
            super::elaborate::is_type_signed_resolved(&resolved, &self.module.typedef_types);
        Some((pw, signed))
    }

    /// §13.5/§10.8: scalar formals own their declared width and signing
    /// before the first frame-local store. Ref/aggregate binding stays with
    /// its identity/member binders; unresolved class types keep their policy.
    pub(super) fn fit_scalar_formal_seed(
        &self,
        port: &crate::ast::decl::FunctionPort,
        mut value: Value,
    ) -> Value {
        if !port.dimensions.is_empty() {
            return value;
        }
        let dt = &port.data_type;
        let integral = match dt {
            DataType::IntegerAtom { .. } | DataType::IntegerVector { .. } => {
                self.scalar_formal_integral(dt)
            }
            DataType::Implicit { dimensions, .. } if !dimensions.is_empty() => {
                self.scalar_formal_integral(dt)
            }
            DataType::TypeReference {
                name, type_args, ..
            } if type_args.is_empty()
                && !name.has_specialized_scope()
                && !self.typeref_names_class(dt)
                && self.local_type_param_concrete(dt).is_none()
                && !super::elaborate::lookup_scoped_type(
                    name,
                    &self.module.typedef_unpacked_dims,
                )
                .is_some_and(|dims| !dims.is_empty()) =>
            {
                self.scalar_formal_integral(dt)
            }
            _ => None,
        };
        if let Some((width, signed)) = integral {
            if value.is_real {
                value = Self::real_to_int(value.to_f64(), width.max(1));
            } else if width > 0 && width != value.width {
                value = value.resize_for_assign(width);
            }
            value.is_signed = signed;
            if super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
                && value.has_xz()
            {
                value = value.to_two_state();
            }
        } else if super::elaborate::is_type_real(dt) && !value.is_real {
            value = Value::from_f64(value.to_f64());
        }
        value
    }

    /// Given a resolved struct DataType, return a list of (field_name, bit_offset, width)
    /// for each member. Walks members in reverse (last member at LSB, matching SV packing).
    /// Returns `(field_name, bit_offset, width, is_real)` per member.
    /// Last member at LSB (matches SV packing).
    /// The first unpacked-struct typedef (in `typedef_types` order) that
    /// is `width` bits wide and has `member`: its (offset, width, is_real).
    /// The layouts are built once — resolving every typedef on each lookup
    /// cost a constraint solver's search seconds per randomize call.
    pub(super) fn typedef_layout_member(
        &mut self,
        width: u32,
        member: &str,
    ) -> Option<(u32, u32, bool)> {
        if self.typedef_layouts_by_width.is_none() {
            let mut by_w: HashMap<u32, Vec<Vec<(String, u32, u32, bool)>>> = HashMap::default();
            for td in self.module.typedef_types.values() {
                let resolved = Self::resolve_type_ref(td, &self.module.typedef_types);
                if let Some(fields) = Self::struct_field_layout(&resolved) {
                    let total: u32 = fields.iter().map(|f| f.2).sum();
                    by_w.entry(total).or_default().push(fields);
                }
            }
            self.typedef_layouts_by_width = Some(by_w);
        }
        self.typedef_layouts_by_width
            .as_ref()?
            .get(&width)?
            .iter()
            .find_map(|fields| {
                fields
                    .iter()
                    .find(|(m, ..)| m == member)
                    .map(|&(_, off, w, fr)| (off, w, fr))
            })
    }

    pub(super) fn struct_field_layout(dt: &DataType) -> Option<Vec<(String, u32, u32, bool)>> {
        let su = match dt {
            DataType::Struct(su) if !su.packed => su,
            _ => return None,
        };
        let mut fields = Vec::new();
        let mut offset: u32 = 0;
        for m in su.members.iter().rev() {
            let fw = resolve_type_width(&m.data_type, None, None);
            let fr = super::elaborate::is_type_real(&m.data_type);
            for d in &m.declarators {
                fields.push((d.name.name.clone(), offset, fw, fr));
                offset += fw;
            }
        }
        Some(fields)
    }

    /// The canonical C element (H.7) of an unpacked array of `dt`, typedefs
    /// resolved: an enum is its base type (§35.5.6), a packed struct its
    /// packed vector, a scalar `bit`/`logic` one svBit/svLogic byte, and the
    /// 4-state atoms `integer`/`time` svLogicVecVal words.
    pub(super) fn dpi_oa_elem(&self, dt: &DataType) -> Option<DpiOaElem> {
        let dt = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let w = super::elaborate::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        Some(match dt {
            DataType::IntegerAtom { kind, .. } => match kind {
                IntegerAtomType::Byte => DpiOaElem::I8,
                IntegerAtomType::ShortInt => DpiOaElem::I16,
                IntegerAtomType::Int => DpiOaElem::I32,
                IntegerAtomType::LongInt => DpiOaElem::I64,
                IntegerAtomType::Integer => DpiOaElem::Logic(32),
                IntegerAtomType::Time => DpiOaElem::Logic(64),
            },
            DataType::Real { kind, .. } => match kind {
                crate::ast::types::RealType::ShortReal => DpiOaElem::F32,
                _ => DpiOaElem::F64,
            },
            DataType::IntegerVector {
                kind, dimensions, ..
            } => {
                let two = matches!(kind, crate::ast::types::IntegerVectorType::Bit);
                match (dimensions.is_empty(), two) {
                    (true, true) => DpiOaElem::SBit,
                    (true, false) => DpiOaElem::SLogic,
                    (false, true) => DpiOaElem::Bit(w),
                    (false, false) => DpiOaElem::Logic(w),
                }
            }
            // An implicitly typed element is a `logic` (§23.2.2.3).
            DataType::Implicit { dimensions, .. } if dimensions.is_empty() => DpiOaElem::SLogic,
            DataType::Implicit { .. } => DpiOaElem::Logic(w),
            DataType::Struct(su) if su.packed && w > 0 => {
                if self.packed_aggregate_two_state(su) {
                    DpiOaElem::Bit(w)
                } else {
                    DpiOaElem::Logic(w)
                }
            }
            DataType::Enum(e) => match e.base_type.as_deref() {
                Some(bt) => return self.dpi_oa_elem(bt),
                None => DpiOaElem::I32,
            },
            _ => return None,
        })
    }

    pub(super) fn dpi_atom_kind(
        &self,
        dt: &DataType,
        dims: &[crate::ast::types::UnpackedDimension],
        dir: PortDirection,
    ) -> Option<DpiArgKind> {
        // Resolve typedefs (e.g. `hdl_data_t` -> `logic [1023:0]`) so
        // that DPI imports using named types map to the right arg kind.
        let dt = super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types);
        let out_dir = matches!(
            dir,
            PortDirection::Output | PortDirection::Ref | PortDirection::Inout
        );
        // §35.5.6: an unpacked struct is passed as `const T*` / `T*`, T the C
        // struct of its members (H.7.6). It mapped to no kind, so the import
        // was left unbound.
        if let DataType::Struct(su) = dt {
            if !su.packed {
                // Not a layout C can take (an open array of structs, a
                // member DPI does not carry): unsupported, not a zeroed copy.
                self.dpi_c_type(dt, dims, false).ok()?;
                return Some(if out_dir {
                    DpiArgKind::StructOut
                } else {
                    DpiArgKind::StructIn
                });
            }
        }
        if !dims.is_empty() {
            // §35.5.6.1/H.7: an open array's elements take their canonical
            // C form — a `byte` array is `char`s, not `int`s.
            let elem = self.dpi_oa_elem(dt)?;
            // §35.5.6.1 / H.12: a formal with several unpacked dimensions
            // (`int a[][]`) is one handle over the whole array, row-major,
            // with a shape per dimension — up to the header's four.
            if dims.is_empty() || dims.len() > 4 {
                return None;
            }
            return Some(if out_dir {
                DpiArgKind::OpenArrayOut(elem)
            } else {
                DpiArgKind::OpenArrayIn(elem)
            });
        }
        match dt {
            DataType::IntegerAtom { kind, .. } => match kind {
                IntegerAtomType::Int | IntegerAtomType::Byte | IntegerAtomType::ShortInt => {
                    Some(if out_dir {
                        DpiArgKind::Int32Out
                    } else {
                        DpiArgKind::Int32In
                    })
                }
                IntegerAtomType::LongInt => Some(if out_dir {
                    DpiArgKind::Int64Out
                } else {
                    DpiArgKind::Int64In
                }),
                // §35.5.6 / H.7.4: `integer` and `time` are 4-state packed
                // vectors (`logic signed [31:0]`, `logic [63:0]`), so an
                // svLogicVecVal* like any packed logic. They went by value
                // as an int / long long, and C reading the svLogicVecVal*
                // its prototype names dereferenced the value and crashed.
                IntegerAtomType::Integer => Some(if out_dir {
                    DpiArgKind::VecLogicOut(32)
                } else {
                    DpiArgKind::VecLogicIn(32)
                }),
                IntegerAtomType::Time => Some(if out_dir {
                    DpiArgKind::VecLogicOut(64)
                } else {
                    DpiArgKind::VecLogicIn(64)
                }),
            },
            DataType::IntegerVector {
                kind, dimensions, ..
            } => {
                if dimensions.is_empty() {
                    // §35.5.6.1: scalar `bit` is svBit (two-state), scalar
                    // `logic`/`reg` is svLogic and carries x and z.
                    let is_2state = matches!(kind, crate::ast::types::IntegerVectorType::Bit);
                    Some(match (out_dir, is_2state) {
                        (true, true) => DpiArgKind::BitOut,
                        (false, true) => DpiArgKind::BitIn,
                        (true, false) => DpiArgKind::LogicOut,
                        (false, false) => DpiArgKind::LogicIn,
                    })
                } else {
                    let w = super::elaborate::resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    // IEEE 1800-2017 Annex H.10.2: a PACKED array `bit [N]` /
                    // `logic [N]` maps to svBitVecVal* / svLogicVecVal* — a
                    // POINTER — for EVERY width, not only > 64 bits. (The
                    // scalar atoms int/byte/shortint/longint pass by value via
                    // their own arms.) Passing a packed vector by value where
                    // the C dereferences the svBitVecVal* pointer crashed.
                    // 2-state `bit` -> svBitVecVal* (plain uint32_t*).
                    // 4-state `logic`/`reg` -> svLogicVecVal* ({aval, bval}*).
                    let is_2state = matches!(kind, crate::ast::types::IntegerVectorType::Bit);
                    Some(match (out_dir, is_2state) {
                        (true, true) => DpiArgKind::VecBitOut(w),
                        (true, false) => DpiArgKind::VecLogicOut(w),
                        (false, true) => DpiArgKind::VecBitIn(w),
                        (false, false) => DpiArgKind::VecLogicIn(w),
                    })
                }
            }
            // An implicitly typed scalar formal is a `logic` (§23.2.2.3), so
            // an svLogic (§35.5.6.1).
            DataType::Implicit { dimensions, .. } if dimensions.is_empty() => Some(if out_dir {
                DpiArgKind::LogicOut
            } else {
                DpiArgKind::LogicIn
            }),
            DataType::Implicit { dimensions: _, .. } => {
                // Implicitly-typed with a packed range (`[N:0] x`) is a 4-state
                // packed vector -> svLogicVecVal* pointer at every width
                // (Annex H.10.2), same as an explicit `logic [N:0]`.
                let w = super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                Some(if out_dir {
                    DpiArgKind::VecLogicOut(w)
                } else {
                    DpiArgKind::VecLogicIn(w)
                })
            }
            DataType::Real { kind, .. } => match kind {
                crate::ast::types::RealType::ShortReal => Some(if out_dir {
                    DpiArgKind::Real32Out
                } else {
                    DpiArgKind::Real32In
                }),
                _ => Some(if out_dir {
                    DpiArgKind::Real64Out
                } else {
                    DpiArgKind::Real64In
                }),
            },
            // IEEE 1800-2023 §35.5.6 / H.7.7: a PACKED struct or union is
            // passed like the packed array of its bits — `svBitVecVal*` when
            // every member is 2-state, else `svLogicVecVal*` — for any width
            // and direction. It mapped to no kind, the whole import was
            // marked unsupported, and every call returned 0 without
            // reaching the C side.
            DataType::Struct(su) if su.packed => {
                let w = super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                if w == 0 {
                    return None;
                }
                let is_2state = self.packed_aggregate_two_state(su);
                Some(match (out_dir, is_2state) {
                    (true, true) => DpiArgKind::VecBitOut(w),
                    (true, false) => DpiArgKind::VecLogicOut(w),
                    (false, true) => DpiArgKind::VecBitIn(w),
                    (false, false) => DpiArgKind::VecLogicIn(w),
                })
            }
            // §35.5.6: an enum passes as its base type (`int` by default).
            DataType::Enum(e) => match e.base_type.as_deref() {
                Some(bt) => self.dpi_atom_kind(bt, dims, dir),
                None => Some(if out_dir {
                    DpiArgKind::Int32Out
                } else {
                    DpiArgKind::Int32In
                }),
            },
            DataType::Simple { kind, .. } => match kind {
                crate::ast::types::SimpleType::Chandle => Some(if out_dir {
                    DpiArgKind::ChandleOut
                } else {
                    DpiArgKind::ChandleIn
                }),
                crate::ast::types::SimpleType::String => Some(if out_dir {
                    DpiArgKind::StringOut
                } else {
                    DpiArgKind::StringIn
                }),
                _ => None,
            },
            _ => None,
        }
    }

    /// §6.11 / §7.2.1: a packed struct or union is 2-state only when every
    /// member (through typedefs and nested packed aggregates) is.
    pub(super) fn packed_aggregate_two_state(
        &self,
        su: &crate::ast::types::StructUnionType,
    ) -> bool {
        su.members.iter().all(|m| {
            let mdt =
                super::elaborate::resolve_typedef_chain(&m.data_type, &self.module.typedef_types);
            match mdt {
                DataType::Struct(inner) => self.packed_aggregate_two_state(inner),
                _ => super::elaborate::is_type_two_state_resolved(mdt, &self.module.typedef_types),
            }
        })
    }

    pub(super) fn dpi_return_kind(&self, dt: &DataType) -> Option<DpiRetKind> {
        match dt {
            DataType::Void(_) => Some(DpiRetKind::Void),
            DataType::IntegerAtom { kind, .. } => match kind {
                IntegerAtomType::Int
                | IntegerAtomType::Integer
                | IntegerAtomType::Byte
                | IntegerAtomType::ShortInt => Some(DpiRetKind::Int32),
                IntegerAtomType::LongInt | IntegerAtomType::Time => Some(DpiRetKind::Int64),
            },
            DataType::IntegerVector {
                kind, dimensions, ..
            } => {
                if !dimensions.is_empty() {
                    Some(DpiRetKind::Int64)
                } else if matches!(kind, crate::ast::types::IntegerVectorType::Bit) {
                    Some(DpiRetKind::Int32)
                } else {
                    Some(DpiRetKind::Logic)
                }
            }
            DataType::Implicit { dimensions, .. } if dimensions.is_empty() => {
                Some(DpiRetKind::Logic)
            }
            DataType::Real { kind, .. } => match kind {
                crate::ast::types::RealType::ShortReal => Some(DpiRetKind::Real32),
                _ => Some(DpiRetKind::Real64),
            },
            DataType::Simple { kind, .. } => match kind {
                crate::ast::types::SimpleType::Chandle => Some(DpiRetKind::Chandle),
                crate::ast::types::SimpleType::String => Some(DpiRetKind::String),
                _ => None,
            },
            // §35.5.6: an enum is interpreted as its base type (`int` when
            // none is given), so an enum result is whatever its base type's
            // result is. It had no arm and the import was left unbound.
            DataType::Enum(e) => match e.base_type.as_deref() {
                Some(bt) => self.dpi_return_kind(&self.dpi_resolve_typedef(bt)),
                None => Some(DpiRetKind::Int32),
            },
            _ => None,
        }
    }

    /// The (width, signed) an integral imported-function result is fitted
    /// to: its declared type's, or, for an enum, its base type's (§35.5.6),
    /// `int` when none is given.
    pub(super) fn dpi_result_integral(&self, dt: &DataType) -> Option<(u32, bool)> {
        match self.dpi_resolve_typedef(dt) {
            DataType::Enum(e) => match e.base_type.as_deref() {
                Some(bt) => self.scalar_formal_integral(bt),
                None => Some((32, true)),
            },
            _ => self.scalar_formal_integral(dt),
        }
    }

    /// A typedef-named DPI return type (`typedef int I; import "DPI-C"
    /// function I f();`), resolved to the type it names so it maps to the
    /// same C type. A typedef name with packed dimensions (`M [1:0]`) is a
    /// packed array, which §35.5.5 does not allow as a result; it stays
    /// unresolved and the prototype is reported unsupported.
    pub(super) fn dpi_resolve_typedef(&self, dt: &DataType) -> DataType {
        let mut cur = dt.clone();
        for _ in 0..16 {
            let DataType::TypeReference {
                name, dimensions, ..
            } = &cur
            else {
                break;
            };
            if !dimensions.is_empty() {
                break;
            }
            // §26.3: a package-qualified result type (`pkg::color_e`) names
            // the package's typedef.
            match super::elaborate::lookup_scoped_type(name, &self.module.typedef_types) {
                Some(t) => cur = t.clone(),
                None => break,
            }
        }
        cur
    }

    pub(super) fn dpi_signature(
        &self,
        spec: &DpiImportSpec,
    ) -> Option<(DpiRetKind, Vec<DpiArgKind>)> {
        match &spec.proto {
            crate::ast::decl::DPIProto::Function(fd) => {
                let ret = self.dpi_return_kind(&self.dpi_resolve_typedef(&fd.return_type))?;
                let mut args = Vec::with_capacity(fd.ports.len());
                for p in &fd.ports {
                    args.push(self.dpi_atom_kind(&p.data_type, &p.dimensions, p.direction)?);
                }
                Some((ret, args))
            }
            crate::ast::decl::DPIProto::Task(td) => {
                let mut args = Vec::with_capacity(td.ports.len());
                for p in &td.ports {
                    args.push(self.dpi_atom_kind(&p.data_type, &p.dimensions, p.direction)?);
                }
                // §35.9: an imported task's C function returns int, 1 when
                // it returns because of a disable (checked by the
                // disable protocol, dpi_task.rs).
                Some((DpiRetKind::Int32, args))
            }
        }
    }

    pub(super) fn try_bind_dpi(&mut self, sv_name: &str, spec: &DpiImportSpec) {
        if self.dpi_bindings.contains_key(sv_name) || self.dpi_unsupported.contains(sv_name) {
            return;
        }
        // LRM §35.5.3: a DPI import declared `pure` must (a) return non-void
        // and (b) have ONLY `input` ports — no output/inout/ref. A `pure`
        // task is illegal. Catch the violations at bind time so a misdeclared
        // C function fails loudly here rather than silently dropping outputs.
        if spec.property == Some(crate::ast::decl::DPIProperty::Pure) {
            let (is_task, ports, has_non_void_ret) = match &spec.proto {
                crate::ast::decl::DPIProto::Function(f) => (
                    false,
                    &f.ports,
                    !matches!(&f.return_type, crate::ast::types::DataType::Void { .. }),
                ),
                crate::ast::decl::DPIProto::Task(t) => (true, &t.ports, false),
            };
            if is_task {
                eprintln!(
                    "[DPI] '{}' declared `pure` but is a task; LRM §35.5.3 requires a function",
                    sv_name
                );
                self.dpi_unsupported.insert(sv_name.to_string());
                return;
            }
            if !has_non_void_ret {
                eprintln!(
                    "[DPI] '{}' declared `pure` but returns void; LRM §35.5.3",
                    sv_name
                );
                self.dpi_unsupported.insert(sv_name.to_string());
                return;
            }
            for p in ports {
                if !matches!(p.direction, crate::ast::types::PortDirection::Input) {
                    eprintln!(
                        "[DPI] '{}' declared `pure` but port '{}' is {:?}; pure imports may only have input ports (LRM §35.5.3)",
                        sv_name, p.name.name, p.direction
                    );
                    self.dpi_unsupported.insert(sv_name.to_string());
                    return;
                }
            }
        }
        let Some((ret, arg_kinds)) = self.dpi_signature(spec) else {
            self.dpi_unsupported.insert(sv_name.to_string());
            eprintln!("[DPI] unsupported prototype for '{}'", sv_name);
            return;
        };
        let mut cname = spec.c_name.clone().into_bytes();
        cname.push(0);
        let arg_types: Vec<Type> = arg_kinds
            .iter()
            .map(|k| match k {
                DpiArgKind::Int32In => Type::i32(),
                DpiArgKind::Int32Out => Type::pointer(),
                DpiArgKind::LogicIn => Type::u8(),
                DpiArgKind::LogicOut => Type::pointer(),
                DpiArgKind::BitIn => Type::u8(),
                DpiArgKind::BitOut => Type::pointer(),
                DpiArgKind::Int64In => Type::i64(),
                DpiArgKind::Int64Out => Type::pointer(),
                DpiArgKind::Real32In => Type::f32(),
                DpiArgKind::Real32Out => Type::pointer(),
                DpiArgKind::Real64In => Type::f64(),
                DpiArgKind::Real64Out => Type::pointer(),
                DpiArgKind::ChandleIn => Type::pointer(),
                DpiArgKind::ChandleOut => Type::pointer(),
                DpiArgKind::StringIn => Type::pointer(),
                DpiArgKind::StringOut => Type::pointer(),
                DpiArgKind::OpenArrayIn(_) | DpiArgKind::OpenArrayOut(_) => Type::pointer(),
                DpiArgKind::VecLogicIn(_) => Type::pointer(),
                DpiArgKind::VecLogicOut(_) => Type::pointer(),
                DpiArgKind::VecBitIn(_) => Type::pointer(),
                DpiArgKind::VecBitOut(_) => Type::pointer(),
                DpiArgKind::StructIn | DpiArgKind::StructOut => Type::pointer(),
            })
            .collect();
        let ret_type = match ret {
            DpiRetKind::Void => Type::void(),
            DpiRetKind::Int32 => Type::i32(),
            DpiRetKind::Logic => Type::u8(),
            DpiRetKind::Int64 => Type::i64(),
            DpiRetKind::Real32 => Type::f32(),
            DpiRetKind::Real64 => Type::f64(),
            DpiRetKind::Chandle => Type::pointer(),
            DpiRetKind::String => Type::pointer(),
        };
        let cif = Cif::new(arg_types, ret_type);
        for lib in &self.dpi_libraries {
            let sym = unsafe { lib.get::<*mut c_void>(cname.as_slice()) };
            if let Ok(s) = sym {
                self.dpi_bindings.insert(
                    sv_name.to_string(),
                    DpiBinding {
                        ret,
                        arg_kinds: arg_kinds.clone(),
                        cif: cif.clone(),
                        fn_ptr: CodePtr::from_ptr(*s),
                    },
                );
                return;
            }
        }
    }

    /// §35.5.6.1 / H.7.4: the svLogic code of a scalar 4-state value —
    /// sv_0 = 0, sv_1 = 1, sv_z = 2, sv_x = 3.
    pub(super) fn dpi_svlogic_code(v: &Value) -> u8 {
        match v.get_bit(0) {
            LogicBit::Zero => 0,
            LogicBit::One => 1,
            LogicBit::Z => 2,
            LogicBit::X => 3,
        }
    }

    /// The 1-bit SV value of an svLogic the C side wrote or returned. Only
    /// the two low bits are defined (H.7.4).
    pub(super) fn dpi_svlogic_value(code: u8) -> Value {
        let mut v = Value::new(1);
        v.set_bit(
            0,
            match code & 3 {
                0 => LogicBit::Zero,
                1 => LogicBit::One,
                2 => LogicBit::Z,
                _ => LogicBit::X,
            },
        );
        v
    }

    pub(super) fn dpi_value_to_logic_words(v: &Value, width: u32) -> (Vec<u32>, Vec<u32>) {
        let nwords = width.div_ceil(32).max(1) as usize;
        let mut aval = vec![0u32; nwords];
        let mut bval = vec![0u32; nwords];
        for bit in 0..(width as usize) {
            let w = bit / 32;
            let m = 1u32 << (bit % 32);
            match v.get_bit(bit) {
                // §35.5.5 encoding: 0=(a0,b0) 1=(a1,b0) Z=(a0,b1) X=(a1,b1).
                // (Matches VPI's bit_code_to_ab and the shipped header — DPI
                // previously had X and Z swapped.)
                LogicBit::Zero => {}
                LogicBit::One => aval[w] |= m,
                LogicBit::Z => bval[w] |= m,
                LogicBit::X => {
                    aval[w] |= m;
                    bval[w] |= m;
                }
            }
        }
        (aval, bval)
    }

    /// Value -> the STANDARD svLogicVecVal buffer: an array of `s_vpi_vecval`
    /// {aval, bval} pairs, i.e. INTERLEAVED u32 words [a0, b0, a1, b1, ...]
    /// (§35.5.5 / Annex H.10.2). This is the layout xezim's shipped svdpi.h
    /// declares and what standard DPI code expects.
    pub(super) fn dpi_value_to_logic_interleaved(v: &Value, width: u32) -> Vec<u32> {
        let (aval, bval) = Self::dpi_value_to_logic_words(v, width);
        let n = aval.len();
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            out.push(aval[i]);
            out.push(bval[i]);
        }
        out
    }

    /// The inverse: an interleaved [a0, b0, a1, b1, ...] buffer -> Value.
    pub(super) fn dpi_logic_interleaved_to_value(buf: &[u32], width: u32) -> Value {
        let n = buf.len() / 2;
        let mut aval = Vec::with_capacity(n);
        let mut bval = Vec::with_capacity(n);
        for i in 0..n {
            aval.push(buf[2 * i]);
            bval.push(buf[2 * i + 1]);
        }
        Self::dpi_logic_words_to_value(&aval, &bval, width)
    }

    pub(super) fn dpi_logic_words_to_value(aval: &[u32], bval: &[u32], width: u32) -> Value {
        let mut out = Value::zero(width.max(1));
        for bit in 0..(width as usize) {
            let w = bit / 32;
            let m = 1u32 << (bit % 32);
            let a = (aval.get(w).copied().unwrap_or(0) & m) != 0;
            let b = (bval.get(w).copied().unwrap_or(0) & m) != 0;
            let lb = match (a, b) {
                (false, false) => LogicBit::Zero,
                (true, false) => LogicBit::One,
                (false, true) => LogicBit::Z,
                (true, true) => LogicBit::X,
            };
            out.set_bit(bit, lb);
        }
        out.resize(width)
    }

    /// The elements of an open-array actual and its declared bounds
    /// (`left`, `right`): a fixed unpacked array (`[7:0]` gives 7, 0), a
    /// dynamic array or queue (0 .. size-1, empty when size is 0), or any
    /// other expression as a one-element array.
    pub(super) fn dpi_collect_open_array(
        &mut self,
        expr: Option<&Expression>,
    ) -> (Vec<Value>, Vec<(i64, i64)>) {
        let Some(e) = expr else {
            return (Vec::new(), vec![(0, -1)]);
        };
        if let ExprKind::Ident(hier) = &e.kind {
            let name = self.resolve_hier_name(hier);
            // §35.5.6.1: a multi-dimensional actual (`int a[2][3]`) — every
            // element, row-major.
            if let Some(shape) = self.dpi_multi_dim_shape(&name) {
                let elem_w = self.dpi_multi_dim_elem_width(&name);
                let out = Self::dpi_multi_dim_names(&name, &shape)
                    .into_iter()
                    .map(|en| {
                        self.get_signal_value_by_name(&en)
                            .unwrap_or_else(|| Value::zero(elem_w))
                    })
                    .collect();
                return (out, shape);
            }
            // A dynamic array or queue also has a `module.arrays` entry, but
            // its range is storage capacity: the live size bounds it.
            let live = self.dpi_open_array_live_size(&name);
            if let Some((lo, hi, elem_w)) = self.module.arrays.get(&*name).copied() {
                let hi = live.map_or(hi, |n| lo + n as i64 - 1);
                let mut out = Vec::with_capacity((hi - lo + 1).max(0) as usize);
                // Compact-resolver fast path: if the array is in
                // array_first_id, walk the contiguous id range with no
                // per-element name allocation.
                if let Some(&(first_id, _, _)) = self.array_first_id.get(name.as_ref()) {
                    for off in 0..(hi - lo + 1).max(0) as usize {
                        out.push(self.cell_read(first_id + off));
                    }
                } else {
                    for idx in lo..=hi {
                        let elem_name = format!("{}[{}]", name, idx);
                        let vv = if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                            self.signal_table[id].clone()
                        } else {
                            self.signals
                                .get(&elem_name)
                                .cloned()
                                .unwrap_or_else(|| Value::zero(elem_w.max(1) as u32))
                        };
                        out.push(vv);
                    }
                }
                let (left, right) =
                    if live.is_none() && self.module.descending_arrays.contains(&*name) {
                        (hi, lo)
                    } else {
                        (lo, hi)
                    };
                return (out, vec![(left, right)]);
            }
            if let Some(n) = live {
                let out = (0..n)
                    .map(|i| {
                        self.get_signal_value_by_name(&format!("{}[{}]", name, i))
                            .unwrap_or_else(|| Value::zero(32))
                    })
                    .collect();
                return (out, vec![(0, n as i64 - 1)]);
            }
        }
        // §10.9.1: an assignment pattern (`'{1, 2, 3}`, `'{'{1, 2}, '{3, 4}}`)
        // is an unpacked array value — every element, not just the last.
        if let Some(r) = self.dpi_collect_pattern(e) {
            return r;
        }
        (vec![self.eval_expr(e)], vec![(0, 0)])
    }

    /// The elements (row-major) and shape of a positional assignment
    /// pattern used as an unpacked-array actual; None for anything else.
    pub(super) fn dpi_collect_pattern(
        &mut self,
        e: &Expression,
    ) -> Option<(Vec<Value>, Vec<(i64, i64)>)> {
        let ExprKind::AssignmentPattern(items) = &e.kind else {
            return None;
        };
        if items.is_empty()
            || !items
                .iter()
                .all(|it| matches!(it, crate::ast::expr::AssignmentPatternItem::Ordered(_)))
        {
            return None;
        }
        let n = items.len() as i64;
        let nested = items
            .iter()
            .all(|it| matches!(it.expr().kind, ExprKind::AssignmentPattern(_)));
        if nested {
            let mut out = Vec::new();
            let mut inner: Option<Vec<(i64, i64)>> = None;
            for it in items {
                let (vals, shape) = self.dpi_collect_pattern(it.expr())?;
                if inner.as_ref().is_some_and(|s| *s != shape) {
                    return None;
                }
                inner = Some(shape);
                out.extend(vals);
            }
            let mut shape = vec![(0, n - 1)];
            shape.extend(inner.unwrap_or_default());
            return (shape.len() <= 4).then_some((out, shape));
        }
        let out = items.iter().map(|it| self.eval_expr(it.expr())).collect();
        Some((out, vec![(0, n - 1)]))
    }

    /// The `(lo, hi)` of each unpacked dimension of a fixed MULTI-
    /// dimensional array (`int a[2][3]`), outermost first; `None` for a
    /// one-dimensional array or anything else.
    pub(super) fn dpi_multi_dim_shape(&self, name: &str) -> Option<Vec<(i64, i64)>> {
        let shape = self.foreach_dims(name)?;
        (shape.len() > 1 && shape.len() <= 4 && shape.iter().all(|&(lo, hi)| hi >= lo))
            .then_some(shape)
    }

    pub(super) fn dpi_multi_dim_elem_width(&self, name: &str) -> u32 {
        if let Some(&(_, _, w)) = self.module.arrays_2d.get(name) {
            return w.max(1);
        }
        self.module
            .arrays_nd
            .get(name)
            .map_or(32, |(_, w)| (*w).max(1))
    }

    /// Element names of a multi-dimensional array, row-major, each
    /// dimension in ascending index order (`a[0][0]`, `a[0][1]`, ...).
    pub(super) fn dpi_multi_dim_names(name: &str, shape: &[(i64, i64)]) -> Vec<String> {
        let mut names = vec![name.to_string()];
        for &(lo, hi) in shape {
            let mut next = Vec::with_capacity(names.len() * (hi - lo + 1) as usize);
            for n in &names {
                for i in lo..=hi {
                    next.push(format!("{}[{}]", n, i));
                }
            }
            names = next;
        }
        names
    }

    /// The element count of a dynamic array or queue actual; None for a
    /// fixed-size array.
    pub(super) fn dpi_open_array_live_size(&self, name: &str) -> Option<u64> {
        (self.module.dynamic_arrays.contains(name)
            || self
                .signals
                .contains_key(&Self::name_with_suffix(name, ".size")))
        .then(|| self.get_queue_size(name))
    }

    /// Write an output open array back to its actual, element by element
    /// in the order `dpi_collect_open_array` read them.
    pub(super) fn dpi_writeback_open_array(&mut self, expr: &Expression, data: &[Value]) {
        if let ExprKind::Ident(hier) = &expr.kind {
            let name = self.resolve_hier_name(hier);
            if let Some(shape) = self.dpi_multi_dim_shape(&name) {
                let elem_w = self.dpi_multi_dim_elem_width(&name);
                for (en, d) in Self::dpi_multi_dim_names(&name, &shape).iter().zip(data) {
                    let v = if d.is_real {
                        d.clone()
                    } else {
                        d.resize(elem_w)
                    };
                    self.set_signal_value_by_name(en, v);
                }
                return;
            }
            let live = self.dpi_open_array_live_size(&name);
            if let Some((lo, hi, elem_w)) = self.module.arrays.get(&*name).copied() {
                let hi = live.map_or(hi, |n| lo + n as i64 - 1);
                let elem_w = elem_w.max(1) as u32;
                let fit = |v: &Value| {
                    if v.is_real {
                        v.clone()
                    } else {
                        v.resize(elem_w)
                    }
                };
                // Compact-resolver fast path.
                if let Some(&(first_id, _, _)) = self.array_first_id.get(name.as_ref()) {
                    for (off, d) in data.iter().enumerate().take((hi - lo + 1).max(0) as usize) {
                        let id = first_id + off;
                        let mut val = fit(d);
                        if is_packed_id(id) {
                            self.packed_store_value(id, &val);
                            continue;
                        }
                        val.is_signed = self.signal_signed[id];
                        if self.signal_table[id] != val {
                            self.mark_dirty_id(id);
                            write_sig!(self, id, val);
                            self.table_modified = true;
                        }
                    }
                    return;
                }
                for (idx, d) in (lo..=hi).zip(data) {
                    let elem_name = format!("{}[{}]", name, idx);
                    let mut val = fit(d);
                    if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                        val.is_signed = self.signal_signed[id];
                        if self.signal_table[id] != val {
                            self.mark_dirty_id(id);
                            write_sig!(self, id, val);
                            self.table_modified = true;
                        }
                    } else {
                        if self.signed_signals.contains(&elem_name) {
                            val.is_signed = true;
                        }
                        let changed = self.signals.get(&elem_name).is_none_or(|p| *p != val);
                        if changed {
                            self.mark_dirty(&elem_name);
                        }
                        self.signals.insert(elem_name, val);
                    }
                }
                return;
            }
            if let Some(n) = live {
                for (i, d) in data.iter().enumerate().take(n as usize) {
                    let elem_name = format!("{}[{}]", name, i);
                    let mut val = d.clone();
                    if let Some(old) = self.get_signal_value_by_name(&elem_name) {
                        if !val.is_real && !old.is_real {
                            val = val.resize(old.width);
                            val.is_signed = old.is_signed;
                        }
                    }
                    self.set_signal_value_by_name(&elem_name, val);
                }
                return;
            }
        }
        if let Some(v0) = data.first() {
            let w = self.infer_lhs_width(expr);
            let v = if v0.is_real {
                v0.clone()
            } else {
                v0.resize(w as u32)
            };
            self.assign_value(expr, &v);
        }
    }

    /// §35.5.3: the scope a context import called without an instance
    /// prefix runs in — where its declaration is. An import declared only in
    /// a package runs in that package wherever it is called from. One
    /// declared in a design scope runs in the scope of the code calling it:
    /// the declaring scope of the running subroutine when that was declared
    /// in another instance, interface, generate block or package (`u.f()`,
    /// `pkg::f()`), else the running process's instance. Every such call ran
    /// in the calling process's instance, so `svGetScope` in an import called
    /// from a package, interface or generate-block function named the module.
    pub(super) fn dpi_context_scope(&self, sv_name: &str) -> String {
        let pkg = self.module.dpi_import_pkg.get(sv_name);
        let in_design = self.module.dpi_import_in_design.contains(sv_name);
        let root = self
            .m_scope_stack
            .first()
            .and_then(|f| f.strip_prefix(M_ROOT_MARK));
        match (root, pkg) {
            (_, Some(p)) if !in_design => p.clone(),
            (Some(r), Some(p)) if self.module.packages.contains(r) => p.clone(),
            (Some(r), _) => r.to_string(),
            (None, _) => match self.module.pkg_subr_owner.get(sv_name) {
                Some(p) if !in_design => p.clone(),
                _ => self.hier_path(&self.active_instance_scope()),
            },
        }
    }

    /// The actuals of an import call in formal order, named ones bound to
    /// their formals and omitted ones taken from the defaults; None when the
    /// call is already positional and complete.
    pub(super) fn dpi_bind_call_args(
        proto: &crate::ast::decl::DPIProto,
        args: &[Expression],
    ) -> Option<Vec<Expression>> {
        let ports = match proto {
            crate::ast::decl::DPIProto::Function(fd) => &fd.ports,
            crate::ast::decl::DPIProto::Task(td) => &td.ports,
        };
        let mut out = Self::normalize_call_args(ports, args);
        let have = out.as_ref().map_or(args.len(), |v| v.len());
        if have < ports.len() && ports[have].default.is_some() {
            let v = out.get_or_insert_with(|| args.to_vec());
            for p in &ports[have..] {
                match &p.default {
                    Some(d) => v.push(d.clone()),
                    None => break,
                }
            }
        }
        out
    }

    pub(super) fn exec_dpi_import_call(
        &mut self,
        sv_name: &str,
        args: &[Expression],
    ) -> Option<Value> {
        self.exec_dpi_import_call_in(sv_name, args, None)
    }

    /// §35.5.6 / §13.5: an actual bound to an integral DPI formal converts as
    /// an assignment to the formal would, so a real rounds (§6.12.2) instead
    /// of passing its f64 bit pattern (`c_add(2.6, 3.4)` handed C 0s).
    pub(super) fn dpi_int_actual(&mut self, e: &Expression) -> Value {
        self.dpi_int_actual_w(e, 64)
    }

    /// `dpi_int_actual` for a packed-vector formal of `width` bits: a real
    /// fills all of them (`1.0e30` into a `bit [127:0]`).
    pub(super) fn dpi_int_actual_w(&mut self, e: &Expression, width: u32) -> Value {
        let v = self.eval_expr(e);
        if v.is_real {
            Self::real_to_int(v.to_f64(), width.max(64))
        } else {
            v
        }
    }

    /// §23.9 / §26.3: ordinary declarations in the calling scope hide a
    /// wildcard-imported DPI name. A package-qualified call remains explicit.
    pub(super) fn resolve_dpi_call_name<'a>(
        &self,
        sv_name: &'a str,
        via: Option<&str>,
    ) -> Option<std::borrow::Cow<'a, str>> {
        use std::borrow::Cow;
        if sv_name.contains("::") {
            return Some(Cow::Borrowed(sv_name));
        }
        if via.is_none()
            && self
                .class_context_stack
                .last()
                .and_then(|c| c.as_deref())
                .is_some_and(|class| self.class_has_method(class, sv_name))
        {
            return None;
        }
        if via.is_none() {
            if let Some(key) = self
                .m_scope_stack
                .first()
                .and_then(|f| f.strip_prefix(M_ROOT_MARK))
                .filter(|p| self.module.packages.contains(*p))
                .map(|p| format!("{p}::{sv_name}"))
                .filter(|key| self.module.dpi_imports.contains_key(key))
            {
                return Some(Cow::Owned(key));
            }
        }
        let activation = self.activation_scope.borrow();
        let scope = via
            .or(activation.as_deref())
            .unwrap_or(self.current_scope.as_str());
        let key = if scope.is_empty() || sv_name.contains('.') {
            Cow::Borrowed(sv_name)
        } else {
            Cow::Owned(format!("{scope}.{sv_name}"))
        };
        if !self.module.func_decl_scope.contains_key(key.as_ref())
            && (self.module.functions.contains_key(key.as_ref())
                || self.module.tasks.contains_key(key.as_ref()))
        {
            return None;
        }
        Some(Cow::Borrowed(sv_name))
    }

    /// `exec_dpi_import_call` for a call reached through a hierarchical
    /// prefix (`u_a.c_fn()`): `via` is that instance path (`u_a`).
    pub(super) fn exec_dpi_import_call_in(
        &mut self,
        sv_name: &str,
        args: &[Expression],
        via: Option<&str>,
    ) -> Option<Value> {
        let resolved_name = self.resolve_dpi_call_name(sv_name, via)?;
        let sv_name = resolved_name.as_ref();
        // §13.5.3 / §13.5.4: an import call binds its actuals like any
        // subroutine call — `.name(expr)` to the named formal, an omitted
        // one from its default. They were taken by position only, so a
        // named actual bound nothing and an omitted one passed 0.
        let bound_args;
        let args: &[Expression] = match self
            .module
            .dpi_imports
            .get(sv_name)
            .and_then(|spec| Self::dpi_bind_call_args(&spec.proto, args))
        {
            Some(v) => {
                bound_args = v;
                &bound_args
            }
            None => args,
        };
        let prev_active = ACTIVE_SIMULATOR.with(|cell| cell.get());
        if !self.dpi_bindings.contains_key(sv_name) {
            let c_name = self.module.dpi_imports.get(sv_name)?.c_name.clone();
            // Binding is attempted once per import; UVM's regex helpers are
            // called often enough that re-cloning the prototype showed.
            if !self.dpi_unsupported.contains(sv_name) && self.dpi_bind_tried.insert(sv_name.into())
            {
                let spec = self.module.dpi_imports.get(sv_name)?.clone();
                self.try_bind_dpi(sv_name, &spec);
            }
            // Built-in implementations of the UVM distribution's DPI-C
            // helpers (src/compiler/simulator/uvm_dpi.rs), so UVM runs
            // without +define+UVM_NO_DPI. A user-loaded shared library that
            // defines one of these symbols wins — the builtin only serves
            // symbols no library resolved.
            if !self.dpi_bindings.contains_key(sv_name) {
                if let Some(v) = self.exec_uvm_dpi_builtin(&c_name, sv_name, args) {
                    ACTIVE_SIMULATOR.with(|cell| cell.set(prev_active));
                    return Some(v);
                }
            }
        }
        let spec = self.module.dpi_imports.get(sv_name)?.clone();
        if self.dpi_unsupported.contains(sv_name) {
            ACTIVE_SIMULATOR.with(|cell| cell.set(prev_active));
            return Some(Value::zero(32));
        }
        // Foreign code may read what the design wrote to a file.
        self.flush_file_writes();
        let Some(binding) = self.dpi_bindings.get(sv_name) else {
            // Calling an import that no library implements is fatal, as in the
            // reference simulator (#201): returning 0 let a run whose DPI
            // layer never loaded finish and exit 0.
            if self.dpi_unresolved.insert(sv_name.to_string()) {
                let msg = format!(
                    "DPI import '{}' has no implementation: C function '{}' is not in any --dpi-lib library",
                    sv_name, spec.c_name
                );
                self.emit_severity_text("Fatal", &msg);
                self.fatal_finish_number = Some(1);
                self.finished = true;
            }
            ACTIVE_SIMULATOR.with(|cell| cell.set(prev_active));
            return Some(Value::zero(32));
        };
        let ret_kind = binding.ret;
        let arg_kinds = binding.arg_kinds.clone();
        let cif = binding.cif.clone();
        let fn_ptr = binding.fn_ptr;

        enum ArgSlot {
            I32(usize),
            U8(usize),
            I64(usize),
            F32(usize),
            F64(usize),
            Ptr(usize),
        }
        let mut arg_slots = Vec::with_capacity(arg_kinds.len());
        let mut i32_vals: Vec<Box<i32>> = Vec::with_capacity(arg_kinds.len());
        let mut u8_vals: Vec<Box<u8>> = Vec::with_capacity(arg_kinds.len());
        let mut i64_vals: Vec<Box<i64>> = Vec::with_capacity(arg_kinds.len());
        let mut f32_vals: Vec<Box<f32>> = Vec::with_capacity(arg_kinds.len());
        let mut f64_vals: Vec<Box<f64>> = Vec::with_capacity(arg_kinds.len());
        // §35.5: assemble owned argument storage before borrowing it for the
        // native call. Slot indices preserve formal order across typed buffers.
        let mut ptr_vals: Vec<Box<*mut c_void>> = Vec::with_capacity(arg_kinds.len());
        let mut string_ptr_cells: Vec<Box<*const std::ffi::c_char>> =
            Vec::with_capacity(arg_kinds.len());
        let mut open_arrays: Vec<DpiOpenArrayArg> = Vec::with_capacity(arg_kinds.len());
        let mut cstrings: Vec<CString> = Vec::with_capacity(arg_kinds.len());
        let mut logic_aval: Vec<Vec<u32>> = Vec::with_capacity(arg_kinds.len());
        // Unpacked-struct arguments: the C object, its type and the actual's
        // name; strings stored in them stay alive for the call.
        let mut struct_bufs: Vec<(Vec<u64>, Option<dpi_export::DpiCType>, Option<String>)> =
            Vec::new();
        let mut struct_keep: Vec<CString> = Vec::new();
        let mut writebacks: Vec<(usize, DpiArgKind, Expression)> = Vec::new();

        for (i, kind) in arg_kinds.iter().enumerate() {
            match kind {
                DpiArgKind::Int32In => {
                    let v = Box::new(
                        args.get(i)
                            .map(|e| self.dpi_int_actual(e).to_i64().unwrap_or(0) as i32)
                            .unwrap_or(0),
                    );
                    i32_vals.push(v);
                    arg_slots.push(ArgSlot::I32(i32_vals.len() - 1));
                }
                DpiArgKind::LogicIn => {
                    let v = Box::new(
                        args.get(i)
                            .map(|e| Self::dpi_svlogic_code(&self.dpi_int_actual(e)))
                            .unwrap_or(0),
                    );
                    u8_vals.push(v);
                    arg_slots.push(ArgSlot::U8(u8_vals.len() - 1));
                }
                DpiArgKind::LogicOut => {
                    // An inout passes the actual's current x / z in; C writes
                    // one byte. The slot is an i32 so a C side that wrongly
                    // writes an int stays inside it; byte 0 is the svLogic.
                    let code = args
                        .get(i)
                        .map(|e| Self::dpi_svlogic_code(&self.eval_expr(e)))
                        .unwrap_or(0);
                    let b = Box::new(i32::from_ne_bytes([code, 0, 0, 0]));
                    let p = Box::new((&*b as *const i32 as *mut i32).cast::<c_void>());
                    i32_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((i32_vals.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::BitIn => {
                    // A scalar two-state `bit` is a 1-bit value crossed as an
                    // svBit (`unsigned char`): the actual converts as an
                    // assignment to the 1-bit formal would (a real rounds
                    // first), so only bit 0 crosses — not the low 8 bits an
                    // i32 pass-through would send (2 -> svBit 0, not 2).
                    let v = Box::new(
                        args.get(i)
                            .map(|e| (self.dpi_int_actual(e).get_bit(0) == LogicBit::One) as u8)
                            .unwrap_or(0),
                    );
                    u8_vals.push(v);
                    arg_slots.push(ArgSlot::U8(u8_vals.len() - 1));
                }
                DpiArgKind::BitOut => {
                    // inout/out: seed byte 0 with the actual's bit 0; C writes
                    // an svBit byte. The slot is an i32 so a C side that
                    // wrongly writes an int stays inside it.
                    let bit = args
                        .get(i)
                        .map(|e| (self.dpi_int_actual(e).get_bit(0) == LogicBit::One) as u8)
                        .unwrap_or(0);
                    let b = Box::new(i32::from_ne_bytes([bit, 0, 0, 0]));
                    let p = Box::new((&*b as *const i32 as *mut i32).cast::<c_void>());
                    i32_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((i32_vals.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::Int64In => {
                    let v = Box::new(
                        args.get(i)
                            .map(|e| self.dpi_int_actual(e).to_i64().unwrap_or(0))
                            .unwrap_or(0),
                    );
                    i64_vals.push(v);
                    arg_slots.push(ArgSlot::I64(i64_vals.len() - 1));
                }
                DpiArgKind::Real32In => {
                    let v = Box::new(
                        args.get(i)
                            .map(|e| self.eval_expr(e).to_f64() as f32)
                            .unwrap_or(0.0),
                    );
                    f32_vals.push(v);
                    arg_slots.push(ArgSlot::F32(f32_vals.len() - 1));
                }
                DpiArgKind::Real64In => {
                    let v = Box::new(
                        args.get(i)
                            .map(|e| self.eval_expr(e).to_f64())
                            .unwrap_or(0.0),
                    );
                    f64_vals.push(v);
                    arg_slots.push(ArgSlot::F64(f64_vals.len() - 1));
                }
                DpiArgKind::ChandleIn => {
                    let p = Box::new(
                        args.get(i)
                            .map(|e| {
                                self.eval_expr(e).to_u64().unwrap_or(0) as usize as *mut c_void
                            })
                            .unwrap_or(std::ptr::null_mut()),
                    );
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                }
                DpiArgKind::StringIn => {
                    let s = args
                        .get(i)
                        .map(|e| {
                            if let ExprKind::StringLiteral(t) = &e.kind {
                                t.clone()
                            } else {
                                self.eval_expr(e).to_sv_string()
                            }
                        })
                        .unwrap_or_default();
                    let c = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
                    let p = Box::new(c.as_ptr() as *mut c_void);
                    cstrings.push(c);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                }
                DpiArgKind::StringOut => {
                    let init_s = args
                        .get(i)
                        .map(|e| {
                            if let ExprKind::StringLiteral(t) = &e.kind {
                                t.clone()
                            } else {
                                self.eval_expr(e).to_sv_string()
                            }
                        })
                        .unwrap_or_default();
                    let init_ptr: *const std::ffi::c_char = if init_s.is_empty() {
                        std::ptr::null()
                    } else {
                        let c = CString::new(init_s).unwrap_or_else(|_| CString::new("").unwrap());
                        let p = c.as_ptr();
                        cstrings.push(c);
                        p
                    };
                    let cell = Box::new(init_ptr);
                    let p = Box::new(
                        (&*cell as *const *const std::ffi::c_char as *mut *const std::ffi::c_char)
                            .cast::<c_void>(),
                    );
                    string_ptr_cells.push(cell);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((string_ptr_cells.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::OpenArrayIn(elem) | DpiArgKind::OpenArrayOut(elem) => {
                    let (mut values, shape) = self.dpi_collect_open_array(args.get(i));
                    // §35.5.6.1: C must not rely on an output formal's entry
                    // value. The reference simulator hands an output array a
                    // zeroed buffer and copies all of it back, so elements C
                    // leaves unwritten come back 0. inout keeps the actual.
                    let out_only = match &spec.proto {
                        crate::ast::decl::DPIProto::Function(fd) => fd.ports.get(i),
                        crate::ast::decl::DPIProto::Task(td) => td.ports.get(i),
                    }
                    .is_some_and(|pt| matches!(pt.direction, PortDirection::Output));
                    if out_only {
                        let zero = Value::zero(elem.width());
                        values.iter_mut().for_each(|v| *v = zero.clone());
                    }
                    let mut oa = DpiOpenArrayArg::new(*elem, &values, &shape);
                    let p = Box::new(oa.handle());
                    open_arrays.push(oa);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if matches!(kind, DpiArgKind::OpenArrayOut(_)) {
                        if let Some(expr) = args.get(i) {
                            writebacks.push((open_arrays.len() - 1, *kind, expr.clone()));
                        }
                    }
                }
                DpiArgKind::VecLogicIn(width) => {
                    // Standard svLogicVecVal*: pointer to an interleaved array
                    // of {aval, bval} u32 pairs (see dpi_value_to_logic_interleaved).
                    let vv = args
                        .get(i)
                        .map(|e| self.dpi_int_actual_w(e, *width))
                        .unwrap_or_else(|| Value::zero(*width));
                    let buf = Self::dpi_value_to_logic_interleaved(&vv, *width);
                    logic_aval.push(buf);
                    let raw_ptr = logic_aval.last_mut().unwrap().as_mut_ptr().cast::<c_void>();
                    ptr_vals.push(Box::new(raw_ptr));
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                }
                DpiArgKind::VecLogicOut(width) => {
                    let init = args
                        .get(i)
                        .map(|e| self.dpi_int_actual_w(e, *width))
                        .unwrap_or_else(|| Value::zero(*width));
                    let buf = Self::dpi_value_to_logic_interleaved(&init, *width);
                    logic_aval.push(buf);
                    let raw_ptr = logic_aval.last_mut().unwrap().as_mut_ptr().cast::<c_void>();
                    ptr_vals.push(Box::new(raw_ptr));
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((logic_aval.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::VecBitIn(width) => {
                    // 2-state `bit [N]` -> svBitVecVal* (just uint32_t*).
                    // ABI differs from VecLogicIn: no bval field.
                    let vv = args
                        .get(i)
                        .map(|e| self.dpi_int_actual_w(e, *width))
                        .unwrap_or_else(|| Value::zero(*width));
                    let (aval, _bval) = Self::dpi_value_to_logic_words(&vv, *width);
                    // Store in a dedicated Vec<Vec<u32>> to keep storage stable.
                    logic_aval.push(aval);
                    let raw_ptr = logic_aval.last_mut().unwrap().as_mut_ptr().cast::<c_void>();
                    // Put the actual data pointer (not a pointer-to-pointer) in the Box.
                    ptr_vals.push(Box::new(raw_ptr));
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                }
                DpiArgKind::VecBitOut(width) => {
                    let init = args
                        .get(i)
                        .map(|e| self.dpi_int_actual_w(e, *width))
                        .unwrap_or_else(|| Value::zero(*width));
                    let (aval, _bval) = Self::dpi_value_to_logic_words(&init, *width);
                    logic_aval.push(aval);
                    let p = Box::new(logic_aval.last_mut().unwrap().as_mut_ptr().cast::<c_void>());
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        // Track which logical_aval slot to read back.
                        writebacks.push((logic_aval.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::StructIn | DpiArgKind::StructOut => {
                    let port = match &spec.proto {
                        crate::ast::decl::DPIProto::Function(fd) => fd.ports.get(i),
                        crate::ast::decl::DPIProto::Task(td) => td.ports.get(i),
                    };
                    let ty =
                        port.and_then(|p| self.dpi_c_type(&p.data_type, &p.dimensions, false).ok());
                    // The actual's members are its leaves (`s.a`, `s.arr[1]`).
                    let base = args.get(i).and_then(|e| match &e.kind {
                        ExprKind::Ident(h) => Some(self.resolve_hier_name(h).into_owned()),
                        _ => None,
                    });
                    let size = ty.as_ref().map_or(8, |t| t.size_align().0);
                    let mut buf = vec![0u64; size.div_ceil(8).max(1)];
                    if let (Some(t), Some(b)) = (&ty, &base) {
                        let mut get =
                            |sfx: &str| self.get_signal_value_by_name(&format!("{b}{sfx}"));
                        // SAFETY: `buf` holds `size` bytes, the C object of `t`.
                        unsafe {
                            Self::dpi_store_leaves(
                                t,
                                buf.as_mut_ptr().cast::<u8>(),
                                "",
                                &mut get,
                                &mut struct_keep,
                            )
                        };
                    }
                    struct_bufs.push((buf, ty, base));
                    let p = struct_bufs
                        .last_mut()
                        .unwrap()
                        .0
                        .as_mut_ptr()
                        .cast::<c_void>();
                    ptr_vals.push(Box::new(p));
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if matches!(kind, DpiArgKind::StructOut) {
                        if let Some(expr) = args.get(i) {
                            writebacks.push((struct_bufs.len() - 1, *kind, expr.clone()));
                        }
                    }
                }
                DpiArgKind::Int32Out => {
                    let init = args
                        .get(i)
                        .map(|e| self.dpi_int_actual(e).to_i64().unwrap_or(0) as i32)
                        .unwrap_or(0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const i32 as *mut i32).cast::<c_void>());
                    i32_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((i32_vals.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::Int64Out => {
                    let init = args
                        .get(i)
                        .map(|e| self.dpi_int_actual(e).to_i64().unwrap_or(0))
                        .unwrap_or(0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const i64 as *mut i64).cast::<c_void>());
                    i64_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((i64_vals.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::Real32Out => {
                    let init = args
                        .get(i)
                        .map(|e| self.eval_expr(e).to_f64() as f32)
                        .unwrap_or(0.0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const f32 as *mut f32).cast::<c_void>());
                    f32_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((f32_vals.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::Real64Out => {
                    let init = args
                        .get(i)
                        .map(|e| self.eval_expr(e).to_f64())
                        .unwrap_or(0.0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const f64 as *mut f64).cast::<c_void>());
                    f64_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((f64_vals.len() - 1, *kind, expr.clone()));
                    }
                }
                DpiArgKind::ChandleOut => {
                    let init = args
                        .get(i)
                        .map(|e| self.eval_expr(e).to_u64().unwrap_or(0) as usize as *mut c_void)
                        .unwrap_or(std::ptr::null_mut());
                    let b = Box::new(init);
                    let p =
                        Box::new((&*b as *const *mut c_void as *mut *mut c_void).cast::<c_void>());
                    ptr_vals.push(b);
                    ptr_vals.push(p);
                    arg_slots.push(ArgSlot::Ptr(ptr_vals.len() - 1));
                    if let Some(expr) = args.get(i) {
                        writebacks.push((ptr_vals.len() - 2, *kind, expr.clone()));
                    }
                }
            }
        }

        let arg_refs: Vec<_> = arg_slots
            .iter()
            .map(|slot| match *slot {
                ArgSlot::I32(i) => Arg::new(i32_vals[i].as_ref()),
                ArgSlot::U8(i) => Arg::new(u8_vals[i].as_ref()),
                ArgSlot::I64(i) => Arg::new(i64_vals[i].as_ref()),
                ArgSlot::F32(i) => Arg::new(f32_vals[i].as_ref()),
                ArgSlot::F64(i) => Arg::new(f64_vals[i].as_ref()),
                ArgSlot::Ptr(i) => Arg::new(ptr_vals[i].as_ref()),
            })
            .collect();

        // Set active simulator for VPI callbacks
        let self_ptr = self as *mut Simulator;
        ACTIVE_SIMULATOR.with(|cell| cell.set(self_ptr));

        // Set the active DPI scope to match the import's declaration
        // site. DPI routines rely on `svGetScope` returning the caller's
        // enclosing scope so that scope-aware helpers find the right
        // package/module context (§35.5.3). A package import is in its
        // package (`pkg::foo` → `pkg`, or a bare name the package owns); a
        // module import is in the instance it is called through or running
        // in (`top.u_a`). The bare import name used to be taken as the
        // scope, so a module's context import saw a scope named after
        // itself.
        let scope_name: String = if let Some((pkg, _)) = sv_name.split_once("::") {
            pkg.to_string()
        } else if let Some(prefix) = via {
            self.hier_path(prefix)
        } else {
            self.dpi_context_scope(sv_name)
        };
        let prev_scope = ACTIVE_SCOPE.with(|cell| cell.get());
        if !scope_name.is_empty() {
            // Look up or create a scope handle and pin it for the
            // duration of this call.
            let scope_ptr = if let Some(&p) = self.dpi_scopes.get(&scope_name) {
                p
            } else {
                let mut scope = DpiScope {
                    name: [0u8; 256],
                    name_len: scope_name.len().min(255),
                };
                let bytes = scope_name.as_bytes();
                scope.name[..scope.name_len].copy_from_slice(&bytes[..scope.name_len]);
                let ptr = Box::into_raw(Box::new(scope)) as *mut libc::c_void;
                self.dpi_scopes.insert(scope_name.clone(), ptr);
                ptr
            };
            ACTIVE_SCOPE.with(|cell| cell.set(scope_ptr));
        }

        let mut result = match ret_kind {
            DpiRetKind::Void => {
                unsafe {
                    cif.call::<()>(fn_ptr, &arg_refs);
                }
                Value::zero(32)
            }
            DpiRetKind::Int32 => {
                // libffi stores an integral result narrower than ffi_arg as a
                // whole ffi_arg (8 bytes on x86-64): an i32 buffer was
                // overrun by 4 bytes on every call.
                let rv: u64 = unsafe { cif.call(fn_ptr, &arg_refs) };
                let mut v = Value::from_u64(rv as u32 as u64, 32);
                v.is_signed = true;
                v
            }
            DpiRetKind::Logic => {
                // libffi widens a return narrower than ffi_arg to a full
                // ffi_arg and stores all of it, so the buffer is a u64.
                let rv: u64 = unsafe { cif.call(fn_ptr, &arg_refs) };
                Self::dpi_svlogic_value(rv as u8)
            }
            DpiRetKind::Int64 => {
                let rv: i64 = unsafe { cif.call(fn_ptr, &arg_refs) };
                let mut v = Value::from_u64(rv as u64, 64);
                v.is_signed = true;
                v
            }
            DpiRetKind::Real32 => {
                let rv: f32 = unsafe { cif.call(fn_ptr, &arg_refs) };
                Value::from_f64(rv as f64)
            }
            DpiRetKind::Real64 => {
                let rv: f64 = unsafe { cif.call(fn_ptr, &arg_refs) };
                Value::from_f64(rv)
            }
            DpiRetKind::Chandle => {
                let rv: *mut c_void = unsafe { cif.call(fn_ptr, &arg_refs) };
                Value::from_u64(rv as usize as u64, 64)
            }
            DpiRetKind::String => {
                let rv: *const std::ffi::c_char = unsafe { cif.call(fn_ptr, &arg_refs) };
                if rv.is_null() {
                    Value::from_string("")
                } else {
                    let s = unsafe { CStr::from_ptr(rv) }.to_string_lossy();
                    Value::from_string(&s)
                }
            }
        };
        // §35.5.5: the C ABI hands a small integral return back promoted to
        // 32/64 bits; the SV side sees the DECLARED type. `byte unsigned`
        // returning 256 and `shortint` reading 65530 in an expression were
        // the symptoms (an assignment to a typed variable fitted it, a direct
        // use in `$display` did not).
        if matches!(ret_kind, DpiRetKind::Int32 | DpiRetKind::Int64) {
            if let crate::ast::decl::DPIProto::Function(fd) = &spec.proto {
                if let Some((w, signed)) = self.dpi_result_integral(&fd.return_type) {
                    if w > 0 && w <= 64 && (w != result.width || signed != result.is_signed) {
                        result = result.resize(w);
                        result.is_signed = signed;
                    }
                }
            }
        }

        // §35.9: a task that returns because of a disable must return 1, and
        // its outputs are not propagated.
        if self.dpi_unwinding {
            if matches!(spec.proto, crate::ast::decl::DPIProto::Task(_))
                && result.to_i64() != Some(1)
            {
                self.dpi_protocol_fatal(&format!(
                    "imported task '{}' was disabled but did not return 1",
                    sv_name
                ));
            }
            writebacks.clear();
        }
        // Write back output/ref/inout values.
        for (idx, kind, expr) in writebacks {
            match kind {
                DpiArgKind::Int32Out => {
                    if let Some(v) = i32_vals.get(idx) {
                        let w = self.infer_lhs_width(&expr);
                        self.assign_value(&expr, &Value::from_u64(**v as u32 as u64, w));
                    }
                }
                DpiArgKind::StructOut => {
                    if let Some((buf, Some(t), Some(b))) = struct_bufs.get(idx) {
                        let mut leaves = Vec::new();
                        // SAFETY: `buf` is the C object of `t` the call filled.
                        unsafe {
                            Self::dpi_load_leaves(t, buf.as_ptr().cast::<u8>(), "", &mut leaves)
                        };
                        for (sfx, v) in leaves {
                            self.set_signal_value_by_name(&format!("{b}{sfx}"), v);
                        }
                    }
                }
                DpiArgKind::LogicOut => {
                    if let Some(v) = i32_vals.get(idx) {
                        let w = self.infer_lhs_width(&expr);
                        let out = Self::dpi_svlogic_value(v.to_ne_bytes()[0]);
                        self.assign_value(&expr, &out.resize(w));
                    }
                }
                DpiArgKind::BitOut => {
                    if let Some(v) = i32_vals.get(idx) {
                        let w = self.infer_lhs_width(&expr);
                        // An svBit carries 0 or 1: keep bit 0 of the byte C
                        // wrote, then widen to the actual.
                        let out = Value::from_u64((v.to_ne_bytes()[0] & 1) as u64, 1);
                        self.assign_value(&expr, &out.resize(w.max(1)));
                    }
                }
                DpiArgKind::Int64Out => {
                    if let Some(v) = i64_vals.get(idx) {
                        let w = self.infer_lhs_width(&expr);
                        let mut out = Value::from_u64(**v as u64, w.max(64));
                        out.is_signed = true;
                        self.assign_value(&expr, &out.resize(w));
                    }
                }
                DpiArgKind::Real64Out => {
                    if let Some(v) = f64_vals.get(idx) {
                        self.assign_value(&expr, &Value::from_f64(**v));
                    }
                }
                DpiArgKind::Real32Out => {
                    if let Some(v) = f32_vals.get(idx) {
                        self.assign_value(&expr, &Value::from_f64(**v as f64));
                    }
                }
                DpiArgKind::ChandleOut => {
                    if let Some(v) = ptr_vals.get(idx) {
                        self.assign_value(&expr, &Value::from_u64((**v) as usize as u64, 64));
                    }
                }
                DpiArgKind::StringOut => {
                    if let Some(v) = string_ptr_cells.get(idx) {
                        let s = if (**v).is_null() {
                            String::new()
                        } else {
                            unsafe { CStr::from_ptr(**v) }.to_string_lossy().to_string()
                        };
                        self.assign_value(&expr, &Value::from_string(&s));
                    }
                }
                DpiArgKind::OpenArrayOut(_) => {
                    if let Some(oa) = open_arrays.get(idx) {
                        let values = oa.values();
                        self.dpi_writeback_open_array(&expr, &values);
                    }
                }
                DpiArgKind::VecLogicOut(width) => {
                    if idx < logic_aval.len() {
                        let out = Self::dpi_logic_interleaved_to_value(&logic_aval[idx], width);
                        let w = self.infer_lhs_width(&expr);
                        self.assign_value(&expr, &out.resize(w));
                    }
                }
                DpiArgKind::VecBitOut(width) if idx < logic_aval.len() => {
                    // 2-state: only aval, no bval
                    let out = Self::dpi_logic_words_to_value(&logic_aval[idx], &[], width);
                    let w = self.infer_lhs_width(&expr);
                    self.assign_value(&expr, &out.resize(w));
                }
                _ => {}
            }
        }

        if spec.property == Some(crate::ast::decl::DPIProperty::Pure) {
            // No side effects expected; kept for future optimization hooks.
            let _ = &mut result;
        }
        // Restore the caller's active simulator context after DPI call.
        ACTIVE_SIMULATOR.with(|cell| cell.set(prev_active));
        // Restore the previous DPI scope so nested DPI calls don't
        // leak scope state across invocations.
        ACTIVE_SCOPE.with(|cell| cell.set(prev_scope));
        Some(result)
    }

    pub fn run(&mut self) {
        // `kill -USR1 <pid>` on a live run prints the hang report.
        install_hang_report_handler();
        self.compile();
        self.simulate();
    }
}
