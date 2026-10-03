//! IEEE 1800-2017 chapter 37 — the VPI object model.
//!
//! xezim flattens the design at elaboration: every module is inlined into one
//! `ElaboratedModule`, signals are dotted names in one table, and the
//! instance tree survives only as `ElaboratedModule::instances`. That is
//! enough to resolve a name, not to answer what a VPI application asks of a
//! design: which generate scopes and named blocks exist, what a variable's
//! declared type and ranges are, which processes, continuous assignments,
//! primitives, tasks and functions a scope holds, and where each object is in
//! the source.
//!
//! This module rebuilds that object model the first time a VPI routine needs
//! it, and never otherwise:
//!
//! * the preprocessed text of each input file (`source_texts`, kept for
//!   diagnostics) is parsed again, which gives every definition's AST with
//!   byte spans into that text;
//! * every instance of the elaborated instance tree walks its definition,
//!   evaluating generate constructs with the instance's own parameter values
//!   the way the elaborator did, and records its scopes and objects together
//!   with the flat signal-table name each declared object was elaborated to;
//! * the result lives in a thread-local table owned by the simulator it was
//!   built for, and the VPI entry points in `simulator.rs` consult it through
//!   the hooks at the bottom of this file.
//!
//! A design loaded from a compiled artifact has no sources, hence no model:
//! the entry points then keep their flat signal-table behavior.
//!
//! Handles. Instances (modules, interfaces, programs) are `VpiKind::Module`
//! handles as before. Other scopes (packages, generate scopes, named blocks,
//! tasks, functions) are `VpiKind::Scope`, with `signal_id` the model scope
//! index. Objects with signal-table storage keep the value-bearing kinds
//! (`Signal`, `Memory`, `Slice`, `Port`), so reading, writing and value-change
//! callbacks work unchanged. Everything else is `VpiKind::Obj`, with
//! `signal_id` the model object index; a constant made on the fly (a range
//! bound, a generate index) is an `Obj` with `signal_id == usize::MAX`, its
//! value in `value` and its `vpiConstType` in `lsb`.
use super::*;
use crate::ast::decl::{
    AlwaysKind, BindDirective, ContinuousAssign, DataDeclaration, FunctionDeclaration,
    GateInstantiation, GateType, GenerateCase, GenerateFor, GenerateIf, ModuleInstantiation,
    ModuleItem, NetDeclaration, PackageItem, ParamAssignment, ParameterDeclaration, ParameterKind,
    PortConnection, PortDeclaration, SpecifyBlock, TaskDeclaration, TypedefDeclaration, UdpDecl,
};
use crate::ast::expr::{
    BinaryOp, ExprKind, Expression, HierPathSegment, HierarchicalIdentifier, NumberBase,
    NumberLiteral, RangeKind, UnaryOp,
};
use crate::ast::module::{
    InterfaceDeclaration, ModuleDeclaration, PackageDeclaration, PortList, ProgramDeclaration,
};
use crate::ast::stmt::{JoinType, Statement, StatementKind};
use crate::ast::types::{
    DataType, IntegerAtomType, IntegerVectorType, Lifetime, NetType, PackedDimension,
    PortDirection, RealType, Signing, SimpleType, StructUnionKind, UnpackedDimension,
};
use crate::ast::{Identifier, Span};
use libc::c_int;

/// "No such index" for the `u32` links of the model.
pub(super) const NONE: u32 = u32::MAX;

// The model's maps key and value on types of their own. An instantiation the
// simulator also uses (`HashMap<usize, u32>`, say) would gain call sites here,
// and that alone changes how the optimizer inlines it into the simulator's hot
// paths: 1% more instructions on a CoreMark run from two memo tables.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct AstKey(usize);
#[derive(Clone, Copy)]
pub(super) struct PortIdx(u32);
#[derive(Clone, Copy)]
struct FileIdx(u32);
#[derive(Clone, Copy)]
struct InstIdx(usize);
#[derive(Clone, Default)]
struct Names(Vec<String>);
#[derive(Default)]
struct ParamList(Vec<(String, Value)>);
#[derive(Default)]
struct Regions(Vec<(u32, u32)>);

/// VPI numbers used by the object model (IEEE 1800-2017 Annex K and M). Kept
/// apart from `super::vpi`, which holds the routines' own constants.
#[allow(dead_code)]
pub(super) mod c {
    use libc::c_int;
    // Object types (vpi_user.h).
    pub const ALWAYS: c_int = 1;
    pub const CONSTANT: c_int = 7;
    pub const CONT_ASSIGN: c_int = 8;
    pub const FUNCTION: c_int = 20;
    pub const GATE: c_int = 21;
    pub const INITIAL: c_int = 24;
    pub const INTEGER_VAR: c_int = 25;
    pub const IO_DECL: c_int = 28;
    pub const MEMORY: c_int = 29;
    pub const MEMORY_WORD: c_int = 30;
    pub const MOD_PATH: c_int = 31;
    pub const MODULE: c_int = 32;
    pub const NAMED_BEGIN: c_int = 33;
    pub const NAMED_EVENT: c_int = 34;
    pub const NAMED_FORK: c_int = 35;
    pub const NET: c_int = 36;
    pub const NET_BIT: c_int = 37;
    pub const OPERATION: c_int = 39;
    pub const PARAMETER: c_int = 41;
    pub const PART_SELECT: c_int = 42;
    pub const PATH_TERM: c_int = 43;
    pub const PORT: c_int = 44;
    pub const PORT_BIT: c_int = 45;
    pub const PRIM_TERM: c_int = 46;
    pub const REAL_VAR: c_int = 47;
    pub const REG: c_int = 48;
    pub const REG_BIT: c_int = 49;
    pub const SWITCH: c_int = 55;
    pub const TASK: c_int = 59;
    pub const TCHK: c_int = 61;
    pub const TCHK_TERM: c_int = 62;
    pub const TIME_VAR: c_int = 63;
    pub const UDP: c_int = 65;
    pub const VAR_SELECT: c_int = 68;
    pub const BIT_SELECT: c_int = 106;
    pub const NET_ARRAY: c_int = 114;
    pub const RANGE: c_int = 115;
    /// Also `vpiArrayVar`.
    pub const REG_ARRAY: c_int = 116;
    pub const NAMED_EVENT_ARRAY: c_int = 129;
    pub const GEN_SCOPE_ARRAY: c_int = 133;
    pub const GEN_SCOPE: c_int = 134;
    // Object types (sv_vpi_user.h).
    pub const PACKAGE: c_int = 600;
    pub const INTERFACE: c_int = 601;
    pub const PROGRAM: c_int = 602;
    pub const TYPESPEC: c_int = 605;
    pub const MODPORT: c_int = 606;
    /// A name the model cannot resolve to a declared object.
    pub const REF_OBJ: c_int = 608;
    pub const LONG_INT_VAR: c_int = 610;
    pub const SHORT_INT_VAR: c_int = 611;
    pub const INT_VAR: c_int = 612;
    pub const SHORT_REAL_VAR: c_int = 613;
    pub const BYTE_VAR: c_int = 614;
    pub const CLASS_VAR: c_int = 615;
    pub const STRING_VAR: c_int = 616;
    pub const ENUM_VAR: c_int = 617;
    pub const STRUCT_VAR: c_int = 618;
    pub const UNION_VAR: c_int = 619;
    pub const BIT_VAR: c_int = 620;
    pub const CHANDLE_VAR: c_int = 622;
    pub const PACKED_ARRAY_VAR: c_int = 623;
    pub const LONG_INT_TS: c_int = 625;
    pub const SHORT_REAL_TS: c_int = 626;
    pub const BYTE_TS: c_int = 627;
    pub const SHORT_INT_TS: c_int = 628;
    pub const INT_TS: c_int = 629;
    pub const CLASS_TS: c_int = 630;
    pub const STRING_TS: c_int = 631;
    pub const CHANDLE_TS: c_int = 632;
    pub const ENUM_TS: c_int = 633;
    pub const ENUM_CONST: c_int = 634;
    pub const INTEGER_TS: c_int = 635;
    pub const TIME_TS: c_int = 636;
    pub const REAL_TS: c_int = 637;
    pub const STRUCT_TS: c_int = 638;
    pub const UNION_TS: c_int = 639;
    pub const BIT_TS: c_int = 640;
    pub const LOGIC_TS: c_int = 641;
    pub const ARRAY_TS: c_int = 642;
    pub const VOID_TS: c_int = 643;
    pub const TYPESPEC_MEMBER: c_int = 644;
    pub const FINAL: c_int = 676;
    pub const ENUM_NET: c_int = 680;
    pub const INTEGER_NET: c_int = 681;
    pub const TIME_NET: c_int = 682;
    pub const STRUCT_NET: c_int = 683;
    pub const PACKED_ARRAY_TS: c_int = 692;
    pub const PACKED_ARRAY_NET: c_int = 693;
    pub const EVENT_TS: c_int = 698;
    pub const VIRTUAL_INTERFACE_VAR: c_int = 728;

    // One-to-one relations.
    pub const CONDITION: c_int = 71;
    pub const DELAY: c_int = 72;
    pub const HIGH_CONN: c_int = 76;
    pub const LHS: c_int = 77;
    pub const INDEX: c_int = 78;
    pub const LEFT_RANGE: c_int = 79;
    pub const LOW_CONN: c_int = 80;
    pub const PARENT: c_int = 81;
    pub const RHS: c_int = 82;
    pub const RIGHT_RANGE: c_int = 83;
    pub const SCOPE: c_int = 84;
    pub const TCHK_DATA_TERM: c_int = 86;
    pub const TCHK_NOTIFIER: c_int = 87;
    pub const TCHK_REF_TERM: c_int = 88;
    pub const EXPR: c_int = 102;
    pub const STMT: c_int = 104;
    pub const BASE_TYPESPEC: c_int = 703;
    pub const ELEM_TYPESPEC: c_int = 704;
    // One-to-many relations.
    pub const BIT: c_int = 90;
    pub const INTERNAL_SCOPE: c_int = 92;
    pub const MOD_PATH_IN: c_int = 95;
    pub const MOD_PATH_OUT: c_int = 96;
    pub const OPERAND: c_int = 97;
    pub const PORT_INST: c_int = 98;
    pub const PROCESS: c_int = 99;
    pub const VARIABLES: c_int = 100;
    pub const PRIMITIVE: c_int = 103;
    pub const PORTS: c_int = 125;
    pub const TASK_FUNC: c_int = 127;
    pub const MEMBER: c_int = 742;
    pub const INSTANCE: c_int = 745;

    // Properties.
    pub const TYPE: c_int = 1;
    pub const NAME: c_int = 2;
    pub const FULL_NAME: c_int = 3;
    pub const SIZE: c_int = 4;
    pub const FILE: c_int = 5;
    pub const LINE_NO: c_int = 6;
    pub const TOP_MODULE: c_int = 7;
    pub const CELL_INSTANCE: c_int = 8;
    pub const DEF_NAME: c_int = 9;
    pub const PROTECTED: c_int = 10;
    pub const DEF_FILE: c_int = 15;
    pub const DEF_LINE_NO: c_int = 16;
    pub const SCALAR: c_int = 17;
    pub const VECTOR: c_int = 18;
    pub const EXPLICIT_NAME: c_int = 19;
    pub const DIRECTION: c_int = 20;
    pub const CONN_BY_NAME: c_int = 21;
    pub const NET_TYPE: c_int = 22;
    pub const IMPLICIT_DECL: c_int = 26;
    pub const ARRAY: c_int = 28;
    pub const PORT_INDEX: c_int = 29;
    pub const TERM_INDEX: c_int = 30;
    pub const PRIM_TYPE: c_int = 33;
    pub const EDGE: c_int = 36;
    pub const TCHK_TYPE: c_int = 38;
    pub const OP_TYPE: c_int = 39;
    pub const CONST_TYPE: c_int = 40;
    pub const NET_DECL_ASSIGN: c_int = 43;
    pub const FUNC_TYPE: c_int = 44;
    pub const AUTOMATIC: c_int = 50;
    pub const RESOLVED_NET_TYPE: c_int = 61;
    pub const SIGNED: c_int = 65;
    pub const LOCAL_PARAM: c_int = 70;
    pub const MOD_PATH_HAS_IF_NONE: c_int = 71;
    pub const IS_MEMORY: c_int = 73;
    pub const IS_PROTECTED: c_int = 74;
    pub const TOP: c_int = 600;
    pub const UNIT: c_int = 602;
    pub const JOIN_TYPE: c_int = 603;
    pub const ACCESS_TYPE: c_int = 604;
    pub const ARRAY_TYPE: c_int = 606;
    pub const ARRAY_MEMBER: c_int = 607;
    pub const PORT_TYPE: c_int = 611;
    pub const CONSTANT_VARIABLE: c_int = 612;
    pub const STRUCT_UNION_MEMBER: c_int = 615;
    pub const VISIBILITY: c_int = 620;
    pub const ALWAYS_TYPE: c_int = 624;
    pub const PACKED: c_int = 630;
    pub const TAGGED: c_int = 632;
    pub const DPI_PURE: c_int = 665;
    pub const DPI_CONTEXT: c_int = 666;

    // Property values.
    pub const INPUT: c_int = 1;
    pub const OUTPUT: c_int = 2;
    pub const INOUT: c_int = 3;
    pub const NO_DIRECTION: c_int = 5;
    pub const REF: c_int = 6;
    pub const WIRE: c_int = 1;
    pub const WAND: c_int = 2;
    pub const WOR: c_int = 3;
    pub const TRI: c_int = 4;
    pub const TRI0: c_int = 5;
    pub const TRI1: c_int = 6;
    pub const TRIREG: c_int = 7;
    pub const TRIAND: c_int = 8;
    pub const TRIOR: c_int = 9;
    pub const SUPPLY1: c_int = 10;
    pub const SUPPLY0: c_int = 11;
    pub const NONE_NET: c_int = 12;
    pub const UWIRE: c_int = 13;
    pub const DEC_CONST: c_int = 1;
    pub const REAL_CONST: c_int = 2;
    pub const BINARY_CONST: c_int = 3;
    pub const OCT_CONST: c_int = 4;
    pub const HEX_CONST: c_int = 5;
    pub const STRING_CONST: c_int = 6;
    pub const INT_CONST: c_int = 7;
    pub const TIME_CONST: c_int = 8;
    pub const INT_FUNC: c_int = 1;
    pub const REAL_FUNC: c_int = 2;
    pub const TIME_FUNC: c_int = 3;
    pub const SIZED_FUNC: c_int = 4;
    pub const SIZED_SIGNED_FUNC: c_int = 5;
    pub const OTHER_FUNC: c_int = 6;
    pub const SEQ_PRIM: c_int = 27;
    pub const COMB_PRIM: c_int = 28;
    pub const JOIN: c_int = 0;
    pub const JOIN_NONE: c_int = 1;
    pub const JOIN_ANY: c_int = 2;
    pub const DPI_IMPORT_ACC: c_int = 4;
    pub const STATIC_ARRAY: c_int = 1;
    pub const DYNAMIC_ARRAY: c_int = 2;
    pub const ASSOC_ARRAY: c_int = 3;
    pub const QUEUE_ARRAY: c_int = 4;
    pub const INTERFACE_PORT: c_int = 1;
    pub const MODPORT_PORT: c_int = 2;
    pub const PUBLIC_VIS: c_int = 1;
    pub const ALWAYS_COMB: c_int = 2;
    pub const ALWAYS_FF: c_int = 3;
    pub const ALWAYS_LATCH: c_int = 4;
    pub const SETUP: c_int = 1;
    pub const HOLD: c_int = 2;
    pub const PERIOD: c_int = 3;
    pub const WIDTH: c_int = 4;
    pub const SKEW: c_int = 5;
    pub const RECOVERY: c_int = 6;
    pub const NO_CHANGE: c_int = 7;
    pub const SETUP_HOLD: c_int = 8;
    pub const FULLSKEW: c_int = 9;
    pub const RECREM: c_int = 10;
    pub const REMOVAL: c_int = 11;
    pub const TIMESKEW: c_int = 12;
    pub const EDGE01: c_int = 0x01;
    pub const EDGE10: c_int = 0x02;
    pub const EDGE0X: c_int = 0x04;
    pub const EDGEX1: c_int = 0x08;
    pub const EDGE1X: c_int = 0x10;
    pub const EDGEX0: c_int = 0x20;
    pub const UNDEFINED: c_int = -1;
}

/// `vpi_get_str(vpiType, h)`: the standard spelling of every type code the
/// object model can report.
pub(super) fn type_name(code: c_int) -> Option<&'static str> {
    Some(match code {
        c::ALWAYS => "vpiAlways",
        c::CONSTANT => "vpiConstant",
        c::CONT_ASSIGN => "vpiContAssign",
        c::FUNCTION => "vpiFunction",
        c::GATE => "vpiGate",
        c::INITIAL => "vpiInitial",
        c::INTEGER_VAR => "vpiIntegerVar",
        27 => "vpiIterator",
        c::IO_DECL => "vpiIODecl",
        c::MEMORY => "vpiMemory",
        c::MEMORY_WORD => "vpiMemoryWord",
        c::MOD_PATH => "vpiModPath",
        c::MODULE => "vpiModule",
        c::NAMED_BEGIN => "vpiNamedBegin",
        c::NAMED_EVENT => "vpiNamedEvent",
        c::NAMED_FORK => "vpiNamedFork",
        c::NET => "vpiNet",
        c::NET_BIT => "vpiNetBit",
        c::OPERATION => "vpiOperation",
        c::PARAMETER => "vpiParameter",
        c::PART_SELECT => "vpiPartSelect",
        c::PATH_TERM => "vpiPathTerm",
        c::PORT => "vpiPort",
        c::PORT_BIT => "vpiPortBit",
        c::PRIM_TERM => "vpiPrimTerm",
        c::REAL_VAR => "vpiRealVar",
        c::REG => "vpiReg",
        c::REG_BIT => "vpiRegBit",
        c::SWITCH => "vpiSwitch",
        56 => "vpiSysFuncCall",
        57 => "vpiSysTaskCall",
        c::TASK => "vpiTask",
        c::TCHK => "vpiTchk",
        c::TCHK_TERM => "vpiTchkTerm",
        c::TIME_VAR => "vpiTimeVar",
        c::UDP => "vpiUdp",
        c::BIT_SELECT => "vpiBitSelect",
        c::NET_ARRAY => "vpiNetArray",
        c::RANGE => "vpiRange",
        c::REG_ARRAY => "vpiRegArray",
        c::NAMED_EVENT_ARRAY => "vpiNamedEventArray",
        c::GEN_SCOPE_ARRAY => "vpiGenScopeArray",
        c::GEN_SCOPE => "vpiGenScope",
        c::PACKAGE => "vpiPackage",
        c::INTERFACE => "vpiInterface",
        c::PROGRAM => "vpiProgram",
        c::TYPESPEC => "vpiTypespec",
        c::MODPORT => "vpiModport",
        c::REF_OBJ => "vpiRefObj",
        c::LONG_INT_VAR => "vpiLongIntVar",
        c::SHORT_INT_VAR => "vpiShortIntVar",
        c::INT_VAR => "vpiIntVar",
        c::SHORT_REAL_VAR => "vpiShortRealVar",
        c::BYTE_VAR => "vpiByteVar",
        c::CLASS_VAR => "vpiClassVar",
        c::STRING_VAR => "vpiStringVar",
        c::ENUM_VAR => "vpiEnumVar",
        c::STRUCT_VAR => "vpiStructVar",
        c::UNION_VAR => "vpiUnionVar",
        c::BIT_VAR => "vpiBitVar",
        c::CHANDLE_VAR => "vpiChandleVar",
        c::PACKED_ARRAY_VAR => "vpiPackedArrayVar",
        c::LONG_INT_TS => "vpiLongIntTypespec",
        c::SHORT_REAL_TS => "vpiShortRealTypespec",
        c::BYTE_TS => "vpiByteTypespec",
        c::SHORT_INT_TS => "vpiShortIntTypespec",
        c::INT_TS => "vpiIntTypespec",
        c::CLASS_TS => "vpiClassTypespec",
        c::STRING_TS => "vpiStringTypespec",
        c::CHANDLE_TS => "vpiChandleTypespec",
        c::ENUM_TS => "vpiEnumTypespec",
        c::ENUM_CONST => "vpiEnumConst",
        c::INTEGER_TS => "vpiIntegerTypespec",
        c::TIME_TS => "vpiTimeTypespec",
        c::REAL_TS => "vpiRealTypespec",
        c::STRUCT_TS => "vpiStructTypespec",
        c::UNION_TS => "vpiUnionTypespec",
        c::BIT_TS => "vpiBitTypespec",
        c::LOGIC_TS => "vpiLogicTypespec",
        c::ARRAY_TS => "vpiArrayTypespec",
        c::VOID_TS => "vpiVoidTypespec",
        c::TYPESPEC_MEMBER => "vpiTypespecMember",
        c::FINAL => "vpiFinal",
        c::ENUM_NET => "vpiEnumNet",
        c::INTEGER_NET => "vpiIntegerNet",
        c::TIME_NET => "vpiTimeNet",
        c::STRUCT_NET => "vpiStructNet",
        c::PACKED_ARRAY_TS => "vpiPackedArrayTypespec",
        c::PACKED_ARRAY_NET => "vpiPackedArrayNet",
        c::EVENT_TS => "vpiEventTypespec",
        c::VIRTUAL_INTERFACE_VAR => "vpiVirtualInterfaceVar",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Type descriptors
// ---------------------------------------------------------------------------

/// The declared shape of a data type, resolved through typedefs and with its
/// dimensions evaluated in the declaring instance's parameter context.
#[derive(Clone, Debug)]
pub(super) enum TKind {
    Logic,
    Reg,
    Bit,
    Integer,
    Int,
    ShortInt,
    LongInt,
    Byte,
    Time,
    Real,
    ShortReal,
    String,
    Chandle,
    Event,
    /// Index into `VpiModel::enums`.
    Enum(u32),
    /// Index into `VpiModel::structs` (struct or union).
    Struct(u32),
    Class(String),
    VirtualIf(String),
    /// No data type written (`wire [3:0] w`, `input a`).
    Implicit,
    Void,
    /// A type the model cannot resolve (e.g. a type parameter).
    Unknown,
}

/// One unpacked dimension.
#[derive(Clone, Copy, Debug)]
pub(super) enum UDim {
    Range(i64, i64),
    Dynamic,
    Queue,
    Assoc,
}

#[derive(Clone, Debug)]
pub(super) struct TDesc {
    pub kind: TKind,
    pub signed: bool,
    /// Packed dimensions as declared `(left, right)`, outermost first.
    pub packed: Vec<(i64, i64)>,
    /// Unpacked dimensions, outermost first.
    pub unpacked: Vec<UDim>,
    /// The typedef the type was declared through, if any.
    pub name: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct EnumDesc {
    pub base: TDesc,
    pub consts: Vec<(String, u64)>,
}

#[derive(Clone, Debug)]
pub(super) struct StructDesc {
    pub union: bool,
    pub packed: bool,
    pub tagged: bool,
    /// Member name and type, declaration order.
    pub members: Vec<(String, TDesc, u32)>,
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SKind {
    Module,
    Interface,
    Program,
    Package,
    GenScope,
    NamedBegin,
    NamedFork,
    Task,
    Function,
}

pub(super) struct MScope {
    pub kind: SKind,
    pub type_code: c_int,
    pub name: String,
    /// The VPI full name (`top.gl[0]`, `pkg::`).
    pub full: String,
    pub def_name: String,
    /// Enclosing scope, `NONE` for a top instance or a package.
    pub parent: u32,
    /// Instances: index into `module.instances`, -1 for the single top.
    /// Every other scope: -2.
    pub inst_idx: isize,
    pub file: u32,
    pub line: u32,
    pub def_file: u32,
    pub def_line: u32,
    /// Child scopes in declaration order.
    pub children: Vec<u32>,
    /// Objects declared in the scope, declaration order.
    pub members: Vec<u32>,
    /// Element of a generate scope array: the array object.
    pub gen_array: u32,
    pub index: i64,
    /// A `genblk<n>` name the source did not write.
    pub implicit: bool,
    pub automatic: bool,
    pub cell: bool,
    pub top: bool,
    /// Named fork: `vpiJoinType`.
    pub join: c_int,
    /// Function: `vpiFuncType` and its return type.
    pub func_type: c_int,
    pub ret_td: u32,
    /// DPI import: `vpiAccessType` etc.
    pub dpi: Option<(bool, bool)>,
    /// Prefix of the flat signal names declared directly in this scope.
    pub flat_prefix: String,
    pub flat_suffix: String,
}

pub(super) struct MObj {
    pub type_code: c_int,
    pub name: String,
    pub full: String,
    pub scope: u32,
    /// Parent object for a sub-object (a primitive's terminal, a path term).
    pub parent: u32,
    pub file: u32,
    pub line: u32,
    pub d: OData,
}

pub(super) struct VarData {
    /// Signal-table name the declaration was elaborated to.
    pub flat: String,
    /// Storage: the signal id (element 0 of an array).
    pub sig: Option<usize>,
    pub td: u32,
    /// `vpiNetType` for a net, 0 for a variable.
    pub net_type: c_int,
    pub dir: c_int,
    pub automatic: bool,
    pub implicit: bool,
    pub is_const: bool,
    /// Struct / union members (objects).
    pub members: Vec<u32>,
    /// A member of a packed struct: its bit range in the container.
    pub member_of: u32,
    pub lsb: u32,
    pub width: u32,
}

pub(super) struct PortData {
    pub index: u32,
    pub dir: c_int,
    pub low: u32,
    pub high: u32,
    pub port_type: c_int,
    pub conn_by_name: bool,
}

pub(super) struct ExprData {
    /// Scope the expression's names resolve in.
    pub scope: u32,
    pub expr: Box<Expression>,
}

pub(super) enum OData {
    Var(VarData),
    Param {
        flat: String,
        sig: Option<usize>,
        td: u32,
        local: bool,
        const_type: c_int,
    },
    Process {
        always_type: c_int,
        stmt: u32,
    },
    ContAssign {
        lhs: u32,
        rhs: u32,
        delay: u32,
        net_decl: bool,
    },
    Prim {
        prim_type: c_int,
        def_name: String,
        terms: Vec<u32>,
        delay: u32,
        inputs: u32,
    },
    PrimTerm {
        index: u32,
        dir: c_int,
        expr: u32,
    },
    Port(PortData),
    IoDecl {
        dir: c_int,
        td: u32,
    },
    GenArray {
        elems: Vec<u32>,
    },
    Expr(ExprData),
    ModPath {
        ins: Vec<u32>,
        outs: Vec<u32>,
        cond: u32,
        ifnone: bool,
    },
    PathTerm {
        dir: c_int,
        expr: u32,
    },
    Tchk {
        tchk_type: c_int,
        ref_term: u32,
        data_term: u32,
        notifier: u32,
    },
    TchkTerm {
        edge: c_int,
        expr: u32,
        cond: u32,
    },
    Typespec {
        td: u32,
    },
    TypespecMember {
        td: u32,
    },
    EnumConst {
        value: u64,
        width: u32,
    },
    Range {
        left: i64,
        right: i64,
    },
    /// An interface modport and its `vpiIODecl`s.
    Modport {
        ios: Vec<u32>,
    },
    /// One port of a modport: its direction and the interface object (or
    /// modport expression) it names.
    ModportIo {
        dir: c_int,
        expr: u32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MRef {
    Scope(u32),
    Obj(u32),
}

pub(super) struct VpiModel {
    /// Address of the simulator the model describes.
    owner: usize,
    pub scopes: Vec<MScope>,
    pub objs: Vec<MObj>,
    pub tdescs: Vec<TDesc>,
    pub enums: Vec<EnumDesc>,
    pub structs: Vec<StructDesc>,
    pub files: Vec<String>,
    file_ids: HashMap<String, FileIdx>,
    /// VPI full name -> object.
    pub by_name: HashMap<String, MRef>,
    /// Port full name -> port object (a port shares its name with its net).
    pub port_by_name: HashMap<String, PortIdx>,
    pub inst_scope: HashMap<isize, u32>,
    /// Top-level instances, then packages.
    pub tops: Vec<u32>,
    pub packages: Vec<u32>,
    /// Flat signal names that stand for no object: the one-bit placeholder
    /// signals elaboration makes for generate block and instance names.
    pub placeholders: HashSet<String>,
    /// The genvar and its value, per generate loop element scope.
    pub genvars: HashMap<u32, (String, i64)>,
    /// Objects made on demand (typespecs, ranges), keyed by what they
    /// describe, so asking twice yields the same object.
    memo: HashMap<(u8, u32, u32), u32>,
}

thread_local! {
    static MODEL: RefCell<Option<Box<VpiModel>>> = const { RefCell::new(None) };
}

/// Run `f` on the model of `sim`, building it first if needed. `None` when
/// the design has no sources to build it from.
///
/// Re-entrant use (a `$systf` evaluated while the model is borrowed) finds
/// the model busy and gets `None`, like a design without sources.
pub(super) fn with_model<R>(
    sim: &mut Simulator,
    f: impl FnOnce(&mut VpiModel, &mut Simulator) -> R,
) -> Option<R> {
    MODEL.with(|cell| {
        let owner = sim as *const Simulator as usize;
        let stale = match cell.try_borrow() {
            Ok(b) => !matches!(&*b, Some(m) if m.owner == owner),
            Err(_) => return None,
        };
        if stale {
            let built = Builder::build(sim);
            *cell.try_borrow_mut().ok()? = built;
        }
        let mut b = cell.try_borrow_mut().ok()?;
        b.as_mut().map(|m| f(m, sim))
    })
}

impl VpiModel {
    fn intern_file(&mut self, f: &str) -> u32 {
        if let Some(&FileIdx(i)) = self.file_ids.get(f) {
            return i;
        }
        self.files.push(f.to_string());
        let i = (self.files.len() - 1) as u32;
        self.file_ids.insert(f.to_string(), FileIdx(i));
        i
    }

    fn add_scope(&mut self, s: MScope) -> u32 {
        let idx = self.scopes.len() as u32;
        let parent = s.parent;
        self.by_name
            .entry(s.full.clone())
            .or_insert(MRef::Scope(idx));
        self.scopes.push(s);
        if parent != NONE {
            self.scopes[parent as usize].children.push(idx);
        }
        idx
    }

    fn add_obj(&mut self, o: MObj, register: bool, member: bool) -> u32 {
        let idx = self.objs.len() as u32;
        if register && !o.full.is_empty() {
            self.by_name.entry(o.full.clone()).or_insert(MRef::Obj(idx));
        }
        let scope = o.scope;
        self.objs.push(o);
        if member && scope != NONE {
            self.scopes[scope as usize].members.push(idx);
        }
        idx
    }

    fn add_td(&mut self, t: TDesc) -> u32 {
        self.tdescs.push(t);
        (self.tdescs.len() - 1) as u32
    }

    /// Bits of one value of `t`, without its unpacked dimensions.
    pub fn td_width(&self, t: &TDesc) -> u32 {
        let base = match &t.kind {
            TKind::Logic | TKind::Reg | TKind::Bit | TKind::Implicit => 1,
            TKind::Integer | TKind::Int => 32,
            TKind::ShortInt => 16,
            TKind::LongInt | TKind::Time | TKind::Real => 64,
            TKind::Byte => 8,
            TKind::ShortReal => 32,
            TKind::Enum(e) => self.td_width(&self.enums[*e as usize].base),
            TKind::Struct(s) => {
                let sd = &self.structs[*s as usize];
                let ws = sd.members.iter().map(|(_, m, _)| {
                    let mut w = self.td_width(m);
                    for d in &m.unpacked {
                        if let UDim::Range(l, r) = d {
                            w *= ((l - r).unsigned_abs() + 1) as u32;
                        }
                    }
                    w
                });
                if sd.union {
                    ws.max().unwrap_or(0)
                } else {
                    ws.sum()
                }
            }
            _ => 0,
        };
        t.packed.iter().fold(base, |w, (l, r)| {
            w.saturating_mul(((l - r).unsigned_abs() + 1) as u32)
        })
    }

    /// Is `t` a 4-state integral type (a legal net data type)?
    fn td_four_state(&self, t: &TDesc) -> bool {
        match &t.kind {
            TKind::Logic | TKind::Reg | TKind::Implicit | TKind::Integer | TKind::Time => true,
            TKind::Enum(e) => self.td_four_state(&self.enums[*e as usize].base),
            TKind::Struct(s) => self.structs[*s as usize]
                .members
                .iter()
                .all(|(_, m, _)| self.td_four_state(m)),
            _ => false,
        }
    }

    /// `vpiType` of a variable of type `t`.
    pub fn var_type_code(&self, t: &TDesc) -> c_int {
        if !t.unpacked.is_empty() {
            return if matches!(t.kind, TKind::Event) {
                c::NAMED_EVENT_ARRAY
            } else {
                c::REG_ARRAY
            };
        }
        match &t.kind {
            TKind::Logic | TKind::Reg | TKind::Implicit | TKind::Unknown => c::REG,
            TKind::Bit => c::BIT_VAR,
            TKind::Integer => c::INTEGER_VAR,
            TKind::Int => c::INT_VAR,
            TKind::ShortInt => c::SHORT_INT_VAR,
            TKind::LongInt => c::LONG_INT_VAR,
            TKind::Byte => c::BYTE_VAR,
            TKind::Time => c::TIME_VAR,
            TKind::Real => c::REAL_VAR,
            TKind::ShortReal => c::SHORT_REAL_VAR,
            TKind::String => c::STRING_VAR,
            TKind::Chandle => c::CHANDLE_VAR,
            TKind::Event => c::NAMED_EVENT,
            TKind::Enum(_) if !t.packed.is_empty() => c::PACKED_ARRAY_VAR,
            TKind::Enum(_) => c::ENUM_VAR,
            TKind::Struct(_) if !t.packed.is_empty() => c::PACKED_ARRAY_VAR,
            TKind::Struct(s) if self.structs[*s as usize].union => c::UNION_VAR,
            TKind::Struct(_) => c::STRUCT_VAR,
            TKind::Class(_) => c::CLASS_VAR,
            TKind::VirtualIf(_) => c::VIRTUAL_INTERFACE_VAR,
            TKind::Void => c::REG,
        }
    }

    /// `vpiType` of a net of type `t`.
    pub fn net_type_code(&self, t: &TDesc) -> c_int {
        if !t.unpacked.is_empty() {
            return c::NET_ARRAY;
        }
        match &t.kind {
            TKind::Enum(_) | TKind::Struct(_) if !t.packed.is_empty() => c::PACKED_ARRAY_NET,
            TKind::Enum(_) => c::ENUM_NET,
            TKind::Struct(_) => c::STRUCT_NET,
            TKind::Integer => c::INTEGER_NET,
            TKind::Time => c::TIME_NET,
            _ => c::NET,
        }
    }

    /// `vpiType` of a typespec for `t`.
    pub fn ts_type_code(&self, t: &TDesc) -> c_int {
        if !t.unpacked.is_empty() {
            return c::ARRAY_TS;
        }
        match &t.kind {
            TKind::Logic | TKind::Reg | TKind::Implicit | TKind::Unknown => c::LOGIC_TS,
            TKind::Bit => c::BIT_TS,
            TKind::Integer => c::INTEGER_TS,
            TKind::Int => c::INT_TS,
            TKind::ShortInt => c::SHORT_INT_TS,
            TKind::LongInt => c::LONG_INT_TS,
            TKind::Byte => c::BYTE_TS,
            TKind::Time => c::TIME_TS,
            TKind::Real => c::REAL_TS,
            TKind::ShortReal => c::SHORT_REAL_TS,
            TKind::String => c::STRING_TS,
            TKind::Chandle => c::CHANDLE_TS,
            TKind::Event => c::EVENT_TS,
            TKind::Enum(_) | TKind::Struct(_) if !t.packed.is_empty() => c::PACKED_ARRAY_TS,
            TKind::Enum(_) => c::ENUM_TS,
            TKind::Struct(s) if self.structs[*s as usize].union => c::UNION_TS,
            TKind::Struct(_) => c::STRUCT_TS,
            TKind::Class(_) => c::CLASS_TS,
            TKind::VirtualIf(_) => c::TYPESPEC,
            TKind::Void => c::VOID_TS,
        }
    }

    /// An on-demand object, made once per key.
    fn memo_obj(&mut self, key: (u8, u32, u32), make: impl FnOnce(&mut Self) -> MObj) -> u32 {
        if let Some(&i) = self.memo.get(&key) {
            return i;
        }
        let o = make(self);
        let i = self.add_obj(o, false, false);
        self.memo.insert(key, i);
        i
    }

    fn typespec_obj(&mut self, td: u32) -> u32 {
        self.memo_obj((1, td, 0), |m| {
            let t = &m.tdescs[td as usize];
            MObj {
                type_code: m.ts_type_code(t),
                name: t.name.clone().unwrap_or_default(),
                full: String::new(),
                scope: NONE,
                parent: NONE,
                file: NONE,
                line: 0,
                d: OData::Typespec { td },
            }
        })
    }

    /// The element type of an array type (its outermost unpacked dimension
    /// removed).
    fn elem_td(&mut self, td: u32) -> u32 {
        if let Some(&i) = self.memo.get(&(2, td, 0)) {
            return self.objs[i as usize].scope; // stored td index
        }
        let mut t = self.tdescs[td as usize].clone();
        if !t.unpacked.is_empty() {
            t.unpacked.remove(0);
        }
        if !t.unpacked.is_empty() {
            t.name = None;
        }
        let e = self.add_td(t);
        // Remember it through a placeholder object slot so the memo stays one
        // map; `scope` carries the element descriptor index.
        let o = MObj {
            type_code: c::TYPESPEC,
            name: String::new(),
            full: String::new(),
            scope: e,
            parent: NONE,
            file: NONE,
            line: 0,
            d: OData::Typespec { td: e },
        };
        let i = self.add_obj(o, false, false);
        self.memo.insert((2, td, 0), i);
        e
    }

    /// A `vpiRange` object for dimension `dim` of `td` (packed dimensions
    /// first when `packed`).
    fn range_obj(&mut self, td: u32, packed: bool, dim: u32) -> Option<u32> {
        let t = &self.tdescs[td as usize];
        let (l, r) = if packed {
            *t.packed.get(dim as usize)?
        } else {
            match t.unpacked.get(dim as usize)? {
                UDim::Range(l, r) => (*l, *r),
                _ => return None,
            }
        };
        Some(self.memo_obj((3 + u8::from(packed), td, dim), |_| MObj {
            type_code: c::RANGE,
            name: String::new(),
            full: String::new(),
            scope: NONE,
            parent: NONE,
            file: NONE,
            line: 0,
            d: OData::Range { left: l, right: r },
        }))
    }

    pub fn scope_of_ref(&self, r: MRef) -> u32 {
        match r {
            MRef::Scope(s) => self.scopes[s as usize].parent,
            MRef::Obj(o) => self.objs[o as usize].scope,
        }
    }

    /// The instance (module, interface, program or package) a scope is in.
    pub fn instance_of(&self, mut s: u32) -> u32 {
        while s != NONE {
            let sc = &self.scopes[s as usize];
            if matches!(
                sc.kind,
                SKind::Module | SKind::Interface | SKind::Program | SKind::Package
            ) {
                return s;
            }
            s = sc.parent;
        }
        NONE
    }

    /// The value of genvar `name` seen from `scope`, when `name` is no
    /// declared object there.
    fn genvar_value(&self, scope: u32, name: &str) -> Option<i64> {
        let mut s = scope;
        while s != NONE {
            let sc = &self.scopes[s as usize];
            let key = format!("{}.{}", sc.full, name);
            if self.by_name.contains_key(&key) {
                return None;
            }
            if let Some((g, v)) = self.genvars.get(&s) {
                if g == name {
                    return Some(*v);
                }
            }
            if matches!(sc.kind, SKind::Module | SKind::Interface | SKind::Program) {
                return None;
            }
            s = sc.parent;
        }
        None
    }

    /// Resolve a simple name in `scope` the way SystemVerilog does: the scope
    /// itself, then each enclosing scope, then the packages.
    pub fn resolve_name(&self, scope: u32, name: &str) -> Option<MRef> {
        let mut s = scope;
        while s != NONE {
            let sc = &self.scopes[s as usize];
            let key = if sc.kind == SKind::Package {
                format!("{}{}", sc.full, name)
            } else {
                format!("{}.{}", sc.full, name)
            };
            if let Some(r) = self.by_name.get(&key) {
                return Some(*r);
            }
            s = sc.parent;
        }
        for &p in &self.packages {
            if let Some(r) = self
                .by_name
                .get(&format!("{}{}", self.scopes[p as usize].full, name))
            {
                return Some(*r);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum DefRef<'a> {
    Module(&'a ModuleDeclaration),
    Interface(&'a InterfaceDeclaration),
    Program(&'a ProgramDeclaration),
    Udp(&'a UdpDecl),
}

impl<'a> DefRef<'a> {
    fn parts(
        &self,
    ) -> Option<(
        &'a PortList,
        &'a [ParameterDeclaration],
        &'a [ModuleItem],
        Option<Lifetime>,
        Span,
    )> {
        match self {
            DefRef::Module(m) => Some((&m.ports, &m.params, &m.items, m.lifetime, m.span)),
            DefRef::Interface(m) => Some((&m.ports, &m.params, &m.items, m.lifetime, m.span)),
            DefRef::Program(m) => Some((&m.ports, &m.params, &m.items, m.lifetime, m.span)),
            DefRef::Udp(_) => None,
        }
    }
}

/// Walk context of one scope.
#[derive(Clone)]
struct Ctx {
    scope: u32,
    /// Source file (index into `source_texts`) of the definition.
    file: u32,
    /// Prefix and suffix of the flat names of declarations made here.
    flat_prefix: String,
    flat_suffix: String,
    /// Prefix of child instance paths (`module.instances[..].path`).
    inst_prefix: String,
    /// Parameter values visible here (instance parameters and genvars).
    params: std::rc::Rc<HashMap<String, Value>>,
    automatic: bool,
    /// Body parameters are local (the definition has a parameter port list).
    body_params_local: bool,
}

/// Port information gathered before the port objects are made.
struct PortDecl {
    name: String,
    dir: c_int,
    port_type: c_int,
}

struct Builder<'a> {
    sim: &'a Simulator,
    m: VpiModel,
    defs: HashMap<&'a str, (DefRef<'a>, u32)>,
    packages: Vec<(&'a PackageDeclaration, u32)>,
    /// §23.11 `bind` directives, with their source file.
    binds: Vec<(&'a BindDirective, u32)>,
    typedefs: HashMap<String, (&'a TypedefDeclaration, u32)>,
    inst_by_path: HashMap<&'a str, InstIdx>,
    attached: Vec<bool>,
    /// Parameters grouped by their flat scope prefix.
    params_by_prefix: HashMap<String, ParamList>,
    global_params: HashMap<String, Value>,
    adopted: HashSet<String>,
    cell_regions: HashMap<String, Regions>,
    struct_memo: HashMap<AstKey, u32>,
    enum_memo: HashMap<AstKey, u32>,
    /// §6.10 implicit nets by the flat prefix of their scope.
    implicit_by_prefix: HashMap<String, Names>,
    /// Byte offset of each line start, per source text (built on demand).
    line_starts: Vec<Option<Vec<usize>>>,
    /// Adopted library files: path, preprocessed text, and whether its
    /// lines are the file's own.
    lib_texts: Vec<(String, String, bool)>,
    depth: u32,
}

impl<'a> Builder<'a> {
    fn build(sim: &Simulator) -> Option<Box<VpiModel>> {
        let texts = &sim.module.source_texts;
        if texts.is_empty() {
            return None;
        }
        // The same lexer and parser the compile ran, on the same text: the
        // spans index `source_texts[i]`, as the elaborated design's do.
        let parse = |t: &str| {
            let tokens = xezim_core::lexer::Lexer::new(t).tokenize();
            let mut p = xezim_core::parse::Parser::new(tokens);
            p.parse_source_text()
        };
        let mut asts: Vec<crate::ast::SourceText> = texts.iter().map(|t| parse(t)).collect();
        // `-v`/`-y` library files the elaboration adopted definitions from
        // are not among the sources: preprocess and parse them again. They
        // follow the sources in file numbering.
        let mut lib_texts: Vec<(String, String, bool)> = Vec::new();
        for (path, _) in xezim_core::adopted_lib_files() {
            if let Some(text) = xezim_core::preprocess_adopted_lib(&path) {
                // Lines are exact only when preprocessing kept the line
                // structure (no `include` spliced in).
                let exact = std::fs::read_to_string(&path)
                    .is_ok_and(|orig| orig.lines().count() == text.lines().count());
                asts.push(parse(&text));
                lib_texts.push((path.display().to_string(), text, exact));
            }
        }
        let mut b = Builder {
            sim,
            m: VpiModel {
                owner: sim as *const Simulator as usize,
                scopes: Vec::new(),
                objs: Vec::new(),
                tdescs: Vec::new(),
                enums: Vec::new(),
                structs: Vec::new(),
                files: Vec::new(),
                file_ids: HashMap::default(),
                by_name: HashMap::default(),
                port_by_name: HashMap::default(),
                inst_scope: HashMap::default(),
                tops: Vec::new(),
                packages: Vec::new(),
                placeholders: HashSet::default(),
                genvars: HashMap::default(),
                memo: HashMap::default(),
            },
            defs: HashMap::default(),
            packages: Vec::new(),
            binds: Vec::new(),
            typedefs: HashMap::default(),
            inst_by_path: HashMap::default(),
            attached: vec![false; sim.module.instances.len()],
            params_by_prefix: HashMap::default(),
            global_params: HashMap::default(),
            adopted: HashSet::default(),
            cell_regions: HashMap::default(),
            struct_memo: HashMap::default(),
            enum_memo: HashMap::default(),
            implicit_by_prefix: HashMap::default(),
            line_starts: vec![None; texts.len() + lib_texts.len()],
            lib_texts,
            depth: 0,
        };
        b.index(&asts);
        b.walk_design();
        Some(Box::new(b.m))
    }

    fn index(&mut self, asts: &'a [crate::ast::SourceText]) {
        use crate::ast::Description as D;
        fn nested<'b>(
            items: &'b [ModuleItem],
            file: u32,
            defs: &mut HashMap<&'b str, (DefRef<'b>, u32)>,
            binds: &mut Vec<(&'b BindDirective, u32)>,
        ) {
            for it in items {
                match it {
                    ModuleItem::NestedModule(m) => {
                        defs.entry(m.name.name.as_str())
                            .or_insert((DefRef::Module(m), file));
                        nested(&m.items, file, defs, binds);
                    }
                    ModuleItem::Bind(b) => binds.push((b, file)),
                    _ => {}
                }
            }
        }
        for (fi, st) in asts.iter().enumerate() {
            let fi = fi as u32;
            for d in &st.descriptions {
                match d {
                    D::Module(m) => {
                        self.defs
                            .entry(m.name.name.as_str())
                            .or_insert((DefRef::Module(m), fi));
                        nested(&m.items, fi, &mut self.defs, &mut self.binds);
                    }
                    D::Interface(m) => {
                        self.defs
                            .entry(m.name.name.as_str())
                            .or_insert((DefRef::Interface(m), fi));
                    }
                    D::Program(m) => {
                        self.defs
                            .entry(m.name.name.as_str())
                            .or_insert((DefRef::Program(m), fi));
                    }
                    D::Udp(u) => {
                        self.defs
                            .entry(u.name.name.as_str())
                            .or_insert((DefRef::Udp(u), fi));
                    }
                    D::Package(p) => {
                        self.packages.push((p, fi));
                        for it in &p.items {
                            if let PackageItem::Typedef(td) = it {
                                self.typedefs
                                    .entry(format!("{}::{}", p.name.name, td.name.name))
                                    .or_insert((td, fi));
                                self.typedefs
                                    .entry(td.name.name.clone())
                                    .or_insert((td, fi));
                            }
                        }
                    }
                    D::Bind(b) => self.binds.push((b, fi)),
                    D::TypedefDecl(td) => {
                        self.typedefs.insert(td.name.name.clone(), (td, fi));
                    }
                    D::PackageItem(PackageItem::Typedef(td)) => {
                        self.typedefs.insert(td.name.name.clone(), (td, fi));
                    }
                    _ => {}
                }
            }
        }
        for (i, inst) in self.sim.module.instances.iter().enumerate() {
            self.inst_by_path.insert(inst.path.as_str(), InstIdx(i));
        }
        for (k, v) in self.sim.module.parameters.iter() {
            if k.contains("::") {
                self.global_params.insert(k.clone(), v.clone());
                if let Some((_, leaf)) = k.rsplit_once("::") {
                    self.global_params
                        .entry(leaf.to_string())
                        .or_insert_with(|| v.clone());
                }
                continue;
            }
            let (prefix, leaf) = match k.rfind('.') {
                Some(i) => (&k[..i + 1], &k[i + 1..]),
                None => ("", k.as_str()),
            };
            self.params_by_prefix
                .entry(prefix.to_string())
                .or_default()
                .0
                .push((leaf.to_string(), v.clone()));
        }
        for (_, mods) in xezim_core::adopted_lib_files() {
            self.adopted.extend(mods);
        }
        for n in self.sim.module.implicit_nets.iter() {
            let (prefix, leaf) = match n.rfind('.') {
                Some(i) => (&n[..i + 1], &n[i + 1..]),
                None => ("", n.as_str()),
            };
            self.implicit_by_prefix
                .entry(prefix.to_string())
                .or_default()
                .0
                .push(leaf.to_string());
        }
    }

    /// §6.10 implicit nets of a scope: nets no declaration made.
    fn implicit_nets(&mut self, ctx: &Ctx) {
        let Some(Names(names)) = self.implicit_by_prefix.get(&ctx.flat_prefix).cloned() else {
            return;
        };
        let full = self.m.scopes[ctx.scope as usize].full.clone();
        for n in names {
            let vfull = format!("{}.{}", full, n);
            if self.m.by_name.contains_key(&vfull) {
                continue;
            }
            let flat = format!("{}{}", ctx.flat_prefix, n);
            let sig = self.sig_of(&flat);
            let w = sig
                .and_then(|id| self.sim.signal_widths.get(id).copied())
                .unwrap_or(1) as i64;
            let t = TDesc {
                kind: TKind::Implicit,
                signed: false,
                packed: if w > 1 { vec![(w - 1, 0)] } else { Vec::new() },
                unpacked: Vec::new(),
                name: None,
            };
            let td = self.m.add_td(t);
            self.m.add_obj(
                MObj {
                    type_code: c::NET,
                    name: n.clone(),
                    full: vfull,
                    scope: ctx.scope,
                    parent: NONE,
                    file: NONE,
                    line: 0,
                    d: OData::Var(VarData {
                        flat,
                        sig,
                        td,
                        net_type: c::WIRE,
                        dir: c::NO_DIRECTION,
                        automatic: false,
                        implicit: true,
                        is_const: false,
                        members: Vec::new(),
                        member_of: NONE,
                        lsb: 0,
                        width: 0,
                    }),
                },
                true,
                true,
            );
        }
    }

    /// Parameter values of the instance whose flat names start `prefix`.
    fn params_for(&self, prefix: &str) -> std::rc::Rc<HashMap<String, Value>> {
        let mut p = self.global_params.clone();
        if let Some(ParamList(own)) = self.params_by_prefix.get(prefix) {
            for (k, v) in own {
                p.insert(k.clone(), v.clone());
            }
        }
        std::rc::Rc::new(p)
    }

    fn eval(&self, e: &Expression, ctx: &Ctx) -> Option<i64> {
        xezim_core::elaborate::const_eval_i64_with_params(e, Some(&ctx.params))
    }

    /// `(file, line)` of a span in source file `file`, through the file's
    /// line map (the `include`d file and line a span's text came from; for
    /// macro text, the invocation). `(NONE, 0)` when it cannot be placed.
    fn loc(&mut self, span: Span, file: u32) -> (u32, u32) {
        if span.start == 0 && span.end == 0 {
            return (NONE, 0);
        }
        let sim = self.sim;
        let fi = file as usize;
        let nsrc = sim.module.source_texts.len();
        let text: &str = match sim.module.source_texts.get(fi) {
            Some(t) => t,
            None => match self.lib_texts.get(fi - nsrc) {
                Some((_, t, true)) => t,
                _ => return (NONE, 0),
            },
        };
        if span.start >= text.len() {
            return (NONE, 0);
        }
        let starts = self.line_starts[fi].get_or_insert_with(|| {
            let mut v = vec![0usize];
            v.extend(
                text.bytes()
                    .enumerate()
                    .filter(|&(_, b)| b == b'\n')
                    .map(|(i, _)| i + 1),
            );
            v
        });
        let out = starts
            .partition_point(|&s| s <= span.start)
            .saturating_sub(1);
        if fi >= nsrc {
            let path = self.lib_texts[fi - nsrc].0.clone();
            return (self.m.intern_file(&path), out as u32 + 1);
        }
        match sim.module.source_line_maps.get(fi).and_then(|m| m.as_ref()) {
            Some(map) => match map.file_line(out) {
                Some((f, l)) => (self.m.intern_file(f), l),
                None => (NONE, 0),
            },
            None => match sim.module.source_files.get(fi).filter(|f| !f.is_empty()) {
                Some(f) => (self.m.intern_file(f), out as u32 + 1),
                None => (NONE, 0),
            },
        }
    }

    fn flat_name(ctx: &Ctx, name: &str) -> String {
        format!("{}{}{}", ctx.flat_prefix, name, ctx.flat_suffix)
    }

    fn sig_of(&self, flat: &str) -> Option<usize> {
        self.sim.signal_name_to_id.get(flat).copied()
    }

    // --- design --------------------------------------------------------

    fn walk_design(&mut self) {
        let sim = self.sim;
        if sim.root_is_multi_top() {
            let tops: Vec<usize> = (0..sim.module.instances.len())
                .filter(|&i| sim.module.instances[i].parent.is_empty())
                .collect();
            for i in tops {
                let path = sim.module.instances[i].path.clone();
                let s = self.instance_scope(i as isize, &path, &path, NONE, None, None);
                self.m.scopes[s as usize].top = true;
                self.m.tops.push(s);
            }
        } else {
            let name = sim.module.name.clone();
            let s = self.instance_scope(-1, &name, "", NONE, None, None);
            self.m.scopes[s as usize].top = true;
            self.m.tops.push(s);
        }
        // Instances the definition walk did not reach (bound instances,
        // unresolved generate naming): attach them under their parent scope,
        // located by path, so the tree stays complete.
        for i in 0..sim.module.instances.len() {
            if self.attached[i] {
                continue;
            }
            self.attach_orphan(i);
        }
        // Packages, in source order, restricted to the ones the design uses.
        let pkgs = self.packages.clone();
        for (p, fi) in pkgs {
            if !sim.module.packages.contains(&p.name.name) {
                continue;
            }
            if self
                .m
                .packages
                .iter()
                .any(|&s| self.m.scopes[s as usize].name == p.name.name)
            {
                continue;
            }
            self.package_scope(p, fi);
        }
    }

    fn attach_orphan(&mut self, i: usize) {
        if self.attached[i] {
            return;
        }
        let sim = self.sim;
        let inst = &sim.module.instances[i];
        let parent_path = inst.parent.clone();
        // The parent: an instance by path, or the top.
        let parent_scope = if parent_path.is_empty() {
            self.m.tops.first().copied().unwrap_or(NONE)
        } else if let Some(&InstIdx(pi)) = self.inst_by_path.get(parent_path.as_str()) {
            self.attach_orphan(pi);
            self.m
                .inst_scope
                .get(&(pi as isize))
                .copied()
                .unwrap_or(NONE)
        } else {
            NONE
        };
        let path = inst.path.clone();
        let full = sim.hier_path(&path);
        // A generate scope named in the path between parent and leaf.
        let mut scope = parent_scope;
        if scope != NONE {
            let pfull = self.m.scopes[scope as usize].full.clone();
            if let Some(rest) = full.strip_prefix(&format!("{}.", pfull)) {
                let mut cur = pfull;
                let segs: Vec<&str> = rest.split('.').collect();
                for seg in &segs[..segs.len().saturating_sub(1)] {
                    cur = format!("{}.{}", cur, seg);
                    if let Some(MRef::Scope(s)) = self.m.by_name.get(&cur) {
                        scope = *s;
                    }
                }
            }
        }
        self.instance_scope(i as isize, &full, &format!("{}.", path), scope, None, None);
    }

    /// Make the scope of instance `inst_idx` and walk its definition.
    /// `flat` is the instance's flat signal prefix without the trailing dot
    /// ("" for the single top).
    fn instance_scope(
        &mut self,
        inst_idx: isize,
        full: &str,
        flat: &str,
        parent: u32,
        at: Option<(Span, u32)>,
        conns: Option<(&'a [PortConnection], u32)>,
    ) -> u32 {
        let sim = self.sim;
        let def_name = if inst_idx < 0 {
            sim.module.name.clone()
        } else {
            sim.module.instances[inst_idx as usize].def_name.clone()
        };
        if inst_idx >= 0 {
            self.attached[inst_idx as usize] = true;
        }
        let def = self.defs.get(def_name.as_str()).copied();
        let (kind, type_code) = match def.map(|d| d.0) {
            Some(DefRef::Interface(_)) => (SKind::Interface, c::INTERFACE),
            Some(DefRef::Program(_)) => (SKind::Program, c::PROGRAM),
            _ if sim.module.interfaces.contains(&def_name) => (SKind::Interface, c::INTERFACE),
            _ => (SKind::Module, c::MODULE),
        };
        let (def_file, def_line) = match def.and_then(|(d, f)| d.parts().map(|p| (p.4, f))) {
            Some((span, f)) => self.loc(span, f),
            None => (NONE, 0),
        };
        let (file, line) = match at {
            Some((span, f)) => self.loc(span, f),
            None => (def_file, def_line),
        };
        let name = full.rsplit('.').next().unwrap_or(full).to_string();
        let cell = self.adopted.contains(&def_name) || self.in_celldefine(def_file, def_line);
        let flat_prefix = if flat.is_empty() {
            String::new()
        } else {
            format!("{}.", flat.trim_end_matches('.'))
        };
        let s = self.m.add_scope(MScope {
            kind,
            type_code,
            name,
            full: full.to_string(),
            def_name: def_name.clone(),
            parent,
            inst_idx,
            file,
            line,
            def_file,
            def_line,
            children: Vec::new(),
            members: Vec::new(),
            gen_array: NONE,
            index: 0,
            implicit: false,
            automatic: false,
            cell,
            top: false,
            join: 0,
            func_type: 0,
            ret_td: NONE,
            dpi: None,
            flat_prefix: flat_prefix.clone(),
            flat_suffix: String::new(),
        });
        self.m.inst_scope.insert(inst_idx, s);
        let Some((def, fi)) = def else {
            self.flat_members(s, &flat_prefix);
            return s;
        };
        let Some((ports, params, items, lifetime, _)) = def.parts() else {
            return s;
        };
        if self.depth > 256 {
            return s;
        }
        self.depth += 1;
        let ctx = Ctx {
            scope: s,
            file: fi,
            flat_prefix: flat_prefix.clone(),
            flat_suffix: String::new(),
            inst_prefix: flat_prefix.clone(),
            params: self.params_for(&flat_prefix),
            automatic: lifetime == Some(Lifetime::Automatic),
            body_params_local: !params.is_empty(),
        };
        for p in params {
            self.param_decl(p, false, &ctx);
        }
        let mut port_decls = self.ansi_ports(ports, &ctx);
        let mut ord = 0u32;
        self.walk_items(items, &ctx, &mut ord, &mut port_decls);
        // §23.11: instances `bind` adds to this one, connected in its scope.
        let binds: Vec<(&'a BindDirective, u32)> = self
            .binds
            .iter()
            .copied()
            .filter(|(b, _)| {
                let paths = std::iter::once(&b.target_path).chain(b.extra_paths.iter());
                if b.target_path.is_empty() && b.extra_paths.is_empty() {
                    b.target_module.name == def_name
                } else {
                    paths.filter(|p| !p.is_empty()).any(|p| {
                        let joined: Vec<&str> = p.iter().map(|i| i.name.as_str()).collect();
                        joined.join(".") == full
                    })
                }
            })
            .collect();
        for (b, bf) in binds {
            let bctx = Ctx {
                file: bf,
                ..ctx.clone()
            };
            self.instantiation(&b.instantiation, &bctx);
        }
        self.implicit_nets(&ctx);
        self.make_ports(ports, &port_decls, &ctx, conns);
        self.depth -= 1;
        s
    }

    fn package_scope(&mut self, p: &'a PackageDeclaration, fi: u32) {
        let (file, line) = self.loc(p.span, fi);
        let name = p.name.name.clone();
        let s = self.m.add_scope(MScope {
            kind: SKind::Package,
            type_code: c::PACKAGE,
            name: name.clone(),
            full: format!("{}::", name),
            def_name: name.clone(),
            parent: NONE,
            inst_idx: -2,
            file,
            line,
            def_file: file,
            def_line: line,
            children: Vec::new(),
            members: Vec::new(),
            gen_array: NONE,
            index: 0,
            implicit: false,
            automatic: p.lifetime == Some(Lifetime::Automatic),
            cell: false,
            top: false,
            join: 0,
            func_type: 0,
            ret_td: NONE,
            dpi: None,
            flat_prefix: format!("{}::", name),
            flat_suffix: String::new(),
        });
        self.m.packages.push(s);
        // `vpi_handle_by_name("pkg", NULL)` as well as `"pkg::"`.
        self.m.by_name.entry(name.clone()).or_insert(MRef::Scope(s));
        let ctx = Ctx {
            scope: s,
            file: fi,
            flat_prefix: format!("{}::", name),
            flat_suffix: String::new(),
            inst_prefix: String::new(),
            params: std::rc::Rc::new(self.global_params.clone()),
            automatic: p.lifetime == Some(Lifetime::Automatic),
            body_params_local: false,
        };
        for it in &p.items {
            match it {
                PackageItem::Parameter(pd) => self.param_decl(pd, false, &ctx),
                PackageItem::Data(dd) => self.data_decl(dd, &ctx, &mut Vec::new()),
                PackageItem::Function(fd) => self.function(fd, &ctx, None),
                PackageItem::Task(td) => self.task(td, &ctx, None),
                PackageItem::DPIImport(di) => self.dpi_import(di, &ctx),
                _ => {}
            }
        }
    }

    /// Objects of a scope whose definition has no AST: everything in the
    /// signal table directly under `prefix`, typed from the elaborated tables.
    fn flat_members(&mut self, s: u32, prefix: &str) {
        let sim = self.sim;
        let scope_path = prefix.trim_end_matches('.');
        let full = self.m.scopes[s as usize].full.clone();
        let mut names: Vec<(String, usize)> = vpi_scope_members(sim, scope_path);
        names.sort();
        for (flat, id) in names {
            let leaf = flat.rsplit('.').next().unwrap_or(&flat).to_string();
            // An unpacked array has no whole-array signal: its storage is its
            // first element.
            let arr = if id == usize::MAX {
                sim.module.arrays.get(flat.as_str()).copied()
            } else {
                None
            };
            let (ty, id) = match arr {
                Some((lo, _, _)) => (
                    c::REG_ARRAY,
                    self.sig_of(&format!("{}[{}]", flat, lo))
                        .unwrap_or(usize::MAX),
                ),
                None if id == usize::MAX => continue,
                None => (vpi_type_of(sim, &flat, id), id),
            };
            let td = self.m.add_td(TDesc {
                kind: TKind::Unknown,
                signed: id != usize::MAX && sim.signal_signed.get(id).copied().unwrap_or(false),
                packed: Vec::new(),
                unpacked: arr
                    .map(|(lo, hi, _)| vec![UDim::Range(lo, hi)])
                    .unwrap_or_default(),
                name: None,
            });
            let d = if ty == c::PARAMETER {
                OData::Param {
                    flat: flat.clone(),
                    sig: Some(id),
                    td,
                    local: false,
                    const_type: c::DEC_CONST,
                }
            } else {
                OData::Var(VarData {
                    flat: flat.clone(),
                    sig: (id != usize::MAX).then_some(id),
                    td,
                    net_type: if ty == c::NET { c::WIRE } else { 0 },
                    dir: vpi_direction_of(sim, &flat),
                    automatic: false,
                    implicit: false,
                    is_const: false,
                    members: Vec::new(),
                    member_of: NONE,
                    lsb: 0,
                    width: 0,
                })
            };
            self.m.add_obj(
                MObj {
                    type_code: ty,
                    name: leaf.clone(),
                    full: format!("{}.{}", full, leaf),
                    scope: s,
                    parent: NONE,
                    file: NONE,
                    line: 0,
                    d,
                },
                true,
                true,
            );
        }
    }

    /// `celldefine regions of a source file, as line ranges.
    fn in_celldefine(&mut self, file: u32, line: u32) -> bool {
        if file == NONE || line == 0 {
            return false;
        }
        let path = self.m.files[file as usize].clone();
        let regions = self.cell_regions.entry(path.clone()).or_insert_with(|| {
            let mut out = Vec::new();
            let Ok(text) = std::fs::read_to_string(&path) else {
                return Regions(out);
            };
            let mut open: Option<u32> = None;
            for (i, l) in text.lines().enumerate() {
                let t = l.trim_start();
                if t.starts_with("`celldefine") {
                    open = Some(i as u32 + 1);
                } else if t.starts_with("`endcelldefine") {
                    if let Some(s) = open.take() {
                        out.push((s, i as u32 + 1));
                    }
                }
            }
            if let Some(s) = open {
                out.push((s, u32::MAX));
            }
            Regions(out)
        });
        regions.0.iter().any(|&(a, b)| line > a && line < b)
    }

    // --- scope items ---------------------------------------------------

    fn walk_items(
        &mut self,
        items: &'a [ModuleItem],
        ctx: &Ctx,
        ord: &mut u32,
        ports: &mut Vec<PortDecl>,
    ) {
        for it in items {
            match it {
                ModuleItem::PortDeclaration(pd) => self.port_decl(pd, ctx, ports),
                ModuleItem::NetDeclaration(nd) => self.net_decl(nd, ctx),
                ModuleItem::DataDeclaration(dd) => self.data_decl(dd, ctx, ports),
                ModuleItem::ParameterDeclaration(p) => {
                    self.param_decl(p, ctx.body_params_local, ctx)
                }
                ModuleItem::LocalparamDeclaration(p) => self.param_decl(p, true, ctx),
                ModuleItem::AlwaysConstruct(ac) => {
                    let at = match ac.kind {
                        AlwaysKind::Always => c::ALWAYS,
                        AlwaysKind::AlwaysComb => c::ALWAYS_COMB,
                        AlwaysKind::AlwaysFf => c::ALWAYS_FF,
                        AlwaysKind::AlwaysLatch => c::ALWAYS_LATCH,
                    };
                    self.process(c::ALWAYS, at, &ac.stmt, ac.span, ctx);
                }
                ModuleItem::InitialConstruct(ic) => {
                    self.process(c::INITIAL, 0, &ic.stmt, ic.span, ctx)
                }
                ModuleItem::FinalConstruct(fc) => self.process(c::FINAL, 0, &fc.stmt, fc.span, ctx),
                ModuleItem::ContinuousAssign(ca) => self.cont_assign(ca, ctx),
                ModuleItem::ModuleInstantiation(mi) => self.instantiation(mi, ctx),
                ModuleItem::GateInstantiation(gi) => self.gates(gi, ctx),
                ModuleItem::GenerateRegion(gr) => self.walk_items(&gr.items, ctx, ord, ports),
                ModuleItem::GenerateIf(gi) => {
                    *ord += 1;
                    let labels: Vec<Option<&String>> =
                        gi.branch_labels.iter().map(|l| l.as_ref()).collect();
                    self.placeholder_labels(&labels, *ord, ctx);
                    self.generate_if(gi, ctx, *ord);
                }
                ModuleItem::GenerateCase(gc) => {
                    *ord += 1;
                    let labels: Vec<Option<&String>> =
                        gc.arms.iter().map(|a| a.label.as_ref()).collect();
                    self.placeholder_labels(&labels, *ord, ctx);
                    self.generate_case(gc, ctx, *ord);
                }
                ModuleItem::GenerateFor(gf) => {
                    *ord += 1;
                    self.placeholder_labels(&[gf.name.as_ref()], *ord, ctx);
                    self.generate_for(gf, ctx, *ord);
                }
                ModuleItem::FunctionDeclaration(fd) => self.function(fd, ctx, None),
                ModuleItem::TaskDeclaration(td) => self.task(td, ctx, None),
                ModuleItem::DPIImport(di) => self.dpi_import(di, ctx),
                ModuleItem::SpecifyBlock(sb) => self.specify(sb, ctx),
                ModuleItem::ModportDeclaration(md) => self.modports(md, ctx),
                _ => {}
            }
        }
    }

    /// Flat prefix of the instance a walk is in.
    fn inst_flat_prefix(&self, ctx: &Ctx) -> String {
        match self.m.instance_of(ctx.scope) {
            NONE => ctx.flat_prefix.clone(),
            i => self.m.scopes[i as usize].flat_prefix.clone(),
        }
    }

    /// The placeholder signals of generate block names at this level.
    fn placeholder_labels(&mut self, labels: &[Option<&String>], ord: u32, ctx: &Ctx) {
        let mut names: Vec<String> = labels.iter().flatten().map(|l| (*l).clone()).collect();
        names.push(format!("genblk{}", ord));
        let inst_prefix = self.inst_flat_prefix(ctx);
        for n in names {
            self.m
                .placeholders
                .insert(format!("{}{}", ctx.flat_prefix, n));
            self.m.placeholders.insert(format!("{}{}", inst_prefix, n));
        }
    }

    // --- declarations --------------------------------------------------

    /// Resolve a data type into a descriptor, following typedefs.
    fn tdesc(&mut self, dt: &DataType, unpacked: &[UnpackedDimension], ctx: &Ctx) -> TDesc {
        let mut t = self.tdesc_inner(dt, ctx, 0);
        let mut ud: Vec<UDim> = unpacked.iter().map(|d| self.udim(d, ctx)).collect();
        ud.extend(t.unpacked.drain(..));
        t.unpacked = ud;
        t
    }

    fn udim(&self, d: &UnpackedDimension, ctx: &Ctx) -> UDim {
        match d {
            UnpackedDimension::Range { left, right, .. } => {
                match (self.eval(left, ctx), self.eval(right, ctx)) {
                    (Some(l), Some(r)) => UDim::Range(l, r),
                    _ => UDim::Range(0, 0),
                }
            }
            UnpackedDimension::Expression { expr, .. } => match self.eval(expr, ctx) {
                // `[N]` is `[0:N-1]`.
                Some(n) if n > 0 => UDim::Range(0, n - 1),
                _ => UDim::Range(0, 0),
            },
            UnpackedDimension::Unsized(_) => UDim::Dynamic,
            UnpackedDimension::Queue { .. } => UDim::Queue,
            UnpackedDimension::Associative { data_type, .. } => {
                // `[N]` with a constant name parses as associative.
                if let Some(dt) = data_type {
                    if let DataType::TypeReference { name, .. } = dt.as_ref() {
                        if name.scopes.is_empty() {
                            if let Some(n) =
                                ctx.params.get(&name.name.name).and_then(|v| v.to_i64())
                            {
                                if n > 0 && !self.typedefs.contains_key(&name.name.name) {
                                    return UDim::Range(0, n - 1);
                                }
                            }
                        }
                    }
                }
                UDim::Assoc
            }
        }
    }

    fn packed_dims(&self, dims: &[PackedDimension], ctx: &Ctx) -> Vec<(i64, i64)> {
        dims.iter()
            .filter_map(|d| match d {
                PackedDimension::Range { left, right, .. } => {
                    Some((self.eval(left, ctx)?, self.eval(right, ctx)?))
                }
                PackedDimension::Unsized(_) => None,
            })
            .collect()
    }

    fn tdesc_inner(&mut self, dt: &DataType, ctx: &Ctx, depth: u32) -> TDesc {
        let signed_of = |s: &Option<Signing>, default: bool| match s {
            Some(Signing::Signed) => true,
            Some(Signing::Unsigned) => false,
            None => default,
        };
        let mk = |kind: TKind, signed: bool, packed: Vec<(i64, i64)>| TDesc {
            kind,
            signed,
            packed,
            unpacked: Vec::new(),
            name: None,
        };
        match dt {
            DataType::IntegerVector {
                kind,
                signing,
                dimensions,
                ..
            } => {
                let k = match kind {
                    IntegerVectorType::Logic => TKind::Logic,
                    IntegerVectorType::Reg => TKind::Reg,
                    IntegerVectorType::Bit => TKind::Bit,
                };
                mk(
                    k,
                    signed_of(signing, false),
                    self.packed_dims(dimensions, ctx),
                )
            }
            DataType::IntegerAtom { kind, signing, .. } => {
                let k = match kind {
                    IntegerAtomType::Byte => TKind::Byte,
                    IntegerAtomType::ShortInt => TKind::ShortInt,
                    IntegerAtomType::Int => TKind::Int,
                    IntegerAtomType::LongInt => TKind::LongInt,
                    IntegerAtomType::Integer => TKind::Integer,
                    IntegerAtomType::Time => TKind::Time,
                };
                let default_signed = !matches!(kind, IntegerAtomType::Time);
                mk(k, signed_of(signing, default_signed), Vec::new())
            }
            DataType::Real { kind, .. } => match kind {
                RealType::ShortReal => mk(TKind::ShortReal, false, Vec::new()),
                _ => mk(TKind::Real, false, Vec::new()),
            },
            DataType::Simple { kind, .. } => match kind {
                SimpleType::String => mk(TKind::String, false, Vec::new()),
                SimpleType::Chandle => mk(TKind::Chandle, false, Vec::new()),
                SimpleType::Event => mk(TKind::Event, false, Vec::new()),
            },
            DataType::Struct(su) => {
                let key = AstKey(su as *const _ as usize);
                let idx = match self.struct_memo.get(&key) {
                    Some(&i) => i,
                    None => {
                        let mut members = Vec::new();
                        for m in &su.members {
                            for d in &m.declarators {
                                let mut t = self.tdesc_inner(&m.data_type, ctx, depth + 1);
                                let mut ud: Vec<UDim> =
                                    d.dimensions.iter().map(|x| self.udim(x, ctx)).collect();
                                ud.extend(t.unpacked.drain(..));
                                t.unpacked = ud;
                                let (_, line) = self.loc(d.span, ctx.file);
                                members.push((d.name.name.clone(), t, line));
                            }
                        }
                        self.m.structs.push(StructDesc {
                            union: su.kind == StructUnionKind::Union,
                            packed: su.packed,
                            tagged: su.tagged,
                            members,
                        });
                        let i = (self.m.structs.len() - 1) as u32;
                        self.struct_memo.insert(key, i);
                        i
                    }
                };
                mk(
                    TKind::Struct(idx),
                    signed_of(&su.signing, false),
                    self.packed_dims(&su.dimensions, ctx),
                )
            }
            DataType::Enum(et) => {
                let key = AstKey(et as *const _ as usize);
                let idx = match self.enum_memo.get(&key) {
                    Some(&i) => i,
                    None => {
                        let base = match &et.base_type {
                            Some(b) => self.tdesc_inner(b, ctx, depth + 1),
                            None => mk(TKind::Int, true, Vec::new()),
                        };
                        let width = self.m.td_width(&base).max(1);
                        let mut consts = Vec::new();
                        let mut next: i64 = 0;
                        for mem in &et.members {
                            let v = match &mem.init {
                                Some(e) => self.eval(e, ctx).unwrap_or(next),
                                None => next,
                            };
                            let mask = if width >= 64 {
                                u64::MAX
                            } else {
                                (1u64 << width) - 1
                            };
                            consts.push((mem.name.name.clone(), (v as u64) & mask));
                            next = v.wrapping_add(1);
                        }
                        self.m.enums.push(EnumDesc { base, consts });
                        let i = (self.m.enums.len() - 1) as u32;
                        self.enum_memo.insert(key, i);
                        i
                    }
                };
                let signed = self.m.enums[idx as usize].base.signed;
                mk(
                    TKind::Enum(idx),
                    signed,
                    self.packed_dims(&et.dimensions, ctx),
                )
            }
            DataType::Void(_) => mk(TKind::Void, false, Vec::new()),
            DataType::Implicit {
                signing,
                dimensions,
                ..
            } => mk(
                TKind::Implicit,
                signed_of(signing, false),
                self.packed_dims(dimensions, ctx),
            ),
            DataType::Interface { name, .. } => {
                mk(TKind::VirtualIf(name.name.clone()), false, Vec::new())
            }
            DataType::TypeReference {
                name, dimensions, ..
            } => {
                let outer = self.packed_dims(dimensions, ctx);
                let key = if name.scopes.is_empty() {
                    name.name.name.clone()
                } else {
                    name.qualified()
                };
                if depth < 16 {
                    if let Some(&(td, _)) = self
                        .typedefs
                        .get(&key)
                        .or_else(|| self.typedefs.get(&name.name.name))
                    {
                        let mut t = self.tdesc_inner(&td.data_type, ctx, depth + 1);
                        let ud: Vec<UDim> =
                            td.dimensions.iter().map(|x| self.udim(x, ctx)).collect();
                        t.unpacked.splice(0..0, ud);
                        if t.name.is_none() {
                            t.name = Some(name.name.name.clone());
                        }
                        if !outer.is_empty() {
                            let mut p = outer;
                            p.extend(t.packed.drain(..));
                            t.packed = p;
                        }
                        return t;
                    }
                    if let Some(dt2) = self.sim.module.typedef_types.get(&key).cloned() {
                        let mut t = self.tdesc_inner(&dt2, ctx, depth + 1);
                        if t.name.is_none() {
                            t.name = Some(name.name.name.clone());
                        }
                        if !outer.is_empty() {
                            let mut p = outer;
                            p.extend(t.packed.drain(..));
                            t.packed = p;
                        }
                        return t;
                    }
                }
                if self.sim.module.classes.contains_key(&name.name.name) {
                    return mk(TKind::Class(name.name.name.clone()), false, Vec::new());
                }
                if self.sim.module.interfaces.contains(&name.name.name) {
                    return mk(TKind::VirtualIf(name.name.name.clone()), false, Vec::new());
                }
                let mut t = mk(TKind::Unknown, false, outer);
                t.name = Some(name.name.name.clone());
                t
            }
        }
    }

    /// The flat storage of a declaration: its signal and, for an array, the
    /// signal of its first element.
    fn storage(&self, flat: &str, t: &TDesc) -> Option<usize> {
        if t.unpacked.is_empty() {
            return self.sig_of(flat);
        }
        let sim = self.sim;
        if let Some(&(lo, _, _)) = sim.module.arrays.get(flat) {
            return self.sig_of(&format!("{}[{}]", flat, lo));
        }
        // First element of each dimension.
        let mut name = flat.to_string();
        for d in &t.unpacked {
            match d {
                UDim::Range(l, r) => name = format!("{}[{}]", name, (*l).min(*r)),
                _ => name = format!("{}[0]", name),
            }
            if let Some(id) = self.sig_of(&name) {
                return Some(id);
            }
        }
        None
    }

    /// Make (or complete) a variable or net `name` declared in `ctx.scope`.
    #[allow(clippy::too_many_arguments)]
    fn declare(
        &mut self,
        name: &Identifier,
        t: TDesc,
        net_type: c_int,
        dir: c_int,
        is_const: bool,
        automatic: bool,
        span: Span,
        ctx: &Ctx,
    ) -> u32 {
        let scope_full = self.m.scopes[ctx.scope as usize].full.clone();
        let full = if self.m.scopes[ctx.scope as usize].kind == SKind::Package {
            format!("{}{}", scope_full, name.name)
        } else {
            format!("{}.{}", scope_full, name.name)
        };
        let mut flat = Self::flat_name(ctx, &name.name);
        let mut sig = self.storage(&flat, &t);
        if sig.is_none() && self.m.scopes[ctx.scope as usize].kind == SKind::Package {
            // Package variables live in the design-wide namespace.
            for cand in [
                format!("{}.{}", self.m.scopes[ctx.scope as usize].name, name.name),
                name.name.clone(),
            ] {
                if let Some(id) = self.storage(&cand, &t) {
                    flat = cand;
                    sig = Some(id);
                    break;
                }
            }
        }
        let (file, line) = self.loc(if span.start == 0 { name.span } else { span }, ctx.file);
        // A non-ANSI port completed by a later net/variable declaration.
        if let Some(MRef::Obj(o)) = self.m.by_name.get(&full).copied() {
            let keep_dir;
            {
                let ob = &self.m.objs[o as usize];
                keep_dir = match &ob.d {
                    OData::Var(v) => v.dir,
                    _ => return o,
                };
            }
            let dir = if dir == c::NO_DIRECTION {
                keep_dir
            } else {
                dir
            };
            let net = if net_type != 0 {
                net_type
            } else if matches!(t.kind, TKind::Implicit) {
                match &self.m.objs[o as usize].d {
                    OData::Var(v) => v.net_type,
                    _ => 0,
                }
            } else {
                0
            };
            let old_td = match &self.m.objs[o as usize].d {
                OData::Var(v) => v.td,
                _ => NONE,
            };
            // Keep the port's range when the completing declaration is bare.
            let t = if matches!(t.kind, TKind::Implicit) && t.packed.is_empty() {
                let mut old = self.m.tdescs[old_td as usize].clone();
                old.unpacked = t.unpacked;
                old
            } else {
                t
            };
            let ty = if net != 0 {
                self.m.net_type_code(&t)
            } else {
                self.m.var_type_code(&t)
            };
            let td = self.m.add_td(t);
            let ob = &mut self.m.objs[o as usize];
            ob.type_code = ty;
            if let OData::Var(v) = &mut ob.d {
                v.td = td;
                v.net_type = net;
                v.dir = dir;
                v.is_const |= is_const;
            }
            return o;
        }
        let ty = if net_type != 0 {
            self.m.net_type_code(&t)
        } else {
            self.m.var_type_code(&t)
        };
        let struct_idx = match &t.kind {
            TKind::Struct(s) if t.packed.is_empty() && t.unpacked.is_empty() => Some(*s),
            _ => None,
        };
        let td = self.m.add_td(t);
        let o = self.m.add_obj(
            MObj {
                type_code: ty,
                name: name.name.clone(),
                full: full.clone(),
                scope: ctx.scope,
                parent: NONE,
                file,
                line,
                d: OData::Var(VarData {
                    flat: flat.clone(),
                    sig,
                    td,
                    net_type,
                    dir,
                    automatic,
                    implicit: false,
                    is_const,
                    members: Vec::new(),
                    member_of: NONE,
                    lsb: 0,
                    width: 0,
                }),
            },
            true,
            true,
        );
        if let Some(si) = struct_idx {
            self.struct_members(o, si, &flat, &full, net_type != 0, ctx);
        }
        o
    }

    /// Member objects of a struct/union variable.
    fn struct_members(&mut self, o: u32, si: u32, flat: &str, full: &str, net: bool, ctx: &Ctx) {
        let sd = self.m.structs[si as usize].clone();
        let fields = self.sim.module.packed_struct_fields.get(flat).cloned();
        let mut members = Vec::new();
        let total = if sd.packed {
            let t = TDesc {
                kind: TKind::Struct(si),
                signed: false,
                packed: Vec::new(),
                unpacked: Vec::new(),
                name: None,
            };
            self.m.td_width(&t)
        } else {
            0
        };
        // Packed members are laid out MSB-first.
        let mut hi = total;
        for (mname, mt, line) in &sd.members {
            let mw = self.m.td_width(mt);
            let mflat = format!("{}.{}", flat, mname);
            let mfull = format!("{}.{}", full, mname);
            let (lsb, width) = if sd.packed {
                match fields
                    .as_ref()
                    .and_then(|f| f.iter().find(|(n, _, _)| n == mname))
                {
                    Some(&(_, l, w)) => (l, w),
                    None if sd.union => (0, mw),
                    None => {
                        let l = hi.saturating_sub(mw);
                        (l, mw)
                    }
                }
            } else {
                (0, 0)
            };
            if sd.packed && !sd.union {
                hi = hi.saturating_sub(mw);
            }
            let sig = if sd.packed {
                None
            } else {
                self.storage(&mflat, mt)
            };
            let ty = if net {
                self.m.net_type_code(mt)
            } else {
                self.m.var_type_code(mt)
            };
            let nested = match &mt.kind {
                TKind::Struct(s) if mt.packed.is_empty() && mt.unpacked.is_empty() => Some(*s),
                _ => None,
            };
            let td = self.m.add_td(mt.clone());
            let file = self.m.objs[o as usize].file;
            let mo = self.m.add_obj(
                MObj {
                    type_code: ty,
                    name: mname.clone(),
                    full: mfull.clone(),
                    scope: ctx.scope,
                    parent: o,
                    file,
                    line: *line,
                    d: OData::Var(VarData {
                        flat: mflat.clone(),
                        sig,
                        td,
                        net_type: if net { c::WIRE } else { 0 },
                        dir: c::NO_DIRECTION,
                        automatic: false,
                        implicit: false,
                        is_const: false,
                        members: Vec::new(),
                        member_of: if sd.packed { o } else { NONE },
                        lsb,
                        width,
                    }),
                },
                true,
                false,
            );
            if let Some(ns) = nested {
                if !sd.packed {
                    self.struct_members(mo, ns, &mflat, &mfull, net, ctx);
                }
            }
            members.push(mo);
        }
        if let OData::Var(v) = &mut self.m.objs[o as usize].d {
            v.members = members;
        }
    }

    fn net_type_code(nt: NetType) -> c_int {
        match nt {
            NetType::Wire => c::WIRE,
            NetType::Tri => c::TRI,
            NetType::Wand => c::WAND,
            NetType::Wor => c::WOR,
            NetType::TriAnd => c::TRIAND,
            NetType::TriOr => c::TRIOR,
            NetType::Tri0 => c::TRI0,
            NetType::Tri1 => c::TRI1,
            NetType::Supply0 => c::SUPPLY0,
            NetType::Supply1 => c::SUPPLY1,
            NetType::TriReg => c::TRIREG,
            NetType::Uwire => c::UWIRE,
            NetType::Interconnect | NetType::Wreal => c::NONE_NET,
        }
    }

    fn dir_code(d: PortDirection) -> c_int {
        match d {
            PortDirection::Input => c::INPUT,
            PortDirection::Output => c::OUTPUT,
            PortDirection::Inout => c::INOUT,
            PortDirection::Ref => c::REF,
        }
    }

    /// §23.2.2.3: the kind of a port whose declaration names no net type
    /// and no `var`: inputs and inouts are nets when their data type can be
    /// one, outputs are nets only when the data type is implicit.
    fn port_is_net(&self, dir: c_int, t: &TDesc, explicit_type: bool) -> bool {
        match dir {
            c::OUTPUT => !explicit_type,
            c::REF => false,
            _ => self.m.td_four_state(t),
        }
    }

    fn ansi_ports(&mut self, ports: &'a PortList, ctx: &Ctx) -> Vec<PortDecl> {
        let mut out = Vec::new();
        let PortList::Ansi(list) = ports else {
            return out;
        };
        let mut prev_dir = c::INOUT;
        let mut prev: Option<(Option<NetType>, bool, Option<&'a DataType>)> = None;
        for p in list {
            let dir = p.direction.map(Self::dir_code).unwrap_or(prev_dir);
            // Direction, kind and type all omitted: inherited (§23.2.2.3).
            let inherit =
                p.direction.is_none() && p.net_type.is_none() && !p.var_kw && p.data_type.is_none();
            let (net_type, var_kw, dt) = if inherit {
                prev.unwrap_or((None, false, None))
            } else {
                (p.net_type, p.var_kw, p.data_type.as_ref())
            };
            prev_dir = dir;
            prev = Some((net_type, var_kw, dt));
            // Interface ports.
            let iface = match dt {
                Some(DataType::Interface { name, modport, .. }) => {
                    Some((name.name.clone(), modport.is_some()))
                }
                Some(DataType::TypeReference { name, .. })
                    if matches!(
                        self.defs.get(name.name.name.as_str()),
                        Some((DefRef::Interface(_), _))
                    ) =>
                {
                    Some((name.name.name.clone(), false))
                }
                _ => None,
            };
            if let Some((_, modport)) = iface {
                out.push(PortDecl {
                    name: p.name.name.clone(),
                    dir: c::NO_DIRECTION,
                    port_type: if modport {
                        c::MODPORT_PORT
                    } else {
                        c::INTERFACE_PORT
                    },
                });
                continue;
            }
            let implicit = DataType::Implicit {
                signing: None,
                dimensions: Vec::new(),
                span: Span::dummy(),
            };
            let t = self.tdesc(dt.unwrap_or(&implicit), &p.dimensions, ctx);
            let explicit_type = !matches!(dt, None | Some(DataType::Implicit { .. }));
            let nt = match net_type {
                Some(nt) => Self::net_type_code(nt),
                None if var_kw => 0,
                None if self.port_is_net(dir, &t, explicit_type) => c::WIRE,
                None => 0,
            };
            self.declare(&p.name, t, nt, dir, false, false, p.span, ctx);
            out.push(PortDecl {
                name: p.name.name.clone(),
                dir,
                port_type: c::PORT,
            });
        }
        out
    }

    /// Non-ANSI `input [3:0] a;` in the body.
    fn port_decl(&mut self, pd: &'a PortDeclaration, ctx: &Ctx, ports: &mut Vec<PortDecl>) {
        let dir = Self::dir_code(pd.direction);
        let explicit_type = !matches!(pd.data_type, DataType::Implicit { .. });
        for d in &pd.declarators {
            let t = self.tdesc(&pd.data_type, &d.dimensions, ctx);
            // A non-ANSI declaration with a data type and no net type
            // (`input logic e;`) declares a variable, as the reference
            // simulator reports it; only an implicit type makes a net.
            let nt = match pd.net_type {
                Some(nt) => Self::net_type_code(nt),
                None if !explicit_type && dir != c::REF => c::WIRE,
                None => 0,
            };
            self.declare(&d.name, t, nt, dir, false, false, pd.span, ctx);
            match ports.iter_mut().find(|p| p.name == d.name.name) {
                Some(p) => p.dir = dir,
                None => ports.push(PortDecl {
                    name: d.name.name.clone(),
                    dir,
                    port_type: c::PORT,
                }),
            }
        }
    }

    fn net_decl(&mut self, nd: &'a NetDeclaration, ctx: &Ctx) {
        let nt = Self::net_type_code(nd.net_type);
        for d in &nd.declarators {
            let t = self.tdesc(&nd.data_type, &d.dimensions, ctx);
            let o = self.declare(&d.name, t, nt, c::NO_DIRECTION, false, false, d.span, ctx);
            if let Some(init) = &d.init {
                // §10.3.1 net declaration assignment.
                let lhs = self.var_expr(o);
                let rhs = self.expr_obj(init, ctx.scope, ctx.file);
                let delay = match &nd.delay {
                    Some(e) => self.expr_obj(e, ctx.scope, ctx.file),
                    None => NONE,
                };
                let (file, line) = self.loc(d.span, ctx.file);
                self.m.add_obj(
                    MObj {
                        type_code: c::CONT_ASSIGN,
                        name: String::new(),
                        full: String::new(),
                        scope: ctx.scope,
                        parent: NONE,
                        file,
                        line,
                        d: OData::ContAssign {
                            lhs,
                            rhs,
                            delay,
                            net_decl: true,
                        },
                    },
                    false,
                    true,
                );
            }
        }
    }

    fn data_decl(&mut self, dd: &'a DataDeclaration, ctx: &Ctx, ports: &mut Vec<PortDecl>) {
        let automatic = match dd.lifetime {
            Some(Lifetime::Automatic) => true,
            Some(Lifetime::Static) => false,
            None => ctx.automatic,
        };
        for d in &dd.declarators {
            let t = self.tdesc(&dd.data_type, &d.dimensions, ctx);
            let is_port = ports.iter().any(|p| p.name == d.name.name);
            self.declare(
                &d.name,
                t,
                0,
                c::NO_DIRECTION,
                dd.const_kw,
                automatic && !is_port,
                d.span,
                ctx,
            );
        }
    }

    fn param_decl(&mut self, p: &'a ParameterDeclaration, local: bool, ctx: &Ctx) {
        let ParameterKind::Data {
            data_type,
            assignments,
        } = &p.kind
        else {
            return;
        };
        let local = local || p.local;
        for a in assignments {
            self.param(a, data_type, local, ctx);
        }
    }

    fn param(&mut self, a: &'a ParamAssignment, dt: &DataType, local: bool, ctx: &Ctx) {
        let sc = &self.m.scopes[ctx.scope as usize];
        let is_pkg = sc.kind == SKind::Package;
        let full = if is_pkg {
            format!("{}{}", sc.full, a.name.name)
        } else {
            format!("{}.{}", sc.full, a.name.name)
        };
        if self.m.by_name.contains_key(&full) {
            return;
        }
        let mut flat = Self::flat_name(ctx, &a.name.name);
        let mut sig = self.sig_of(&flat);
        if sig.is_none() && is_pkg {
            let bare = a.name.name.clone();
            if let Some(id) = self.sig_of(&bare) {
                flat = bare;
                sig = Some(id);
            }
        }
        let t = self.tdesc(dt, &a.dimensions, ctx);
        let const_type = match &t.kind {
            TKind::Real | TKind::ShortReal => c::REAL_CONST,
            TKind::String => c::STRING_CONST,
            TKind::Time => c::TIME_CONST,
            TKind::Int | TKind::ShortInt | TKind::LongInt | TKind::Byte => c::INT_CONST,
            _ => match a.init.as_ref().map(|e| &e.kind) {
                Some(ExprKind::Number(NumberLiteral::Integer { base, size, .. }))
                    if size.is_some() || *base != NumberBase::Decimal =>
                {
                    match base {
                        NumberBase::Binary => c::BINARY_CONST,
                        NumberBase::Octal => c::OCT_CONST,
                        NumberBase::Hex => c::HEX_CONST,
                        NumberBase::Decimal => c::DEC_CONST,
                    }
                }
                Some(ExprKind::Number(NumberLiteral::Real(_))) => c::REAL_CONST,
                Some(ExprKind::StringLiteral(_)) => c::STRING_CONST,
                _ => c::DEC_CONST,
            },
        };
        let td = self.m.add_td(t);
        let (file, line) = self.loc(a.span, ctx.file);
        self.m.add_obj(
            MObj {
                type_code: c::PARAMETER,
                name: a.name.name.clone(),
                full,
                scope: ctx.scope,
                parent: NONE,
                file,
                line,
                d: OData::Param {
                    flat,
                    sig,
                    td,
                    local,
                    const_type,
                },
            },
            true,
            true,
        );
    }

    // --- expressions ---------------------------------------------------

    fn expr_obj(&mut self, e: &Expression, scope: u32, file: u32) -> u32 {
        let (f, line) = self.loc(e.span, file);
        let ty = match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => c::CONSTANT,
            ExprKind::Ident(_) => c::REF_OBJ,
            ExprKind::Index { .. } => c::BIT_SELECT,
            ExprKind::RangeSelect { .. } => c::PART_SELECT,
            _ => c::OPERATION,
        };
        self.m.add_obj(
            MObj {
                type_code: ty,
                name: String::new(),
                full: String::new(),
                scope,
                parent: NONE,
                file: f,
                line,
                d: OData::Expr(ExprData {
                    scope,
                    expr: Box::new(e.clone()),
                }),
            },
            false,
            false,
        )
    }

    /// An expression object that stands for variable `o` itself.
    fn var_expr(&mut self, o: u32) -> u32 {
        let ob = &self.m.objs[o as usize];
        let e = Expression::new(
            ExprKind::Ident(HierarchicalIdentifier {
                root: None,
                path: vec![HierPathSegment {
                    name: Identifier {
                        name: ob.name.clone(),
                        span: Span::dummy(),
                    },
                    selects: Vec::new(),
                }],
                span: Span::dummy(),
                cached_signal_id: Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            Span::dummy(),
        );
        let scope = ob.scope;
        let file = ob.file;
        let line = ob.line;
        self.m.add_obj(
            MObj {
                type_code: c::REF_OBJ,
                name: String::new(),
                full: String::new(),
                scope,
                parent: NONE,
                file,
                line,
                d: OData::Expr(ExprData {
                    scope,
                    expr: Box::new(e),
                }),
            },
            false,
            false,
        )
    }

    // --- processes, assignments, primitives ----------------------------

    fn process(
        &mut self,
        ty: c_int,
        always_type: c_int,
        stmt: &'a Statement,
        span: Span,
        ctx: &Ctx,
    ) {
        let (file, line) = self.loc(span, ctx.file);
        let p = self.m.add_obj(
            MObj {
                type_code: ty,
                name: String::new(),
                full: String::new(),
                scope: ctx.scope,
                parent: NONE,
                file,
                line,
                d: OData::Process {
                    always_type,
                    stmt: NONE,
                },
            },
            false,
            true,
        );
        let top = self.m.scopes.len() as u32;
        self.walk_stmt(stmt, ctx.scope, ctx);
        // The process's own statement when it is a named block.
        if matches!(
            &stmt.kind,
            StatementKind::SeqBlock { name: Some(_), .. }
                | StatementKind::ParBlock { name: Some(_), .. }
        ) && (top as usize) < self.m.scopes.len()
        {
            if let OData::Process { stmt: s, .. } = &mut self.m.objs[p as usize].d {
                *s = top;
            }
        }
    }

    /// Named blocks (and their variables) inside a statement.
    fn walk_stmt(&mut self, st: &'a Statement, scope: u32, ctx: &Ctx) {
        match &st.kind {
            StatementKind::SeqBlock { name, stmts }
            | StatementKind::ParBlock { name, stmts, .. } => {
                let inner = match name {
                    Some(n) => {
                        let (kind, ty, join) = match &st.kind {
                            StatementKind::ParBlock { join_type, .. } => (
                                SKind::NamedFork,
                                c::NAMED_FORK,
                                match join_type {
                                    JoinType::Join => c::JOIN,
                                    JoinType::JoinAny => c::JOIN_ANY,
                                    JoinType::JoinNone => c::JOIN_NONE,
                                },
                            ),
                            _ => (SKind::NamedBegin, c::NAMED_BEGIN, 0),
                        };
                        let psc = &self.m.scopes[scope as usize];
                        let full = format!("{}.{}", psc.full, n.name);
                        let flat_prefix = format!("{}{}.", psc.flat_prefix, n.name);
                        let flat_suffix = psc.flat_suffix.clone();
                        let automatic = psc.automatic;
                        let (file, line) = self.loc(st.span, ctx.file);
                        self.m.add_scope(MScope {
                            kind,
                            type_code: ty,
                            name: n.name.clone(),
                            full,
                            def_name: String::new(),
                            parent: scope,
                            inst_idx: -2,
                            file,
                            line,
                            def_file: NONE,
                            def_line: 0,
                            children: Vec::new(),
                            members: Vec::new(),
                            gen_array: NONE,
                            index: 0,
                            implicit: false,
                            automatic,
                            cell: false,
                            top: false,
                            join,
                            func_type: 0,
                            ret_td: NONE,
                            dpi: None,
                            flat_prefix,
                            flat_suffix,
                        })
                    }
                    None => scope,
                };
                for s in stmts {
                    if name.is_some() {
                        if let StatementKind::VarDecl {
                            data_type,
                            lifetime,
                            declarators,
                        } = &s.kind
                        {
                            let sub = Ctx {
                                scope: inner,
                                flat_prefix: self.m.scopes[inner as usize].flat_prefix.clone(),
                                flat_suffix: self.m.scopes[inner as usize].flat_suffix.clone(),
                                automatic: lifetime
                                    .map(|l| l == Lifetime::Automatic)
                                    .unwrap_or(self.m.scopes[inner as usize].automatic),
                                ..ctx.clone()
                            };
                            for d in declarators {
                                let t = self.tdesc(data_type, &d.dimensions, &sub);
                                self.declare(
                                    &d.name,
                                    t,
                                    0,
                                    c::NO_DIRECTION,
                                    false,
                                    sub.automatic,
                                    d.span,
                                    &sub,
                                );
                            }
                            continue;
                        }
                    }
                    self.walk_stmt(s, inner, ctx);
                }
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.walk_stmt(then_stmt, scope, ctx);
                if let Some(e) = else_stmt {
                    self.walk_stmt(e, scope, ctx);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items {
                    self.walk_stmt(&it.stmt, scope, ctx);
                }
            }
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body } => self.walk_stmt(body, scope, ctx),
            StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => {
                self.walk_stmt(stmt, scope, ctx)
            }
            StatementKind::RandCase { items } => {
                for (_, s) in items {
                    self.walk_stmt(s, scope, ctx);
                }
            }
            _ => {}
        }
    }

    fn cont_assign(&mut self, ca: &'a ContinuousAssign, ctx: &Ctx) {
        for (l, r) in &ca.assignments {
            let lhs = self.expr_obj(l, ctx.scope, ctx.file);
            let rhs = self.expr_obj(r, ctx.scope, ctx.file);
            let delay = match &ca.delay {
                Some(e) => self.expr_obj(e, ctx.scope, ctx.file),
                None => NONE,
            };
            let (file, line) = self.loc(ca.span, ctx.file);
            self.m.add_obj(
                MObj {
                    type_code: c::CONT_ASSIGN,
                    name: String::new(),
                    full: String::new(),
                    scope: ctx.scope,
                    parent: NONE,
                    file,
                    line,
                    d: OData::ContAssign {
                        lhs,
                        rhs,
                        delay,
                        net_decl: false,
                    },
                },
                false,
                true,
            );
        }
    }

    fn gates(&mut self, gi: &'a GateInstantiation, ctx: &Ctx) {
        let (prim_type, ty, def) = match gi.gate_type {
            GateType::And => (1, c::GATE, "and"),
            GateType::Nand => (2, c::GATE, "nand"),
            GateType::Nor => (3, c::GATE, "nor"),
            GateType::Or => (4, c::GATE, "or"),
            GateType::Xor => (5, c::GATE, "xor"),
            GateType::Xnor => (6, c::GATE, "xnor"),
            GateType::Buf => (7, c::GATE, "buf"),
            GateType::Not => (8, c::GATE, "not"),
            GateType::Bufif0 => (9, c::GATE, "bufif0"),
            GateType::Bufif1 => (10, c::GATE, "bufif1"),
            GateType::Notif0 => (11, c::GATE, "notif0"),
            GateType::Notif1 => (12, c::GATE, "notif1"),
            GateType::Nmos => (13, c::SWITCH, "nmos"),
            GateType::Pmos => (14, c::SWITCH, "pmos"),
            GateType::Cmos => (15, c::SWITCH, "cmos"),
            GateType::Rnmos => (16, c::SWITCH, "rnmos"),
            GateType::Rpmos => (17, c::SWITCH, "rpmos"),
            GateType::Rcmos => (18, c::SWITCH, "rcmos"),
            GateType::Rtran => (19, c::SWITCH, "rtran"),
            GateType::Rtranif0 => (20, c::SWITCH, "rtranif0"),
            GateType::Rtranif1 => (21, c::SWITCH, "rtranif1"),
            GateType::Tran => (22, c::SWITCH, "tran"),
            GateType::Tranif0 => (23, c::SWITCH, "tranif0"),
            GateType::Tranif1 => (24, c::SWITCH, "tranif1"),
            GateType::Pullup => (25, c::GATE, "pullup"),
            GateType::Pulldown => (26, c::GATE, "pulldown"),
        };
        for g in &gi.instances {
            let n = g.terminals.len();
            // Terminal directions (§28): outputs first; buf/not may have
            // several outputs and one input; tran terminals are inouts.
            let dirs: Vec<c_int> = (0..n)
                .map(|i| match gi.gate_type {
                    GateType::Buf | GateType::Not => {
                        if i + 1 < n {
                            c::OUTPUT
                        } else {
                            c::INPUT
                        }
                    }
                    GateType::Tran | GateType::Rtran => c::INOUT,
                    GateType::Tranif0
                    | GateType::Tranif1
                    | GateType::Rtranif0
                    | GateType::Rtranif1 => {
                        if i < 2 {
                            c::INOUT
                        } else {
                            c::INPUT
                        }
                    }
                    GateType::Pullup | GateType::Pulldown => c::OUTPUT,
                    _ => {
                        if i == 0 {
                            c::OUTPUT
                        } else {
                            c::INPUT
                        }
                    }
                })
                .collect();
            let inputs = dirs.iter().filter(|&&d| d == c::INPUT).count() as u32;
            let terms: Vec<(&'a Expression, c_int)> =
                g.terminals.iter().zip(dirs.iter().copied()).collect();
            self.primitive(
                g.name.as_ref(),
                ty,
                prim_type,
                def,
                &terms,
                gi.delay.as_ref(),
                inputs,
                g.span,
                ctx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn primitive(
        &mut self,
        name: Option<&Identifier>,
        ty: c_int,
        prim_type: c_int,
        def: &str,
        terms: &[(&'a Expression, c_int)],
        delay: Option<&'a Expression>,
        inputs: u32,
        span: Span,
        ctx: &Ctx,
    ) {
        let sc_full = self.m.scopes[ctx.scope as usize].full.clone();
        let (nm, full) = match name {
            Some(n) => (n.name.clone(), format!("{}.{}", sc_full, n.name)),
            None => (String::new(), String::new()),
        };
        let (file, line) = self.loc(span, ctx.file);
        let delay = match delay {
            Some(e) => self.expr_obj(e, ctx.scope, ctx.file),
            None => NONE,
        };
        let p = self.m.add_obj(
            MObj {
                type_code: ty,
                name: nm,
                full,
                scope: ctx.scope,
                parent: NONE,
                file,
                line,
                d: OData::Prim {
                    prim_type,
                    def_name: def.to_string(),
                    terms: Vec::new(),
                    delay,
                    inputs,
                },
            },
            true,
            true,
        );
        let mut tv = Vec::new();
        for (i, (e, dir)) in terms.iter().enumerate() {
            let expr = self.expr_obj(e, ctx.scope, ctx.file);
            let t = self.m.add_obj(
                MObj {
                    type_code: c::PRIM_TERM,
                    name: String::new(),
                    full: String::new(),
                    scope: ctx.scope,
                    parent: p,
                    file,
                    line,
                    d: OData::PrimTerm {
                        index: i as u32,
                        dir: *dir,
                        expr,
                    },
                },
                false,
                false,
            );
            tv.push(t);
        }
        if let OData::Prim { terms, .. } = &mut self.m.objs[p as usize].d {
            *terms = tv;
        }
    }

    fn instantiation(&mut self, mi: &'a ModuleInstantiation, ctx: &Ctx) {
        let def = self.defs.get(mi.module_name.name.as_str()).copied();
        if let Some((DefRef::Udp(u), _)) = def {
            for hi in &mi.instances {
                let exprs: Vec<&'a Expression> = hi
                    .connections
                    .iter()
                    .filter_map(|c| match c {
                        PortConnection::Ordered(Some(e)) => Some(e),
                        PortConnection::Named { expr: Some(e), .. } => Some(e),
                        _ => None,
                    })
                    .collect();
                let terms: Vec<(&'a Expression, c_int)> = exprs
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (*e, if i == 0 { c::OUTPUT } else { c::INPUT }))
                    .collect();
                let inputs = u.ports.len().saturating_sub(1) as u32;
                self.primitive(
                    Some(&hi.name),
                    c::UDP,
                    if u.is_sequential {
                        c::SEQ_PRIM
                    } else {
                        c::COMB_PRIM
                    },
                    &u.name.name,
                    &terms,
                    None,
                    inputs,
                    hi.span,
                    ctx,
                );
            }
            return;
        }
        let inst_prefix = self.inst_flat_prefix(ctx);
        for hi in &mi.instances {
            self.m
                .placeholders
                .insert(format!("{}{}", inst_prefix, hi.name.name));
            self.m
                .placeholders
                .insert(format!("{}{}", ctx.flat_prefix, hi.name.name));
            let path = format!("{}{}", ctx.inst_prefix, hi.name.name);
            let mut found: Vec<(usize, String)> = Vec::new();
            if let Some(&InstIdx(i)) = self.inst_by_path.get(path.as_str()) {
                found.push((i, path.clone()));
            } else if !hi.dimensions.is_empty() {
                // An instance array: its elements `u[3]`, `u[2]`, ...
                let pre = format!("{}[", path);
                let mut v: Vec<(usize, String)> = self
                    .inst_by_path
                    .iter()
                    .filter(|(p, _)| p.starts_with(&pre) && !p[pre.len()..].contains('.'))
                    .map(|(p, &InstIdx(i))| (i, p.to_string()))
                    .collect();
                v.sort_by_key(|(i, _)| *i);
                found = v;
            }
            for (i, p) in found {
                if self.attached[i] {
                    continue;
                }
                let full = self.sim.hier_path(&p);
                self.instance_scope(
                    i as isize,
                    &full,
                    &p,
                    ctx.scope,
                    Some((hi.span, ctx.file)),
                    Some((&hi.connections, ctx.scope)),
                );
            }
        }
    }

    fn make_ports(
        &mut self,
        ports: &'a PortList,
        decls: &[PortDecl],
        ctx: &Ctx,
        conns: Option<(&'a [PortConnection], u32)>,
    ) {
        let names: Vec<&str> = match ports {
            PortList::Ansi(l) => l.iter().map(|p| p.name.name.as_str()).collect(),
            PortList::NonAnsi(l) => l.iter().map(|p| p.name.as_str()).collect(),
            PortList::Empty => Vec::new(),
        };
        let scope_full = self.m.scopes[ctx.scope as usize].full.clone();
        let by_name_conn = conns.is_some_and(|(cs, _)| {
            cs.iter()
                .any(|c| matches!(c, PortConnection::Named { .. } | PortConnection::Wildcard))
        });
        let wildcard =
            conns.is_some_and(|(cs, _)| cs.iter().any(|c| matches!(c, PortConnection::Wildcard)));
        for (i, pname) in names.iter().enumerate() {
            let decl = decls.iter().find(|d| d.name == *pname);
            let full = format!("{}.{}", scope_full, pname);
            let low = match self.m.by_name.get(&full) {
                Some(MRef::Obj(o)) => *o,
                _ => NONE,
            };
            let dir = decl.map(|d| d.dir).unwrap_or_else(|| match low {
                NONE => c::NO_DIRECTION,
                o => match &self.m.objs[o as usize].d {
                    OData::Var(v) => v.dir,
                    _ => c::NO_DIRECTION,
                },
            });
            let port_type = decl.map(|d| d.port_type).unwrap_or(c::PORT);
            // The high connection, in the parent scope.
            let high = match conns {
                Some((cs, pscope)) => {
                    let mut e: Option<Expression> = None;
                    let mut named = false;
                    for c in cs {
                        if let PortConnection::Named {
                            name,
                            expr,
                            implicit,
                        } = c
                        {
                            if name.name == *pname {
                                named = true;
                                e = match expr {
                                    Some(x) => Some(x.clone()),
                                    None if *implicit => Some(ident_expr(pname)),
                                    None => None,
                                };
                            }
                        }
                    }
                    if !named {
                        if by_name_conn {
                            if wildcard {
                                e = Some(ident_expr(pname));
                            }
                        } else if let Some(PortConnection::Ordered(Some(x))) = cs.get(i) {
                            e = Some(x.clone());
                        }
                    }
                    match e {
                        Some(x) => {
                            let pfile = ctx.file;
                            self.expr_obj_owned(x, pscope, pfile)
                        }
                        None => NONE,
                    }
                }
                None => NONE,
            };
            let (file, line) = match low {
                NONE => (NONE, 0),
                o => (self.m.objs[o as usize].file, self.m.objs[o as usize].line),
            };
            let po = self.m.add_obj(
                MObj {
                    type_code: c::PORT,
                    name: pname.to_string(),
                    full: full.clone(),
                    scope: ctx.scope,
                    parent: NONE,
                    file,
                    line,
                    d: OData::Port(PortData {
                        index: i as u32,
                        dir,
                        low,
                        high,
                        port_type,
                        conn_by_name: by_name_conn,
                    }),
                },
                false,
                true,
            );
            self.m.port_by_name.insert(full, PortIdx(po));
        }
    }

    fn expr_obj_owned(&mut self, e: Expression, scope: u32, file: u32) -> u32 {
        let o = self.expr_obj(&e, scope, file);
        // The port-connection location is the parent's source file, whose
        // span we do not know here; keep only the scope.
        let _ = file;
        o
    }

    // --- generate constructs -------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn gen_scope(
        &mut self,
        name: String,
        implicit: bool,
        span: Span,
        ctx: &Ctx,
        flat_prefix: String,
        flat_suffix: String,
        inst_prefix: String,
        params: std::rc::Rc<HashMap<String, Value>>,
        array: u32,
        index: i64,
    ) -> Ctx {
        let psc = &self.m.scopes[ctx.scope as usize];
        let full = format!("{}.{}", psc.full, name);
        let (file, line) = self.loc(span, ctx.file);
        let s = self.m.add_scope(MScope {
            kind: SKind::GenScope,
            type_code: c::GEN_SCOPE,
            name,
            full,
            def_name: String::new(),
            parent: ctx.scope,
            inst_idx: -2,
            file,
            line,
            def_file: NONE,
            def_line: 0,
            children: Vec::new(),
            members: Vec::new(),
            gen_array: array,
            index,
            implicit,
            automatic: false,
            cell: false,
            top: false,
            join: 0,
            func_type: 0,
            ret_td: NONE,
            dpi: None,
            flat_prefix: flat_prefix.clone(),
            flat_suffix: flat_suffix.clone(),
        });
        Ctx {
            scope: s,
            file: ctx.file,
            flat_prefix,
            flat_suffix,
            inst_prefix,
            params,
            automatic: ctx.automatic,
            body_params_local: false,
        }
    }

    /// A conditional generate block (§27.5) that the elaborator took.
    fn gen_block(
        &mut self,
        label: Option<&String>,
        ord: u32,
        items: &'a [ModuleItem],
        span: Span,
        ctx: &Ctx,
    ) {
        let name = match label {
            Some(l) => l.clone(),
            None => format!("genblk{}", ord),
        };
        // A labeled block scopes its declarations; an unlabeled one leaves
        // them in the enclosing flat namespace.
        let flat_prefix = match label {
            Some(l) => format!("{}{}.", ctx.flat_prefix, l),
            None => ctx.flat_prefix.clone(),
        };
        let inst_prefix = format!("{}{}.", ctx.inst_prefix, name);
        let params = self.merge_params(ctx, &flat_prefix, None);
        let sub = self.gen_scope(
            name,
            label.is_none(),
            span,
            ctx,
            flat_prefix,
            ctx.flat_suffix.clone(),
            inst_prefix,
            params,
            NONE,
            0,
        );
        let mut ord2 = 0u32;
        let mut ports = Vec::new();
        self.walk_items(items, &sub, &mut ord2, &mut ports);
    }

    fn merge_params(
        &self,
        ctx: &Ctx,
        flat_prefix: &str,
        genvar: Option<(&str, i64)>,
    ) -> std::rc::Rc<HashMap<String, Value>> {
        let own = self.params_by_prefix.get(flat_prefix);
        if own.is_none() && genvar.is_none() {
            return ctx.params.clone();
        }
        let mut p = (*ctx.params).clone();
        if let Some(ParamList(own)) = own {
            for (k, v) in own {
                p.insert(k.clone(), v.clone());
            }
        }
        if let Some((g, v)) = genvar {
            p.insert(g.to_string(), Value::from_u64(v as u64, 32));
        }
        std::rc::Rc::new(p)
    }

    fn generate_if(&mut self, gi: &'a GenerateIf, ctx: &Ctx, ord: u32) {
        for (bi, (cond, items)) in gi.branches.iter().enumerate() {
            let take = match cond {
                Some(e) => match self.eval(e, ctx) {
                    Some(v) => v != 0,
                    None => {
                        let label = gi.branch_labels.get(bi).and_then(|l| l.as_ref());
                        self.branch_elaborated(label, ord, items, ctx)
                    }
                },
                None => true,
            };
            if take {
                let label = gi.branch_labels.get(bi).and_then(|l| l.as_ref());
                self.gen_block(label, ord, items, gi.span, ctx);
                return;
            }
        }
    }

    fn generate_case(&mut self, gc: &'a GenerateCase, ctx: &Ctx, ord: u32) {
        let sel = self.eval(&gc.selector, ctx);
        let mut chosen = None;
        if let Some(sel) = sel {
            for arm in &gc.arms {
                if !arm.values.is_empty()
                    && arm.values.iter().any(|v| self.eval(v, ctx) == Some(sel))
                {
                    chosen = Some(arm);
                    break;
                }
            }
        } else {
            for arm in &gc.arms {
                if self.branch_elaborated(arm.label.as_ref(), ord, &arm.items, ctx) {
                    chosen = Some(arm);
                    break;
                }
            }
        }
        if chosen.is_none() {
            chosen = gc.arms.iter().find(|a| a.values.is_empty());
        }
        if let Some(arm) = chosen {
            self.gen_block(arm.label.as_ref(), ord, &arm.items, gc.span, ctx);
        }
    }

    /// When a generate condition cannot be evaluated here: did the
    /// elaborator take this branch? Judged by what it left behind.
    fn branch_elaborated(
        &self,
        label: Option<&String>,
        ord: u32,
        items: &[ModuleItem],
        ctx: &Ctx,
    ) -> bool {
        let name = match label {
            Some(l) => l.clone(),
            None => format!("genblk{}", ord),
        };
        let ipre = format!("{}{}.", ctx.inst_prefix, name);
        if self.inst_by_path.keys().any(|p| p.starts_with(&ipre)) {
            return true;
        }
        let fpre = match label {
            Some(l) => format!("{}{}.", ctx.flat_prefix, l),
            None => ctx.flat_prefix.clone(),
        };
        items.iter().any(|it| {
            let names: Vec<&str> = match it {
                ModuleItem::DataDeclaration(dd) => dd
                    .declarators
                    .iter()
                    .map(|d| d.name.name.as_str())
                    .collect(),
                ModuleItem::NetDeclaration(nd) => nd
                    .declarators
                    .iter()
                    .map(|d| d.name.name.as_str())
                    .collect(),
                _ => Vec::new(),
            };
            names.iter().any(|n| {
                self.sig_of(&format!("{}{}{}", fpre, n, ctx.flat_suffix))
                    .is_some()
            })
        })
    }

    fn generate_for(&mut self, gf: &'a GenerateFor, ctx: &Ctx, ord: u32) {
        let name = match &gf.name {
            Some(l) => l.clone(),
            None => format!("genblk{}", ord),
        };
        // The iteration values, as the elaborator computed them.
        let mut values: Vec<i64> = Vec::new();
        let mut i = gf.init_val;
        let mut p = (*ctx.params).clone();
        let mut ok = true;
        for _ in 0..10000 {
            p.insert(gf.var.clone(), Value::from_u64(i as u64, 32));
            match xezim_core::elaborate::const_eval_i64_with_params(&gf.cond, Some(&p)) {
                Some(0) => break,
                Some(_) => values.push(i),
                None => {
                    ok = false;
                    break;
                }
            }
            i = match &gf.incr.kind {
                ExprKind::Unary {
                    op: UnaryOp::PostIncr | UnaryOp::PreIncr,
                    ..
                } => i + 1,
                ExprKind::Unary {
                    op: UnaryOp::PostDecr | UnaryOp::PreDecr,
                    ..
                } => i - 1,
                ExprKind::AssignExpr { rvalue, .. } => {
                    match xezim_core::elaborate::const_eval_i64_with_params(rvalue, Some(&p)) {
                        Some(v) if v != i => v,
                        _ => i + 1,
                    }
                }
                _ => match xezim_core::elaborate::const_eval_i64_with_params(&gf.incr, Some(&p)) {
                    Some(v) if v != i => v,
                    _ => i + 1,
                },
            };
        }
        if !ok {
            // Recover the indices from the instance paths and names left.
            values.clear();
            let pre = format!("{}{}[", ctx.inst_prefix, name);
            let fpre = format!("{}{}[", ctx.flat_prefix, name);
            let mut set: std::collections::BTreeSet<i64> = Default::default();
            for p in self.inst_by_path.keys() {
                if let Some(r) = p.strip_prefix(&pre) {
                    if let Some(n) = r.split(']').next().and_then(|n| n.parse().ok()) {
                        set.insert(n);
                    }
                }
            }
            for k in self.sim.signal_name_to_id.keys() {
                if let Some(r) = k.strip_prefix(fpre.as_str()) {
                    if let Some(n) = r.split(']').next().and_then(|n| n.parse().ok()) {
                        set.insert(n);
                    }
                }
            }
            values.extend(set);
        }
        if values.is_empty() {
            // A loop that generated nothing has no array.
            return;
        }
        let psc_full = self.m.scopes[ctx.scope as usize].full.clone();
        let (file, line) = self.loc(gf.span, ctx.file);
        let arr = self.m.add_obj(
            MObj {
                type_code: c::GEN_SCOPE_ARRAY,
                name: name.clone(),
                full: format!("{}.{}", psc_full, name),
                scope: ctx.scope,
                parent: NONE,
                file,
                line,
                d: OData::GenArray { elems: Vec::new() },
            },
            true,
            true,
        );
        let mut elems = Vec::new();
        for v in values {
            let ename = format!("{}[{}]", name, v);
            let (flat_prefix, flat_suffix) = match &gf.name {
                Some(_) => (
                    format!("{}{}.", ctx.flat_prefix, ename),
                    ctx.flat_suffix.clone(),
                ),
                None => (
                    ctx.flat_prefix.clone(),
                    format!("__gf_{}_{}_{}", gf.var, v, ctx.flat_suffix),
                ),
            };
            let inst_prefix = format!("{}{}.", ctx.inst_prefix, ename);
            let params = self.merge_params(ctx, &flat_prefix, Some((&gf.var, v)));
            let sub = self.gen_scope(
                ename,
                gf.name.is_none(),
                gf.span,
                ctx,
                flat_prefix,
                flat_suffix,
                inst_prefix,
                params,
                arr,
                v,
            );
            elems.push(sub.scope);
            self.m.genvars.insert(sub.scope, (gf.var.clone(), v));
            let mut ord2 = 0u32;
            let mut ports = Vec::new();
            self.walk_items(&gf.items, &sub, &mut ord2, &mut ports);
        }
        if let OData::GenArray { elems: e } = &mut self.m.objs[arr as usize].d {
            *e = elems;
        }
    }

    // --- tasks and functions -------------------------------------------

    fn subroutine_scope(
        &mut self,
        name: &str,
        kind: SKind,
        lifetime: Option<Lifetime>,
        span: Span,
        ctx: &Ctx,
    ) -> Option<u32> {
        let psc = &self.m.scopes[ctx.scope as usize];
        let full = if psc.kind == SKind::Package {
            format!("{}{}", psc.full, name)
        } else {
            format!("{}.{}", psc.full, name)
        };
        if self.m.by_name.contains_key(&full) {
            return None;
        }
        let automatic = match lifetime {
            Some(Lifetime::Automatic) => true,
            Some(Lifetime::Static) => false,
            None => ctx.automatic,
        };
        let flat_prefix = format!("{}{}.", psc.flat_prefix, name);
        let (file, line) = self.loc(span, ctx.file);
        Some(self.m.add_scope(MScope {
            kind,
            type_code: if kind == SKind::Task {
                c::TASK
            } else {
                c::FUNCTION
            },
            name: name.to_string(),
            full,
            def_name: String::new(),
            parent: ctx.scope,
            inst_idx: -2,
            file,
            line,
            def_file: NONE,
            def_line: 0,
            children: Vec::new(),
            members: Vec::new(),
            gen_array: NONE,
            index: 0,
            implicit: false,
            automatic,
            cell: false,
            top: false,
            join: 0,
            func_type: 0,
            ret_td: NONE,
            dpi: None,
            flat_prefix,
            flat_suffix: String::new(),
        }))
    }

    fn io_decls(&mut self, s: u32, ports: &'a [crate::ast::decl::FunctionPort], ctx: &Ctx) {
        let sub = Ctx {
            scope: s,
            ..ctx.clone()
        };
        let full = self.m.scopes[s as usize].full.clone();
        let mut prev_dir = c::INPUT;
        for p in ports {
            let dir = Self::dir_code(p.direction);
            let dir = if dir == 0 { prev_dir } else { dir };
            prev_dir = dir;
            let t = self.tdesc(&p.data_type, &p.dimensions, &sub);
            let td = self.m.add_td(t);
            let (file, line) = self.loc(p.span, ctx.file);
            self.m.add_obj(
                MObj {
                    type_code: c::IO_DECL,
                    name: p.name.name.clone(),
                    full: format!("{}.{}", full, p.name.name),
                    scope: s,
                    parent: NONE,
                    file,
                    line,
                    d: OData::IoDecl { dir, td },
                },
                true,
                true,
            );
        }
    }

    fn body_decls(&mut self, s: u32, items: &'a [Statement], ctx: &Ctx) {
        let automatic = self.m.scopes[s as usize].automatic;
        let sub = Ctx {
            scope: s,
            flat_prefix: self.m.scopes[s as usize].flat_prefix.clone(),
            flat_suffix: String::new(),
            automatic,
            ..ctx.clone()
        };
        for st in items {
            match &st.kind {
                StatementKind::VarDecl {
                    data_type,
                    lifetime,
                    declarators,
                } => {
                    let auto = lifetime
                        .map(|l| l == Lifetime::Automatic)
                        .unwrap_or(automatic);
                    for d in declarators {
                        let t = self.tdesc(data_type, &d.dimensions, &sub);
                        self.declare(&d.name, t, 0, c::NO_DIRECTION, false, auto, d.span, &sub);
                    }
                }
                _ => self.walk_stmt(st, s, &sub),
            }
        }
    }

    fn function(&mut self, fd: &'a FunctionDeclaration, ctx: &Ctx, dpi: Option<(bool, bool)>) {
        if fd.name.has_scope() {
            return; // an out-of-class method body
        }
        let Some(s) = self.subroutine_scope(
            &fd.name.name.name,
            SKind::Function,
            fd.lifetime,
            fd.span,
            ctx,
        ) else {
            return;
        };
        let rt = self.tdesc(&fd.return_type, &[], ctx);
        let func_type = match &rt.kind {
            TKind::Int | TKind::Integer | TKind::ShortInt | TKind::LongInt | TKind::Byte => {
                c::INT_FUNC
            }
            TKind::Real | TKind::ShortReal => c::REAL_FUNC,
            TKind::Time => c::TIME_FUNC,
            TKind::Logic | TKind::Reg | TKind::Bit | TKind::Implicit => {
                if rt.signed {
                    c::SIZED_SIGNED_FUNC
                } else {
                    c::SIZED_FUNC
                }
            }
            _ => c::OTHER_FUNC,
        };
        let td = self.m.add_td(rt);
        {
            let sc = &mut self.m.scopes[s as usize];
            sc.func_type = func_type;
            sc.ret_td = td;
            sc.dpi = dpi;
        }
        self.io_decls(s, &fd.ports, ctx);
        if dpi.is_none() {
            self.body_decls(s, &fd.items, ctx);
        }
    }

    fn task(&mut self, td: &'a TaskDeclaration, ctx: &Ctx, dpi: Option<(bool, bool)>) {
        if td.name.has_scope() {
            return;
        }
        let Some(s) =
            self.subroutine_scope(&td.name.name.name, SKind::Task, td.lifetime, td.span, ctx)
        else {
            return;
        };
        self.m.scopes[s as usize].dpi = dpi;
        self.io_decls(s, &td.ports, ctx);
        if dpi.is_none() {
            self.body_decls(s, &td.items, ctx);
        }
    }

    fn dpi_import(&mut self, di: &'a crate::ast::decl::DPIImport, ctx: &Ctx) {
        let flags = (
            di.property == Some(crate::ast::decl::DPIProperty::Context),
            di.property == Some(crate::ast::decl::DPIProperty::Pure),
        );
        match &di.proto {
            crate::ast::decl::DPIProto::Function(f) => self.function(f, ctx, Some(flags)),
            crate::ast::decl::DPIProto::Task(t) => self.task(t, ctx, Some(flags)),
        }
    }

    // --- modports ------------------------------------------------------

    fn modports(&mut self, md: &'a crate::ast::decl::ModportDeclaration, ctx: &Ctx) {
        let sc_full = self.m.scopes[ctx.scope as usize].full.clone();
        for item in &md.items {
            let (file, line) = self.loc(item.span, ctx.file);
            let full = format!("{}.{}", sc_full, item.name.name);
            let mp = self.m.add_obj(
                MObj {
                    type_code: c::MODPORT,
                    name: item.name.name.clone(),
                    full: full.clone(),
                    scope: ctx.scope,
                    parent: NONE,
                    file,
                    line,
                    d: OData::Modport { ios: Vec::new() },
                },
                true,
                true,
            );
            let mut ios = Vec::new();
            for port in &item.ports {
                let e = match &port.expr {
                    Some(e) => e.clone(),
                    None => ident_expr(&port.name.name),
                };
                let expr = self.expr_obj(&e, ctx.scope, ctx.file);
                let (f, l) = self.loc(port.span, ctx.file);
                ios.push(self.m.add_obj(
                    MObj {
                        type_code: c::IO_DECL,
                        name: port.name.name.clone(),
                        full: format!("{}.{}", full, port.name.name),
                        scope: ctx.scope,
                        parent: mp,
                        file: f,
                        line: l,
                        d: OData::ModportIo {
                            dir: Self::dir_code(port.direction),
                            expr,
                        },
                    },
                    true,
                    false,
                ));
            }
            if let OData::Modport { ios: v } = &mut self.m.objs[mp as usize].d {
                *v = ios;
            }
        }
    }

    // --- specify -------------------------------------------------------

    fn specify(&mut self, sb: &'a SpecifyBlock, ctx: &Ctx) {
        for p in &sb.paths {
            let (file, line) = self.loc(p.span, ctx.file);
            let cond = match &p.cond {
                Some(e) => self.expr_obj(e, ctx.scope, ctx.file),
                None => NONE,
            };
            let mp = self.m.add_obj(
                MObj {
                    type_code: c::MOD_PATH,
                    name: String::new(),
                    full: String::new(),
                    scope: ctx.scope,
                    parent: NONE,
                    file,
                    line,
                    d: OData::ModPath {
                        ins: Vec::new(),
                        outs: Vec::new(),
                        cond,
                        ifnone: p.ifnone,
                    },
                },
                false,
                true,
            );
            let mut ins = Vec::new();
            let mut outs = Vec::new();
            for (list, dir, out) in [
                (&p.srcs, c::INPUT, &mut ins),
                (&p.dsts, c::OUTPUT, &mut outs),
            ] {
                for id in list.iter() {
                    let e = ident_expr(&id.name);
                    let expr = self.expr_obj(&e, ctx.scope, ctx.file);
                    let (f, l) = self.loc(id.span, ctx.file);
                    out.push(self.m.add_obj(
                        MObj {
                            type_code: c::PATH_TERM,
                            name: String::new(),
                            full: String::new(),
                            scope: ctx.scope,
                            parent: mp,
                            file: f,
                            line: l,
                            d: OData::PathTerm { dir, expr },
                        },
                        false,
                        false,
                    ));
                }
            }
            if let OData::ModPath {
                ins: i, outs: o, ..
            } = &mut self.m.objs[mp as usize].d
            {
                *i = ins;
                *o = outs;
            }
        }
        for tc in &sb.timing_checks {
            let (tchk_type, ref_i, data_i, notif_i) = match tc.name.as_str() {
                "$setup" => (c::SETUP, Some(1), Some(0), Some(3)),
                "$hold" => (c::HOLD, Some(0), Some(1), Some(3)),
                "$setuphold" => (c::SETUP_HOLD, Some(0), Some(1), Some(4)),
                "$recovery" => (c::RECOVERY, Some(0), Some(1), Some(3)),
                "$removal" => (c::REMOVAL, Some(0), Some(1), Some(3)),
                "$recrem" => (c::RECREM, Some(0), Some(1), Some(4)),
                "$skew" => (c::SKEW, Some(0), Some(1), Some(3)),
                "$timeskew" => (c::TIMESKEW, Some(0), Some(1), Some(3)),
                "$fullskew" => (c::FULLSKEW, Some(0), Some(1), Some(4)),
                "$period" => (c::PERIOD, Some(0), None, Some(2)),
                "$width" => (c::WIDTH, Some(0), None, Some(3)),
                "$nochange" => (c::NO_CHANGE, Some(0), Some(1), Some(4)),
                _ => continue,
            };
            let (file, line) = self.loc(tc.span, ctx.file);
            let t = self.m.add_obj(
                MObj {
                    type_code: c::TCHK,
                    name: tc.name.clone(),
                    full: String::new(),
                    scope: ctx.scope,
                    parent: NONE,
                    file,
                    line,
                    d: OData::Tchk {
                        tchk_type,
                        ref_term: NONE,
                        data_term: NONE,
                        notifier: NONE,
                    },
                },
                false,
                true,
            );
            let mut term = |b: &mut Self, i: Option<usize>, is_notifier: bool| -> u32 {
                let Some(Some(a)) = i.and_then(|i| tc.args.get(i)) else {
                    return NONE;
                };
                if is_notifier {
                    return b.expr_obj(&a.expr, ctx.scope, ctx.file);
                }
                let expr = b.expr_obj(&a.expr, ctx.scope, ctx.file);
                let cond = match &a.cond {
                    Some(e) => b.expr_obj(e, ctx.scope, ctx.file),
                    None => NONE,
                };
                let edge = match a.edges {
                    None => 0,
                    Some(mask) => edge_code(mask),
                };
                b.m.add_obj(
                    MObj {
                        type_code: c::TCHK_TERM,
                        name: String::new(),
                        full: String::new(),
                        scope: ctx.scope,
                        parent: t,
                        file,
                        line,
                        d: OData::TchkTerm { edge, expr, cond },
                    },
                    false,
                    false,
                )
            };
            let r = term(self, ref_i, false);
            let d = term(self, data_i, false);
            let n = term(self, notif_i, true);
            if let OData::Tchk {
                ref_term,
                data_term,
                notifier,
                ..
            } = &mut self.m.objs[t as usize].d
            {
                *ref_term = r;
                *data_term = d;
                *notifier = n;
            }
        }
    }
}

/// §31.5 edge-control mask (xezim's 3x3 transition bits) to `vpiEdge` flags.
fn edge_code(mask: u16) -> c_int {
    use crate::ast::decl::timing_edge_bit as b;
    let mut e = 0;
    for (from, to, flag) in [
        (0u8, 1u8, c::EDGE01),
        (1, 0, c::EDGE10),
        (0, 2, c::EDGE0X),
        (2, 1, c::EDGEX1),
        (1, 2, c::EDGE1X),
        (2, 0, c::EDGEX0),
    ] {
        if mask & b(from, to) != 0 {
            e |= flag;
        }
    }
    e
}

/// A bare identifier expression.
fn ident_expr(name: &str) -> Expression {
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: None,
            path: vec![HierPathSegment {
                name: Identifier {
                    name: name.to_string(),
                    span: Span::dummy(),
                },
                selects: Vec::new(),
            }],
            span: Span::dummy(),
            cached_signal_id: Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        }),
        Span::dummy(),
    )
}

// ---------------------------------------------------------------------------
// Handles
// ---------------------------------------------------------------------------

/// A constant made on the fly (a range bound, a generate index): an `Obj`
/// with no model object, its value in `value` and `vpiConstType` in `lsb`.
fn const_handle(v: Value, const_type: c_int) -> VpiHandle {
    let mut h = VpiHandle::signal(usize::MAX, c::CONSTANT, "", "");
    h.kind = VpiKind::Obj;
    h.width = v.width;
    h.lsb = const_type as u32;
    h.value = Some(v);
    h
}

/// A range made on the fly (the current bounds of a dynamic array or queue):
/// an `Obj` with no model object, `lsb`/`width` holding left/right.
fn dyn_range_handle(left: i64, right: i64) -> VpiHandle {
    let mut h = VpiHandle::signal(usize::MAX, c::RANGE, "", "");
    h.kind = VpiKind::Obj;
    h.lsb = left as i32 as u32;
    h.width = right as i32 as u32;
    h
}

/// Current `[0:size-1]` bounds of a variable whose outermost dimension is
/// dynamic or a queue.
fn dyn_bounds(m: &VpiModel, sim: &Simulator, o: u32) -> Option<(i64, i64)> {
    let OData::Var(v) = &m.objs[o as usize].d else {
        return None;
    };
    match m.tdescs[v.td as usize].unpacked.first() {
        Some(UDim::Dynamic | UDim::Queue) => {
            let n = sim
                .signals
                .get(&format!("{}.size", v.flat))
                .and_then(|x| x.to_u64())
                .unwrap_or(0) as i64;
            Some((0, n - 1))
        }
        _ => None,
    }
}

fn int_const(v: i64) -> VpiHandle {
    let mut val = Value::from_u64(v as u64, 32);
    val.is_signed = true;
    const_handle(val, c::DEC_CONST)
}

pub(super) fn scope_handle(m: &VpiModel, s: u32) -> VpiHandle {
    let sc = &m.scopes[s as usize];
    let mut h = VpiHandle::signal(0, sc.type_code, &sc.name, &sc.full);
    h.def_name = sc.def_name.clone();
    if sc.inst_idx >= -1 {
        h.kind = VpiKind::Module;
        h.inst_idx = sc.inst_idx;
    } else {
        h.kind = VpiKind::Scope;
        h.signal_id = s as usize;
    }
    h
}

fn plain_obj_handle(m: &VpiModel, o: u32) -> VpiHandle {
    let ob = &m.objs[o as usize];
    let mut h = VpiHandle::signal(o as usize, ob.type_code, &ob.name, &ob.full);
    h.kind = VpiKind::Obj;
    h
}

fn td_is_array(m: &VpiModel, td: u32) -> bool {
    !m.tdescs[td as usize].unpacked.is_empty()
}

/// Number of elements of an array type (all fixed dimensions).
fn fixed_count(t: &TDesc) -> Option<u32> {
    let mut n: u64 = 1;
    for d in &t.unpacked {
        match d {
            UDim::Range(l, r) => n = n.saturating_mul((l - r).unsigned_abs() + 1),
            _ => return None,
        }
    }
    Some(n.min(u32::MAX as u64) as u32)
}

/// The handle of a declared object. Storage-backed objects keep the
/// value-bearing kinds so reads, writes and callbacks work on them.
pub(super) fn obj_handle(m: &mut VpiModel, sim: &mut Simulator, o: u32) -> VpiHandle {
    let (ty, name, full) = {
        let ob = &m.objs[o as usize];
        (ob.type_code, ob.name.clone(), ob.full.clone())
    };
    match &m.objs[o as usize].d {
        OData::Var(v) => {
            if v.member_of != NONE {
                let container = match &m.objs[v.member_of as usize].d {
                    OData::Var(pv) => pv.sig,
                    _ => None,
                };
                if let Some(id) = container {
                    let mut h = VpiHandle::signal(id, ty, &name, &full);
                    h.kind = VpiKind::Slice;
                    h.lsb = v.lsb;
                    h.width = v.width;
                    return h;
                }
                return plain_obj_handle(m, o);
            }
            let Some(id) = v.sig else {
                return plain_obj_handle(m, o);
            };
            if td_is_array(m, v.td) {
                let t = &m.tdescs[v.td as usize];
                let lo = match t.unpacked.first() {
                    Some(UDim::Range(l, r)) => (*l).min(*r),
                    _ => 0,
                };
                let mut h = VpiHandle::signal(id, ty, &name, &full);
                h.kind = VpiKind::Memory;
                h.lsb = lo as u32;
                h.width = fixed_count(t).unwrap_or(0);
                return h;
            }
            let mut h = VpiHandle::signal(id, ty, &name, &full);
            h.direction = v.dir;
            h
        }
        OData::Param { sig: Some(id), .. } => VpiHandle::signal(*id, ty, &name, &full),
        OData::Port(p) => {
            let low_sig = match p.low {
                NONE => None,
                l => match &m.objs[l as usize].d {
                    OData::Var(v) if v.member_of == NONE && !td_is_array(m, v.td) => v.sig,
                    _ => None,
                },
            };
            match low_sig {
                Some(id) => {
                    let width = sim.signal_widths.get(id).copied().unwrap_or(1);
                    VpiHandle::port(id, &name, &full, p.dir, width)
                }
                None => {
                    let mut h = plain_obj_handle(m, o);
                    h.direction = p.dir;
                    h
                }
            }
        }
        OData::Expr(_) => expr_handle(m, sim, o),
        _ => plain_obj_handle(m, o),
    }
}

fn ref_handle(m: &mut VpiModel, sim: &mut Simulator, r: MRef) -> VpiHandle {
    match r {
        MRef::Scope(s) => scope_handle(m, s),
        MRef::Obj(o) => obj_handle(m, sim, o),
    }
}

/// `vpiConstType` of a literal.
fn literal_const_type(e: &Expression) -> Option<c_int> {
    Some(match &e.kind {
        ExprKind::Number(NumberLiteral::Integer { base, size, .. }) => match base {
            NumberBase::Binary => c::BINARY_CONST,
            NumberBase::Octal => c::OCT_CONST,
            NumberBase::Hex => c::HEX_CONST,
            NumberBase::Decimal if size.is_some() => c::DEC_CONST,
            NumberBase::Decimal => c::DEC_CONST,
        },
        ExprKind::Number(NumberLiteral::Real(_)) => c::REAL_CONST,
        ExprKind::Number(NumberLiteral::UnbasedUnsized(_)) => c::BINARY_CONST,
        ExprKind::Number(NumberLiteral::Time(_)) => c::TIME_CONST,
        ExprKind::StringLiteral(_) => c::STRING_CONST,
        _ => return None,
    })
}

/// An expression object's handle: a name is the object it names, a
/// literal is a constant, anything else is the expression itself.
fn expr_handle(m: &mut VpiModel, sim: &mut Simulator, o: u32) -> VpiHandle {
    let OData::Expr(ed) = &m.objs[o as usize].d else {
        return plain_obj_handle(m, o);
    };
    let scope = ed.scope;
    let mut e: &Expression = &ed.expr;
    while let ExprKind::Paren(inner) = &e.kind {
        e = inner;
    }
    if let ExprKind::Ident(h) = &e.kind {
        if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) {
            // A name, possibly hierarchical (`u_sub.sig`, `top.x`).
            let name = ident_path(h);
            if let Some(r) = m
                .resolve_name(scope, &name)
                .or_else(|| m.by_name.get(&name).copied())
            {
                return ref_handle(m, sim, r);
            }
            // A genvar inside its loop is that element's index.
            if let Some(v) = m.genvar_value(scope, &name) {
                return int_const(v);
            }
        }
    }
    if let Some(ct) = literal_const_type(e) {
        let e = e.clone();
        let v = sim.eval_expr(&e);
        return const_handle(v, ct);
    }
    plain_obj_handle(m, o)
}

/// The dotted spelling of a hierarchical identifier's path.
fn ident_path(h: &HierarchicalIdentifier) -> String {
    h.path
        .iter()
        .map(|s| s.name.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// Rewrite the names of an expression in `scope` to their flat signal
/// names, so the simulator's evaluator reads the right signals.
fn flatten_expr(m: &VpiModel, scope: u32, e: &Expression) -> Expression {
    let mut out = e.clone();
    flatten_in_place(m, scope, &mut out);
    out
}

fn flatten_in_place(m: &VpiModel, scope: u32, e: &mut Expression) {
    // A genvar of an enclosing generate loop is the loop element's index.
    if let ExprKind::Ident(h) = &e.kind {
        if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() {
            if let Some(v) = m.genvar_value(scope, &h.path[0].name.name) {
                e.kind = ExprKind::Number(NumberLiteral::Integer {
                    size: None,
                    signed: true,
                    base: NumberBase::Decimal,
                    value: v.to_string(),
                    cached_val: Cell::new(None),
                });
                e.cached_width.set(None);
                return;
            }
        }
    }
    match &mut e.kind {
        ExprKind::Ident(h) => {
            if h.path.is_empty() || h.root.is_some() {
                return;
            }
            for seg in h.path.iter_mut() {
                for s in seg.selects.iter_mut() {
                    flatten_in_place(m, scope, s);
                }
            }
            let flat_of = |r: Option<MRef>| match r {
                Some(MRef::Obj(o)) => match &m.objs[o as usize].d {
                    OData::Var(v) => Some(v.flat.clone()),
                    OData::Param { flat, .. } => Some(flat.clone()),
                    _ => None,
                },
                _ => None,
            };
            let n = h.path.len();
            // The whole (hierarchical) name, selects allowed on its last
            // segment only; else its first segment.
            let whole = if h.path[..n - 1].iter().all(|s| s.selects.is_empty()) {
                let name = ident_path(h);
                flat_of(
                    m.resolve_name(scope, &name)
                        .or_else(|| m.by_name.get(&name).copied()),
                )
            } else {
                None
            };
            if let Some(flat) = whole {
                if let Some(last) = h.path.pop() {
                    h.path = vec![HierPathSegment {
                        name: Identifier {
                            name: flat,
                            span: last.name.span,
                        },
                        selects: last.selects,
                    }];
                }
            } else if let Some(flat) = flat_of(m.resolve_name(scope, &h.path[0].name.name)) {
                h.path[0].name.name = flat;
            } else {
                return;
            }
            h.cached_signal_id.set(None);
            h.cached_resolved_name = std::cell::OnceCell::new();
        }
        ExprKind::Unary { operand, .. } => flatten_in_place(m, scope, operand),
        ExprKind::Binary { left, right, .. } => {
            flatten_in_place(m, scope, left);
            flatten_in_place(m, scope, right);
        }
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            flatten_in_place(m, scope, condition);
            flatten_in_place(m, scope, then_expr);
            flatten_in_place(m, scope, else_expr);
        }
        ExprKind::Concatenation(v) => {
            for x in v.iter_mut() {
                flatten_in_place(m, scope, x);
            }
        }
        ExprKind::Replication { count, exprs } => {
            flatten_in_place(m, scope, count);
            for x in exprs.iter_mut() {
                flatten_in_place(m, scope, x);
            }
        }
        ExprKind::Index { expr, index } => {
            flatten_in_place(m, scope, expr);
            flatten_in_place(m, scope, index);
        }
        ExprKind::RangeSelect {
            expr, left, right, ..
        } => {
            flatten_in_place(m, scope, expr);
            flatten_in_place(m, scope, left);
            flatten_in_place(m, scope, right);
        }
        ExprKind::MemberAccess { expr, .. } => flatten_in_place(m, scope, expr),
        ExprKind::Paren(x) => flatten_in_place(m, scope, x),
        ExprKind::Inside { expr, ranges } => {
            flatten_in_place(m, scope, expr);
            for x in ranges.iter_mut() {
                flatten_in_place(m, scope, x);
            }
        }
        ExprKind::SystemCall { args, .. } | ExprKind::Call { args, .. } => {
            for x in args.iter_mut() {
                flatten_in_place(m, scope, x);
            }
        }
        _ => {}
    }
    e.cached_width.set(None);
}

/// Evaluate an expression object in its scope.
fn eval_obj(m: &VpiModel, sim: &mut Simulator, o: u32) -> Option<Value> {
    match &m.objs[o as usize].d {
        OData::Expr(ed) => {
            let e = flatten_expr(m, ed.scope, &ed.expr);
            Some(sim.eval_expr(&e))
        }
        OData::PrimTerm { expr, .. }
        | OData::PathTerm { expr, .. }
        | OData::TchkTerm { expr, .. } => eval_obj(m, sim, *expr),
        OData::EnumConst { value, width } => Some(Value::from_u64(*value, (*width).max(1))),
        OData::Param { flat, .. } => sim.module.parameters.get(flat.as_str()).cloned(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Identifying a handle in the model
// ---------------------------------------------------------------------------

/// The model object a handle refers to.
fn ident(m: &VpiModel, h: &VpiHandle) -> Option<MRef> {
    match h.kind {
        VpiKind::Module => m.inst_scope.get(&h.inst_idx).map(|&s| MRef::Scope(s)),
        VpiKind::Scope => {
            Some(MRef::Scope(h.signal_id as u32)).filter(|_| h.signal_id < m.scopes.len())
        }
        VpiKind::Obj => {
            if h.signal_id < m.objs.len() {
                Some(MRef::Obj(h.signal_id as u32))
            } else {
                None
            }
        }
        VpiKind::Port => m
            .port_by_name
            .get(&h.full_name)
            .map(|&PortIdx(o)| MRef::Obj(o)),
        VpiKind::Signal | VpiKind::Memory | VpiKind::Slice => match m.by_name.get(&h.full_name) {
            Some(&MRef::Obj(o))
                if matches!(m.objs[o as usize].d, OData::Var(_) | OData::Param { .. }) =>
            {
                Some(MRef::Obj(o))
            }
            _ => None,
        },
        _ => None,
    }
}

/// A bit or element handle (`w[3]`, `mem[1]`): its base object and index.
fn split_select(full: &str) -> Option<(&str, i64)> {
    let s = full.strip_suffix(']')?;
    let open = s.rfind('[')?;
    let idx = s[open + 1..].trim().parse().ok()?;
    Some((&full[..open], idx))
}

// ---------------------------------------------------------------------------
// vpi_handle_by_name
// ---------------------------------------------------------------------------

/// Resolve `name`, relative to `scope` when one is given. `None` leaves the
/// lookup to the flat signal table.
pub(super) fn handle_by_name(
    sim: &mut Simulator,
    name: &str,
    scope: Option<&VpiHandle>,
) -> Option<*mut libc::c_void> {
    let found = with_model(sim, |m, sim| {
        let mut cands: Vec<String> = Vec::new();
        if let Some(sh) = scope {
            if let Some(MRef::Scope(s)) = ident(m, sh) {
                let sc = &m.scopes[s as usize];
                if sc.kind == SKind::Package {
                    cands.push(format!("{}{}", sc.full, name));
                } else {
                    cands.push(format!("{}.{}", sc.full, name));
                }
            }
        }
        cands.push(name.to_string());
        // `pkg.x` for `pkg::x`.
        if let Some((p, rest)) = name.split_once('.') {
            if m.packages.iter().any(|&s| m.scopes[s as usize].name == p) {
                cands.push(format!("{}::{}", p, rest));
            }
        }
        for cand in &cands {
            if let Some(&r) = m.by_name.get(cand) {
                return Some(Some(ref_handle(m, sim, r)));
            }
        }
        // A generate block or instance placeholder signal is not an object.
        let rel = vpi_strip_top(sim, name);
        if m.placeholders.contains(rel) {
            return Some(None);
        }
        None
    })
    .flatten();
    match found {
        Some(Some(h)) => Some(h.into_raw()),
        Some(None) => Some(std::ptr::null_mut()),
        None => select_by_name(sim, name, scope),
    }
}

/// `base[i]`: an element of an array or a bit of a vector.
fn select_by_name(
    sim: &mut Simulator,
    name: &str,
    scope: Option<&VpiHandle>,
) -> Option<*mut libc::c_void> {
    let (base, idx) = split_select(name)?;
    let bh = handle_by_name(sim, base, scope)?;
    if bh.is_null() {
        return None;
    }
    let b = unsafe { Box::from_raw(bh as *mut VpiHandle) };
    let r = select(sim, &b, idx);
    r.map(|h| h.into_raw())
}

/// Element `idx` of an array handle, or bit `idx` of a vector handle.
fn select(sim: &mut Simulator, b: &VpiHandle, idx: i64) -> Option<VpiHandle> {
    with_model(sim, |m, sim| {
        let (o, base_full, mut indices) = match ident(m, b) {
            Some(MRef::Obj(o)) => (o, b.full_name.clone(), Vec::new()),
            Some(MRef::Scope(_)) => return None,
            None => resolve_select(m, &b.full_name)?,
        };
        indices.push(idx);
        select_chain(m, sim, o, &base_full, &indices)
    })
    .flatten()
}

/// A bit or element name (`top.mem[1]`, `top.m2[1][0][3]`): its declared
/// object, that object's full name, and the indices applied to it.
fn resolve_select(m: &VpiModel, full: &str) -> Option<(u32, String, Vec<i64>)> {
    let mut idx: Vec<i64> = Vec::new();
    let mut s = full;
    while let Some((base, i)) = split_select(s) {
        idx.push(i);
        s = base;
        if let Some(&MRef::Obj(o)) = m.by_name.get(s) {
            idx.reverse();
            return Some((o, s.to_string(), idx));
        }
    }
    None
}

/// Object `o` (full name `base_full`) with `indices` applied: an element or
/// sub-array of an array, then at most one bit of the (element's) vector.
fn select_chain(
    m: &mut VpiModel,
    sim: &mut Simulator,
    o: u32,
    base_full: &str,
    indices: &[i64],
) -> Option<VpiHandle> {
    let (flat, td, sig, net) = match &m.objs[o as usize].d {
        OData::Var(v) if v.member_of == NONE => (v.flat.clone(), v.td, v.sig, v.net_type != 0),
        OData::Param { flat, td, sig, .. } => (flat.clone(), *td, *sig, false),
        _ => return None,
    };
    let t = m.tdescs[td as usize].clone();
    let nun = t.unpacked.len();
    if indices.is_empty() {
        return None;
    }
    if indices.len() <= nun {
        return element_handle(m, sim, o, &flat, &t, base_full, indices);
    }
    if indices.len() != nun + 1 {
        return None;
    }
    // A bit of a packed vector: the object's own, or its element's.
    let (id, elem_full) = if nun == 0 {
        (sig?, base_full.to_string())
    } else {
        let eh = element_handle(m, sim, o, &flat, &t, base_full, &indices[..nun])?;
        if eh.kind != VpiKind::Signal {
            return None;
        }
        (eh.signal_id, eh.full_name.clone())
    };
    let idx = indices[nun];
    let w = sim.signal_widths.get(id).copied().unwrap_or(0) as i64;
    let phys = match t.packed.first() {
        Some(&(l, r)) => {
            let (lo, hi) = (l.min(r), l.max(r));
            if idx < lo || idx > hi || t.packed.len() > 1 {
                return None;
            }
            if l >= r { idx - r } else { r - idx }
        }
        None if w > 1 && idx >= 0 && idx < w => idx,
        None => return None,
    };
    let ty = if net { c::NET_BIT } else { c::REG_BIT };
    let leaf = elem_full
        .rsplit('.')
        .next()
        .unwrap_or(&elem_full)
        .to_string();
    let mut h = VpiHandle::signal(
        id,
        ty,
        &format!("{}[{}]", leaf, idx),
        &format!("{}[{}]", elem_full, idx),
    );
    h.kind = VpiKind::Slice;
    h.lsb = phys as u32;
    h.width = 1;
    Some(h)
}

/// The element of array object `o` at `idx` (one index per dimension from
/// the outermost; fewer indices select a sub-array).
fn element_handle(
    m: &mut VpiModel,
    sim: &mut Simulator,
    o: u32,
    flat: &str,
    t: &TDesc,
    full: &str,
    idx: &[i64],
) -> Option<VpiHandle> {
    let mut fname = flat.to_string();
    let mut vname = full.to_string();
    for (k, i) in idx.iter().enumerate() {
        match t.unpacked.get(k)? {
            UDim::Range(l, r) => {
                if *i < (*l).min(*r) || *i > (*l).max(*r) {
                    return None;
                }
            }
            UDim::Dynamic | UDim::Queue => {
                let size = sim
                    .signals
                    .get(&format!("{}.size", fname))
                    .and_then(|v| v.to_u64())
                    .unwrap_or(0) as i64;
                if *i < 0 || *i >= size {
                    return None;
                }
            }
            UDim::Assoc => {}
        }
        fname = format!("{}[{}]", fname, i);
        vname = format!("{}[{}]", vname, i);
    }
    let leaf = vname.rsplit('.').next().unwrap_or(&vname).to_string();
    let rest = &t.unpacked[idx.len()..];
    let mut et = t.clone();
    et.unpacked = rest.to_vec();
    let net = matches!(&m.objs[o as usize].d, OData::Var(v) if v.net_type != 0);
    if !rest.is_empty() {
        // A sub-array.
        let id = {
            let mut probe = fname.clone();
            let mut found = None;
            for d in rest {
                let lo = match d {
                    UDim::Range(l, r) => (*l).min(*r),
                    _ => 0,
                };
                probe = format!("{}[{}]", probe, lo);
                if let Some(&id) = sim.signal_name_to_id.get(probe.as_str()) {
                    found = Some(id);
                }
            }
            found?
        };
        let mut h = VpiHandle::signal(
            id,
            if net { c::NET_ARRAY } else { c::REG_ARRAY },
            &leaf,
            &vname,
        );
        h.kind = VpiKind::Memory;
        h.width = fixed_count(&et).unwrap_or(0);
        return Some(h);
    }
    let id = *sim.signal_name_to_id.get(fname.as_str())?;
    let ty = if net {
        m.net_type_code(&et)
    } else {
        m.var_type_code(&et)
    };
    Some(VpiHandle::signal(id, ty, &leaf, &vname))
}

/// `vpi_handle_by_index` on a model array or generate scope array.
pub(super) fn handle_by_index(
    sim: &mut Simulator,
    h: &VpiHandle,
    index: i64,
) -> Option<*mut libc::c_void> {
    let r = with_model(sim, |m, sim| match ident(m, h)? {
        MRef::Obj(o) => {
            if let OData::GenArray { elems } = &m.objs[o as usize].d {
                let s = elems
                    .iter()
                    .copied()
                    .find(|&s| m.scopes[s as usize].index == index)?;
                return Some(Some(scope_handle(m, s)));
            }
            None
        }
        _ => None,
    })
    .flatten();
    if let Some(r) = r {
        return Some(r.map(|h| h.into_raw()).unwrap_or(std::ptr::null_mut()));
    }
    if matches!(h.kind, VpiKind::Memory | VpiKind::Signal | VpiKind::Slice) {
        // A model array, a sub-array or element of one, or a vector.
        let known = with_model(sim, |m, _| {
            ident(m, h).is_some() || resolve_select(m, &h.full_name).is_some()
        })
        .unwrap_or(false);
        if known {
            return Some(
                select(sim, h, index)
                    .map(|h| h.into_raw())
                    .unwrap_or(std::ptr::null_mut()),
            );
        }
    }
    None
}

// ---------------------------------------------------------------------------
// vpi_handle
// ---------------------------------------------------------------------------

/// One-to-one relations. `None` leaves the call to the flat-table code.
pub(super) fn handle(
    sim: &mut Simulator,
    rel: c_int,
    refh: Option<&VpiHandle>,
) -> Option<*mut libc::c_void> {
    with_model(sim, |m, sim| {
        let out =
            |h: Option<VpiHandle>| Some(h.map(|h| h.into_raw()).unwrap_or(std::ptr::null_mut()));
        let Some(h) = refh else {
            // xezim extension: vpiScope of NULL is the (first) top.
            if rel == c::SCOPE {
                return out(m.tops.first().map(|&s| scope_handle(m, s)));
            }
            return None;
        };
        // Bits and elements made by `select`/`element_handle`.
        let r = match ident(m, h) {
            Some(r) => r,
            None => {
                if matches!(h.kind, VpiKind::Slice | VpiKind::Signal | VpiKind::Memory) {
                    if let Some((o, base_full, ix)) = resolve_select(m, &h.full_name) {
                        let last = *ix.last().unwrap_or(&0);
                        return match rel {
                            c::PARENT => out(if ix.len() == 1 {
                                Some(obj_handle(m, sim, o))
                            } else {
                                select_chain(m, sim, o, &base_full, &ix[..ix.len() - 1])
                            }),
                            c::INDEX => out(Some(int_const(last))),
                            c::SCOPE => {
                                let s = m.objs[o as usize].scope;
                                out((s != NONE).then(|| scope_handle(m, s)))
                            }
                            c::TYPESPEC => {
                                let mut td = match &m.objs[o as usize].d {
                                    OData::Var(v) => v.td,
                                    OData::Param { td, .. } => *td,
                                    _ => return out(None),
                                };
                                for _ in 0..ix.len() {
                                    if !td_is_array(m, td) {
                                        // A bit has no typespec of its own.
                                        return out(None);
                                    }
                                    td = m.elem_td(td);
                                }
                                let ts = m.typespec_obj(td);
                                out(Some(plain_obj_handle(m, ts)))
                            }
                            _ => None,
                        };
                    }
                }
                if h.kind == VpiKind::Obj && h.signal_id == usize::MAX {
                    // A range made on the fly has its bounds; a constant has
                    // no relations.
                    if h.type_code == c::RANGE && (rel == c::LEFT_RANGE || rel == c::RIGHT_RANGE) {
                        let v = if rel == c::LEFT_RANGE { h.lsb } else { h.width };
                        return out(Some(int_const(v as i32 as i64)));
                    }
                    return out(None);
                }
                return None;
            }
        };
        // A select's parent is the object it selects from.
        if let MRef::Obj(o) = r {
            if rel == c::PARENT && matches!(m.objs[o as usize].d, OData::Expr(_)) {
                return obj_rel(m, sim, rel, o, h);
            }
        }
        match rel {
            c::SCOPE => {
                let s = match r {
                    MRef::Scope(s) => m.scopes[s as usize].parent,
                    MRef::Obj(o) => m.objs[o as usize].scope,
                };
                out((s != NONE).then(|| scope_handle(m, s)))
            }
            c::PARENT => {
                let p = match r {
                    MRef::Scope(s) => {
                        let sc = &m.scopes[s as usize];
                        if sc.gen_array != NONE {
                            let a = sc.gen_array;
                            return out(Some(plain_obj_handle(m, a)));
                        }
                        sc.parent
                    }
                    MRef::Obj(o) => {
                        let ob = &m.objs[o as usize];
                        if ob.parent != NONE {
                            let p = ob.parent;
                            return out(Some(obj_handle(m, sim, p)));
                        }
                        ob.scope
                    }
                };
                out((p != NONE).then(|| scope_handle(m, p)))
            }
            c::MODULE | c::INSTANCE => {
                // The instance around the object (for an instance, the one
                // that instantiates it).
                let s = match r {
                    MRef::Scope(s) => m.scopes[s as usize].parent,
                    MRef::Obj(o) => m.objs[o as usize].scope,
                };
                let i = m.instance_of(s);
                if i == NONE {
                    return out(None);
                }
                if rel == c::MODULE && m.scopes[i as usize].kind != SKind::Module {
                    return out(None);
                }
                out(Some(scope_handle(m, i)))
            }
            _ => match r {
                MRef::Scope(s) => scope_rel(m, sim, rel, s),
                MRef::Obj(o) => obj_rel(m, sim, rel, o, h),
            },
        }
    })
    .flatten()
}

fn scope_rel(
    m: &mut VpiModel,
    _sim: &mut Simulator,
    rel: c_int,
    s: u32,
) -> Option<*mut libc::c_void> {
    let out = |h: Option<VpiHandle>| Some(h.map(|h| h.into_raw()).unwrap_or(std::ptr::null_mut()));
    let sc = &m.scopes[s as usize];
    match rel {
        c::INDEX if sc.gen_array != NONE => out(Some(int_const(sc.index))),
        c::TYPESPEC if sc.kind == SKind::Function && sc.ret_td != NONE => {
            let td = sc.ret_td;
            let ts = m.typespec_obj(td);
            out(Some(plain_obj_handle(m, ts)))
        }
        _ => out(None),
    }
}

fn range_ends(m: &VpiModel, td: u32, sim_width: Option<u32>) -> Option<(i64, i64)> {
    let t = &m.tdescs[td as usize];
    if let Some(UDim::Range(l, r)) = t.unpacked.first() {
        return Some((*l, *r));
    }
    if !t.unpacked.is_empty() {
        return None;
    }
    if let Some(&p) = t.packed.first() {
        return Some(p);
    }
    let w = match &t.kind {
        TKind::Integer
        | TKind::Int
        | TKind::ShortInt
        | TKind::LongInt
        | TKind::Byte
        | TKind::Time => m.td_width(t),
        TKind::Struct(s) if !m.structs[*s as usize].packed => return None,
        TKind::Enum(_) | TKind::Struct(_) => m.td_width(t),
        TKind::Unknown => sim_width.unwrap_or(0),
        _ => return None,
    };
    if w == 0 {
        return None;
    }
    Some((w as i64 - 1, 0))
}

fn obj_td(m: &VpiModel, o: u32) -> Option<u32> {
    match &m.objs[o as usize].d {
        OData::Var(v) => Some(v.td),
        OData::Param { td, .. } => Some(*td),
        OData::IoDecl { td, .. } => Some(*td),
        OData::Typespec { td } | OData::TypespecMember { td } => Some(*td),
        _ => None,
    }
}

fn obj_rel(
    m: &mut VpiModel,
    sim: &mut Simulator,
    rel: c_int,
    o: u32,
    h: &VpiHandle,
) -> Option<*mut libc::c_void> {
    let out = |h: Option<VpiHandle>| Some(h.map(|h| h.into_raw()).unwrap_or(std::ptr::null_mut()));
    let sub = |m: &mut VpiModel, sim: &mut Simulator, x: u32| -> Option<VpiHandle> {
        (x != NONE).then(|| obj_handle(m, sim, x))
    };
    if let OData::Expr(ed) = &m.objs[o as usize].d {
        let scope = ed.scope;
        let mut e: Expression = (*ed.expr).clone();
        while let ExprKind::Paren(inner) = e.kind {
            e = *inner;
        }
        let eval_int = |m: &VpiModel, sim: &mut Simulator, x: &Expression| -> Option<VpiHandle> {
            let v = sim.eval_expr(&flatten_expr(m, scope, x));
            v.to_i64().map(int_const)
        };
        match (rel, &e.kind) {
            (c::PARENT, ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. }) => {
                if let ExprKind::Ident(hi) = &expr.kind {
                    if hi.path.len() == 1 && hi.path[0].selects.is_empty() {
                        if let Some(r) = m.resolve_name(scope, &hi.path[0].name.name) {
                            return out(Some(ref_handle(m, sim, r)));
                        }
                    }
                }
                return out(None);
            }
            (c::INDEX, ExprKind::Index { index, .. }) => return out(eval_int(m, sim, index)),
            (
                c::LEFT_RANGE | c::RIGHT_RANGE,
                ExprKind::RangeSelect {
                    kind: RangeKind::Constant,
                    left,
                    right,
                    ..
                },
            ) => {
                let x = if rel == c::LEFT_RANGE { left } else { right };
                return out(eval_int(m, sim, x));
            }
            _ => {}
        }
    }
    match rel {
        c::LEFT_RANGE | c::RIGHT_RANGE => {
            if let OData::Range { left, right } = m.objs[o as usize].d {
                return out(Some(int_const(if rel == c::LEFT_RANGE {
                    left
                } else {
                    right
                })));
            }
            if let Some((l, r)) = dyn_bounds(m, sim, o) {
                return out(Some(int_const(if rel == c::LEFT_RANGE { l } else { r })));
            }
            let td = obj_td(m, o)?;
            let w = if matches!(h.kind, VpiKind::Signal | VpiKind::Port) {
                sim.signal_widths.get(h.signal_id).copied()
            } else {
                None
            };
            let r = range_ends(m, td, w);
            out(r.map(|(l, rr)| int_const(if rel == c::LEFT_RANGE { l } else { rr })))
        }
        c::TYPESPEC => {
            let td = match &m.objs[o as usize].d {
                OData::TypespecMember { td } => *td,
                _ => match obj_td(m, o) {
                    Some(td) if !matches!(m.objs[o as usize].d, OData::Typespec { .. }) => td,
                    _ => return out(None),
                },
            };
            let ts = m.typespec_obj(td);
            out(Some(plain_obj_handle(m, ts)))
        }
        c::ELEM_TYPESPEC => {
            let OData::Typespec { td } = m.objs[o as usize].d else {
                return out(None);
            };
            if !td_is_array(m, td) {
                return out(None);
            }
            let e = m.elem_td(td);
            let ts = m.typespec_obj(e);
            out(Some(plain_obj_handle(m, ts)))
        }
        c::BASE_TYPESPEC => {
            let OData::Typespec { td } = m.objs[o as usize].d else {
                return out(None);
            };
            let TKind::Enum(e) = m.tdescs[td as usize].kind.clone() else {
                return out(None);
            };
            let base = m.enums[e as usize].base.clone();
            let bt = m.add_td(base);
            let ts = m.typespec_obj(bt);
            out(Some(plain_obj_handle(m, ts)))
        }
        _ => {
            let r = match &m.objs[o as usize].d {
                OData::ContAssign {
                    lhs, rhs, delay, ..
                } => match rel {
                    c::LHS => Some(*lhs),
                    c::RHS => Some(*rhs),
                    c::DELAY => Some(*delay),
                    _ => None,
                },
                OData::Prim { delay, .. } if rel == c::DELAY => Some(*delay),
                OData::PrimTerm { expr, .. }
                | OData::PathTerm { expr, .. }
                | OData::ModportIo { expr, .. }
                    if rel == c::EXPR =>
                {
                    Some(*expr)
                }
                OData::TchkTerm { expr, cond, .. } => match rel {
                    c::EXPR => Some(*expr),
                    c::CONDITION => Some(*cond),
                    _ => None,
                },
                OData::ModPath { cond, .. } if rel == c::CONDITION => Some(*cond),
                OData::Tchk {
                    ref_term,
                    data_term,
                    notifier,
                    ..
                } => match rel {
                    c::TCHK_REF_TERM => Some(*ref_term),
                    c::TCHK_DATA_TERM => Some(*data_term),
                    c::TCHK_NOTIFIER => Some(*notifier),
                    _ => None,
                },
                OData::Port(p) => match rel {
                    c::LOW_CONN => Some(p.low),
                    c::HIGH_CONN => Some(p.high),
                    _ => None,
                },
                OData::Process { stmt, .. } if rel == c::STMT => {
                    let s = *stmt;
                    return out((s != NONE).then(|| scope_handle(m, s)));
                }
                OData::GenArray { .. } => None,
                _ => None,
            };
            match r {
                Some(x) => out(sub(m, sim, x)),
                None => out(None),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// vpi_iterate
// ---------------------------------------------------------------------------

fn is_net_type(ty: c_int) -> bool {
    matches!(
        ty,
        c::NET | c::ENUM_NET | c::INTEGER_NET | c::TIME_NET | c::STRUCT_NET | c::PACKED_ARRAY_NET
    )
}

/// One-to-many relations. `None` leaves the call to the flat-table code.
pub(super) fn iterate(
    sim: &mut Simulator,
    rel: c_int,
    refh: Option<&VpiHandle>,
) -> Option<*mut libc::c_void> {
    with_model(sim, |m, sim| {
        let items: Vec<VpiHandle> = match refh {
            None => {
                let pick = |m: &VpiModel, k: &[SKind]| -> Vec<u32> {
                    m.tops
                        .iter()
                        .copied()
                        .filter(|&s| k.contains(&m.scopes[s as usize].kind))
                        .collect()
                };
                let v: Vec<u32> = match rel {
                    c::MODULE => pick(m, &[SKind::Module]),
                    c::INTERFACE => pick(m, &[SKind::Interface]),
                    c::PROGRAM => pick(m, &[SKind::Program]),
                    c::INSTANCE => {
                        let mut v = pick(m, &[SKind::Module, SKind::Interface, SKind::Program]);
                        v.extend(m.packages.iter().copied());
                        v
                    }
                    c::PACKAGE => m.packages.clone(),
                    _ => return None,
                };
                v.into_iter().map(|s| scope_handle(m, s)).collect()
            }
            Some(h) => match ident(m, h) {
                Some(MRef::Scope(s)) => scope_iter(m, sim, rel, s)?,
                Some(MRef::Obj(o)) => obj_iter(m, sim, rel, o, h)?,
                None => {
                    if h.kind == VpiKind::Obj && h.signal_id == usize::MAX {
                        Vec::new()
                    } else {
                        return None;
                    }
                }
            },
        };
        Some(vpi_make_iterator(items))
    })
    .flatten()
}

fn scope_iter(m: &mut VpiModel, sim: &mut Simulator, rel: c_int, s: u32) -> Option<Vec<VpiHandle>> {
    let children: Vec<u32> = m.scopes[s as usize].children.clone();
    let members: Vec<u32> = m.scopes[s as usize].members.clone();
    let kids = |m: &VpiModel, k: &[SKind]| -> Vec<VpiHandle> {
        children
            .iter()
            .copied()
            .filter(|&c| k.contains(&m.scopes[c as usize].kind))
            .map(|c| scope_handle(m, c))
            .collect()
    };
    let objs_where = |m: &mut VpiModel,
                      sim: &mut Simulator,
                      f: &dyn Fn(&VpiModel, &MObj) -> bool|
     -> Vec<VpiHandle> {
        let sel: Vec<u32> = members
            .iter()
            .copied()
            .filter(|&o| f(m, &m.objs[o as usize]))
            .collect();
        sel.into_iter().map(|o| obj_handle(m, sim, o)).collect()
    };
    Some(match rel {
        c::MODULE => kids(m, &[SKind::Module]),
        c::INTERFACE => kids(m, &[SKind::Interface]),
        c::PROGRAM => kids(m, &[SKind::Program]),
        c::INSTANCE => kids(m, &[SKind::Module, SKind::Interface, SKind::Program]),
        c::INTERNAL_SCOPE => children.iter().map(|&c| scope_handle(m, c)).collect(),
        c::GEN_SCOPE => kids(m, &[SKind::GenScope]),
        c::TASK_FUNC => kids(m, &[SKind::Task, SKind::Function]),
        c::TASK => kids(m, &[SKind::Task]),
        c::FUNCTION => kids(m, &[SKind::Function]),
        c::NAMED_BEGIN => kids(m, &[SKind::NamedBegin]),
        c::NAMED_FORK => kids(m, &[SKind::NamedFork]),
        c::VARIABLES => objs_where(m, sim, &|_, ob| {
            matches!(&ob.d, OData::Var(v) if v.net_type == 0)
                && !matches!(ob.type_code, c::NAMED_EVENT | c::NAMED_EVENT_ARRAY)
        }),
        c::NET => objs_where(m, sim, &|_, ob| {
            matches!(&ob.d, OData::Var(v) if v.net_type != 0) && is_net_type(ob.type_code)
        }),
        c::MEMORY => objs_where(m, sim, &|m, ob| match &ob.d {
            OData::Var(v) if v.net_type == 0 && ob.type_code == c::REG_ARRAY => {
                let t = &m.tdescs[v.td as usize];
                t.unpacked.len() == 1
                    && matches!(t.unpacked[0], UDim::Range(..))
                    && matches!(t.kind, TKind::Logic | TKind::Reg | TKind::Implicit)
            }
            _ => false,
        }),
        c::PARAMETER => objs_where(m, sim, &|_, ob| matches!(ob.d, OData::Param { .. })),
        c::PORT => {
            let mut v: Vec<u32> = members
                .iter()
                .copied()
                .filter(|&o| matches!(m.objs[o as usize].d, OData::Port(_)))
                .collect();
            v.sort_by_key(|&o| match &m.objs[o as usize].d {
                OData::Port(p) => p.index,
                _ => 0,
            });
            v.into_iter().map(|o| obj_handle(m, sim, o)).collect()
        }
        c::PROCESS => objs_where(m, sim, &|_, ob| matches!(ob.d, OData::Process { .. })),
        c::PRIMITIVE => objs_where(m, sim, &|_, ob| matches!(ob.d, OData::Prim { .. })),
        c::GATE | c::SWITCH | c::UDP => objs_where(m, sim, &|_, ob| {
            matches!(ob.d, OData::Prim { .. }) && ob.type_code == rel
        }),
        c::CONT_ASSIGN
        | c::GEN_SCOPE_ARRAY
        | c::MOD_PATH
        | c::TCHK
        | c::IO_DECL
        | c::NAMED_EVENT
        | c::NAMED_EVENT_ARRAY
        | c::ALWAYS
        | c::INITIAL
        | c::FINAL
        | c::MODPORT => objs_where(m, sim, &|_, ob| ob.type_code == rel),
        // A specific variable or net type (vpiReg, vpiIntVar, vpiNetArray,
        // vpiRegArray/vpiArrayVar, vpiStructVar, ...).
        _ if type_name(rel).is_some() => objs_where(m, sim, &|_, ob| {
            ob.type_code == rel && matches!(ob.d, OData::Var(_))
        }),
        _ => Vec::new(),
    })
}

fn obj_iter(
    m: &mut VpiModel,
    sim: &mut Simulator,
    rel: c_int,
    o: u32,
    h: &VpiHandle,
) -> Option<Vec<VpiHandle>> {
    let mut v: Vec<VpiHandle> = Vec::new();
    match rel {
        c::RANGE => {
            let td = match &m.objs[o as usize].d {
                OData::Var(v) => v.td,
                OData::Param { td, .. } | OData::IoDecl { td, .. } | OData::Typespec { td } => *td,
                _ => return Some(v),
            };
            let t = m.tdescs[td as usize].clone();
            if let Some((l, r)) = dyn_bounds(m, sim, o) {
                v.push(dyn_range_handle(l, r));
                return Some(v);
            }
            let packed = t.unpacked.is_empty();
            let n = if packed {
                t.packed.len()
            } else {
                t.unpacked.len()
            };
            for d in 0..n {
                if let Some(r) = m.range_obj(td, packed, d as u32) {
                    v.push(plain_obj_handle(m, r));
                }
            }
        }
        c::GEN_SCOPE => {
            if let OData::GenArray { elems } = &m.objs[o as usize].d {
                let e = elems.clone();
                v = e.into_iter().map(|s| scope_handle(m, s)).collect();
            }
        }
        c::PRIM_TERM => {
            if let OData::Prim { terms, .. } = &m.objs[o as usize].d {
                let t = terms.clone();
                v = t.into_iter().map(|x| obj_handle(m, sim, x)).collect();
            }
        }
        c::MOD_PATH_IN | c::MOD_PATH_OUT => {
            if let OData::ModPath { ins, outs, .. } = &m.objs[o as usize].d {
                let t = if rel == c::MOD_PATH_IN {
                    ins.clone()
                } else {
                    outs.clone()
                };
                v = t.into_iter().map(|x| obj_handle(m, sim, x)).collect();
            }
        }
        c::MEMBER => {
            if let OData::Var(vd) = &m.objs[o as usize].d {
                let t = vd.members.clone();
                v = t.into_iter().map(|x| obj_handle(m, sim, x)).collect();
            }
        }
        c::ENUM_CONST | c::TYPESPEC_MEMBER => {
            let OData::Typespec { td } = m.objs[o as usize].d else {
                return Some(v);
            };
            match (rel, m.tdescs[td as usize].kind.clone()) {
                (c::ENUM_CONST, TKind::Enum(e)) if m.tdescs[td as usize].unpacked.is_empty() => {
                    let consts = m.enums[e as usize].consts.clone();
                    let width = m.td_width(&m.enums[e as usize].base.clone()).max(1);
                    for (i, (name, value)) in consts.into_iter().enumerate() {
                        let x = m.memo_obj((6, o, i as u32), |_| MObj {
                            type_code: c::ENUM_CONST,
                            name,
                            full: String::new(),
                            scope: NONE,
                            parent: o,
                            file: NONE,
                            line: 0,
                            d: OData::EnumConst { value, width },
                        });
                        v.push(plain_obj_handle(m, x));
                    }
                }
                (c::TYPESPEC_MEMBER, TKind::Struct(s))
                    if m.tdescs[td as usize].unpacked.is_empty() =>
                {
                    let members = m.structs[s as usize].members.clone();
                    for (i, (name, mt, line)) in members.into_iter().enumerate() {
                        let x = m.memo_obj((7, o, i as u32), |m| {
                            let mtd = m.add_td(mt);
                            MObj {
                                type_code: c::TYPESPEC_MEMBER,
                                name,
                                full: String::new(),
                                scope: NONE,
                                parent: o,
                                file: NONE,
                                line,
                                d: OData::TypespecMember { td: mtd },
                            }
                        });
                        v.push(plain_obj_handle(m, x));
                    }
                }
                _ => {}
            }
        }
        c::IO_DECL => {
            if let OData::Modport { ios } = &m.objs[o as usize].d {
                let t = ios.clone();
                v = t.into_iter().map(|x| obj_handle(m, sim, x)).collect();
            }
        }
        c::BIT | c::PORT_BIT | c::NET_BIT | c::REG_BIT => {
            // Bits of a vector port or variable.
            let (td, base, net, dir, sig) = match &m.objs[o as usize].d {
                OData::Port(p) if p.low != NONE => match &m.objs[p.low as usize].d {
                    OData::Var(lv) => (lv.td, h.full_name.clone(), lv.net_type != 0, p.dir, lv.sig),
                    _ => return Some(v),
                },
                OData::Var(vd) if vd.member_of == NONE => (
                    vd.td,
                    h.full_name.clone(),
                    vd.net_type != 0,
                    c::NO_DIRECTION,
                    vd.sig,
                ),
                _ => return Some(v),
            };
            let is_port = matches!(m.objs[o as usize].d, OData::Port(_));
            let t = m.tdescs[td as usize].clone();
            let Some(id) = sig else { return Some(v) };
            if !t.unpacked.is_empty() {
                return Some(v);
            }
            let w = sim.signal_widths.get(id).copied().unwrap_or(0) as i64;
            if w <= 1 && t.packed.is_empty() {
                return Some(v);
            }
            let (l, r) = t.packed.first().copied().unwrap_or((w - 1, 0));
            if t.packed.len() > 1 {
                return Some(v);
            }
            let step = if l >= r { -1 } else { 1 };
            let mut i = l;
            let leaf = m.objs[o as usize].name.clone();
            loop {
                let phys = if l >= r { i - r } else { r - i };
                let ty = if is_port {
                    c::PORT_BIT
                } else if net {
                    c::NET_BIT
                } else {
                    c::REG_BIT
                };
                let mut bh = VpiHandle::signal(
                    id,
                    ty,
                    &format!("{}[{}]", leaf, i),
                    &format!("{}[{}]", base, i),
                );
                bh.kind = VpiKind::Slice;
                bh.lsb = phys as u32;
                bh.width = 1;
                bh.direction = dir;
                v.push(bh);
                if i == r {
                    break;
                }
                i += step;
            }
        }
        c::REG | c::NET | c::MEMORY_WORD | c::VAR_SELECT | c::REG_ARRAY | c::NET_ARRAY => {
            // Elements of an array, outermost dimension.
            let (flat, td) = match &m.objs[o as usize].d {
                OData::Var(vd) => (vd.flat.clone(), vd.td),
                _ => return Some(v),
            };
            let t = m.tdescs[td as usize].clone();
            let Some(first) = t.unpacked.first().copied() else {
                return Some(v);
            };
            let idxs: Vec<i64> = match first {
                UDim::Range(l, r) => {
                    if l <= r {
                        (l..=r).collect()
                    } else {
                        (r..=l).rev().collect()
                    }
                }
                UDim::Dynamic | UDim::Queue => {
                    let n = sim
                        .signals
                        .get(&format!("{}.size", flat))
                        .and_then(|x| x.to_u64())
                        .unwrap_or(0) as i64;
                    (0..n).collect()
                }
                UDim::Assoc => Vec::new(),
            };
            let full = h.full_name.clone();
            for i in idxs {
                if let Some(e) = element_handle(m, sim, o, &flat, &t, &full, &[i]) {
                    v.push(e);
                }
            }
        }
        c::OPERAND => {
            let OData::Expr(ed) = &m.objs[o as usize].d else {
                return Some(v);
            };
            let scope = ed.scope;
            let mut e: &Expression = &ed.expr;
            while let ExprKind::Paren(inner) = &e.kind {
                e = inner;
            }
            let ops: Vec<Expression> = match &e.kind {
                ExprKind::Unary { operand, .. } => vec![(**operand).clone()],
                ExprKind::Binary { left, right, .. } => vec![(**left).clone(), (**right).clone()],
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => vec![
                    (**condition).clone(),
                    (**then_expr).clone(),
                    (**else_expr).clone(),
                ],
                ExprKind::Concatenation(x) => x.clone(),
                ExprKind::Replication { count, exprs } => {
                    let mut x = vec![(**count).clone()];
                    x.extend(exprs.iter().cloned());
                    x
                }
                _ => Vec::new(),
            };
            let file = m.objs[o as usize].file;
            let line = m.objs[o as usize].line;
            for (i, e) in ops.into_iter().enumerate() {
                let ty = match &e.kind {
                    ExprKind::Number(_) | ExprKind::StringLiteral(_) => c::CONSTANT,
                    ExprKind::Ident(_) => c::REF_OBJ,
                    ExprKind::Index { .. } => c::BIT_SELECT,
                    ExprKind::RangeSelect { .. } => c::PART_SELECT,
                    _ => c::OPERATION,
                };
                let x = m.memo_obj((8, o, i as u32), |_| MObj {
                    type_code: ty,
                    name: String::new(),
                    full: String::new(),
                    scope,
                    parent: o,
                    file,
                    line,
                    d: OData::Expr(ExprData {
                        scope,
                        expr: Box::new(e),
                    }),
                });
                v.push(obj_handle(m, sim, x));
            }
        }
        c::PORT_INST | c::PORTS => {
            // The ports this net or variable connects to: from above
            // (vpiPortInst, the high connection of a child instance's port)
            // or from inside (vpiPorts, the module's own port).
            let scope = m.objs[o as usize].scope;
            if scope == NONE || !matches!(m.objs[o as usize].d, OData::Var(_)) {
                return Some(v);
            }
            let name = m.objs[o as usize].name.clone();
            let mut hits: Vec<u32> = Vec::new();
            if rel == c::PORTS {
                for &x in &m.scopes[scope as usize].members {
                    if let OData::Port(p) = &m.objs[x as usize].d {
                        if p.low == o {
                            hits.push(x);
                        }
                    }
                }
            } else {
                // Instances reachable from the declaring scope without
                // entering another instance: its children, and those of its
                // generate scopes and named blocks.
                let mut kids: Vec<u32> = Vec::new();
                let mut stack: Vec<u32> = m.scopes[scope as usize].children.clone();
                while let Some(c) = stack.pop() {
                    match m.scopes[c as usize].kind {
                        SKind::Module | SKind::Interface | SKind::Program => kids.push(c),
                        SKind::GenScope | SKind::NamedBegin | SKind::NamedFork => {
                            stack.extend(m.scopes[c as usize].children.iter().copied())
                        }
                        _ => {}
                    }
                }
                kids.sort_unstable();
                for c in kids {
                    for &x in &m.scopes[c as usize].members {
                        if let OData::Port(p) = &m.objs[x as usize].d {
                            if p.high != NONE {
                                if let OData::Expr(ed) = &m.objs[p.high as usize].d {
                                    if expr_names(&ed.expr, &name)
                                        && m.resolve_name(ed.scope, &name) == Some(MRef::Obj(o))
                                    {
                                        hits.push(x);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            v = hits.into_iter().map(|x| obj_handle(m, sim, x)).collect();
        }
        _ => return Some(v),
    }
    Some(v)
}

/// Does `e` name `name` (as a whole identifier) anywhere?
fn expr_names(e: &Expression, name: &str) -> bool {
    match &e.kind {
        ExprKind::Ident(h) => h.path.first().is_some_and(|s| s.name.name == name),
        ExprKind::Unary { operand, .. } => expr_names(operand, name),
        ExprKind::Binary { left, right, .. } => expr_names(left, name) || expr_names(right, name),
        ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            expr_names(condition, name)
                || expr_names(then_expr, name)
                || expr_names(else_expr, name)
        }
        ExprKind::Concatenation(v) => v.iter().any(|x| expr_names(x, name)),
        ExprKind::Replication { exprs, .. } => exprs.iter().any(|x| expr_names(x, name)),
        ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => expr_names(expr, name),
        ExprKind::Paren(x) => expr_names(x, name),
        ExprKind::MemberAccess { expr, .. } => expr_names(expr, name),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// vpi_get
// ---------------------------------------------------------------------------

fn op_type(e: &Expression) -> c_int {
    match &e.kind {
        ExprKind::Paren(x) => op_type(x),
        ExprKind::Unary { op, .. } => match op {
            UnaryOp::Minus => 1,
            UnaryOp::Plus => 2,
            UnaryOp::LogNot => 3,
            UnaryOp::BitNot => 4,
            UnaryOp::BitAnd => 5,
            UnaryOp::BitNand => 6,
            UnaryOp::BitOr => 7,
            UnaryOp::BitNor => 8,
            UnaryOp::BitXor => 9,
            UnaryOp::BitXnor => 10,
            UnaryOp::PostIncr => 62,
            UnaryOp::PreIncr => 63,
            UnaryOp::PostDecr => 64,
            UnaryOp::PreDecr => 65,
            _ => c::UNDEFINED,
        },
        ExprKind::Binary { op, .. } => match op {
            BinaryOp::Sub => 11,
            BinaryOp::Div => 12,
            BinaryOp::Mod => 13,
            BinaryOp::Eq => 14,
            BinaryOp::Neq => 15,
            BinaryOp::CaseEq => 16,
            BinaryOp::CaseNeq => 17,
            BinaryOp::Gt => 18,
            BinaryOp::Geq => 19,
            BinaryOp::Lt => 20,
            BinaryOp::Leq => 21,
            BinaryOp::ShiftLeft => 22,
            BinaryOp::ShiftRight => 23,
            BinaryOp::Add => 24,
            BinaryOp::Mul => 25,
            BinaryOp::LogAnd => 26,
            BinaryOp::LogOr => 27,
            BinaryOp::BitAnd => 28,
            BinaryOp::BitOr => 29,
            BinaryOp::BitXor => 30,
            BinaryOp::BitXnor => 31,
            BinaryOp::ArithShiftLeft => 41,
            BinaryOp::ArithShiftRight => 42,
            BinaryOp::Power => 43,
            BinaryOp::WildcardEq => 69,
            BinaryOp::WildcardNeq => 70,
            _ => c::UNDEFINED,
        },
        ExprKind::Conditional { .. } => 32,
        ExprKind::Concatenation(_) => 33,
        ExprKind::Replication { .. } => 34,
        ExprKind::Inside { .. } => 95,
        ExprKind::AssignmentPattern(_) => 75,
        ExprKind::StreamOp { left_to_right, .. } => {
            if *left_to_right {
                71
            } else {
                72
            }
        }
        _ => c::UNDEFINED,
    }
}

/// Integer properties. For a model scope or object every property is
/// answered here (`vpiUndefined` when it does not apply); for other handles
/// `None` leaves the call to the flat-table code.
/// The properties answered without the simulator: the location of a
/// `$systf` call, its constant arguments and an iterator (none the model
/// knows), and those of the objects made on the fly.
pub(super) fn get_static(prop: c_int, h: &VpiHandle) -> Option<c_int> {
    if matches!(
        h.kind,
        VpiKind::SysTfCall | VpiKind::Constant | VpiKind::Iterator
    ) {
        return (prop == c::LINE_NO).then_some(0);
    }
    None
}

pub(super) fn get(sim: &mut Simulator, prop: c_int, h: &VpiHandle) -> Option<c_int> {
    // A constant made on the fly.
    if h.kind == VpiKind::Obj && h.signal_id == usize::MAX && h.type_code == c::RANGE {
        let (l, r) = (h.lsb as i32 as i64, h.width as i32 as i64);
        return Some(match prop {
            c::TYPE => c::RANGE,
            c::SIZE => ((l - r).unsigned_abs() + 1) as c_int,
            _ => c::UNDEFINED,
        });
    }
    if h.kind == VpiKind::Obj && h.signal_id == usize::MAX {
        let v = h.value.as_ref();
        return Some(match prop {
            c::TYPE => h.type_code,
            c::SIZE => h.width as c_int,
            c::CONST_TYPE => h.lsb as c_int,
            c::SIGNED => i32::from(v.is_some_and(|v| v.is_signed)),
            c::LINE_NO => 0,
            _ => c::UNDEFINED,
        });
    }
    let owned = matches!(h.kind, VpiKind::Obj | VpiKind::Scope);
    let r = with_model(sim, |m, sim| {
        if matches!(h.kind, VpiKind::Slice | VpiKind::Signal | VpiKind::Memory)
            && ident(m, h).is_none()
        {
            return bit_get(m, sim, prop, h);
        }
        match ident(m, h)? {
            MRef::Scope(s) => Some(scope_get(m, prop, s, h)),
            MRef::Obj(o) => obj_get(m, sim, prop, o, h),
        }
    })
    .flatten();
    match r {
        Some(v) => Some(v),
        None if owned => Some(if prop == c::TYPE {
            h.type_code
        } else {
            c::UNDEFINED
        }),
        None => None,
    }
}

/// Bits and elements made by `select`: answered from their base object.
fn bit_get(m: &VpiModel, sim: &Simulator, prop: c_int, h: &VpiHandle) -> Option<c_int> {
    let (o, base) = match resolve_select(m, &h.full_name) {
        Some((o, b, _)) => (o, b),
        None => {
            // A port bit: named after its port.
            let (b, _) = split_select(&h.full_name)?;
            m.port_by_name.get(b)?;
            (NONE, b.to_string())
        }
    };
    let base = base.as_str();
    let (line, file_known) = if o == NONE {
        (0, false)
    } else {
        (m.objs[o as usize].line as c_int, true)
    };
    let _ = file_known;
    Some(match prop {
        c::LINE_NO => line,
        c::ARRAY_MEMBER => i32::from(h.kind != VpiKind::Slice),
        c::SIZE if h.kind == VpiKind::Slice => h.width as c_int,
        c::SIZE if h.kind == VpiKind::Memory => h.width as c_int,
        c::SIZE => sim.signal_widths.get(h.signal_id).copied().unwrap_or(0) as c_int,
        c::SCALAR if h.kind == VpiKind::Slice => 1,
        c::VECTOR if h.kind == VpiKind::Slice => 0,
        c::DIRECTION => h.direction,
        c::PORT_INDEX if h.type_code == c::PORT_BIT => match m.port_by_name.get(base) {
            Some(&PortIdx(p)) => match &m.objs[p as usize].d {
                OData::Port(pd) => pd.index as c_int,
                _ => c::UNDEFINED,
            },
            None => c::UNDEFINED,
        },
        c::ARRAY => i32::from(h.kind == VpiKind::Memory),
        _ => return None,
    })
}

fn scope_get(m: &VpiModel, prop: c_int, s: u32, h: &VpiHandle) -> c_int {
    let sc = &m.scopes[s as usize];
    let inst = matches!(sc.kind, SKind::Module | SKind::Interface | SKind::Program);
    match prop {
        c::TYPE => sc.type_code,
        c::LINE_NO => sc.line as c_int,
        c::DEF_LINE_NO if inst || sc.kind == SKind::Package => sc.def_line as c_int,
        c::TOP_MODULE if sc.kind == SKind::Module => i32::from(sc.top),
        c::TOP if inst || sc.kind == SKind::Package => {
            i32::from(sc.top || sc.kind == SKind::Package)
        }
        c::UNIT if sc.kind == SKind::Package => 0,
        c::CELL_INSTANCE if inst => i32::from(sc.cell),
        c::PROTECTED | c::IS_PROTECTED => 0,
        c::AUTOMATIC
            if matches!(
                sc.kind,
                SKind::Task
                    | SKind::Function
                    | SKind::NamedBegin
                    | SKind::NamedFork
                    | SKind::Package
            ) =>
        {
            i32::from(sc.automatic)
        }
        c::FUNC_TYPE if sc.kind == SKind::Function => sc.func_type,
        c::SIZE if sc.kind == SKind::Function && sc.ret_td != NONE => {
            m.td_width(&m.tdescs[sc.ret_td as usize]) as c_int
        }
        c::SIGNED if sc.kind == SKind::Function && sc.ret_td != NONE => {
            i32::from(m.tdescs[sc.ret_td as usize].signed)
        }
        c::VISIBILITY if matches!(sc.kind, SKind::Task | SKind::Function) => c::PUBLIC_VIS,
        c::ACCESS_TYPE if matches!(sc.kind, SKind::Task | SKind::Function) && sc.dpi.is_some() => {
            c::DPI_IMPORT_ACC
        }
        c::DPI_CONTEXT if sc.dpi.is_some() => i32::from(sc.dpi.is_some_and(|d| d.0)),
        c::DPI_PURE if sc.dpi.is_some() => i32::from(sc.dpi.is_some_and(|d| d.1)),
        c::JOIN_TYPE if sc.kind == SKind::NamedFork => sc.join,
        c::ARRAY_MEMBER if sc.kind == SKind::GenScope => i32::from(sc.gen_array != NONE),
        c::IMPLICIT_DECL if sc.kind == SKind::GenScope => i32::from(sc.implicit),
        _ => {
            let _ = h;
            c::UNDEFINED
        }
    }
}

fn obj_get(
    m: &mut VpiModel,
    sim: &mut Simulator,
    prop: c_int,
    o: u32,
    h: &VpiHandle,
) -> Option<c_int> {
    let (ty, line) = {
        let ob = &m.objs[o as usize];
        (ob.type_code, ob.line as c_int)
    };
    // The handle's own answer where it has one (Slice/Memory sizes, the
    // Signal width) comes from the flat-table code.
    let flat_size = || -> Option<c_int> { None };
    let _ = flat_size;
    Some(match prop {
        c::TYPE => ty,
        c::LINE_NO => line,
        c::PROTECTED | c::IS_PROTECTED => 0,
        _ => match &m.objs[o as usize].d {
            OData::Var(v) => {
                let t = m.tdescs[v.td as usize].clone();
                let array = !t.unpacked.is_empty();
                match prop {
                    c::SIZE => {
                        if array {
                            match t.unpacked.first() {
                                Some(UDim::Dynamic | UDim::Queue) => {
                                    sim.signals
                                        .get(&format!("{}.size", v.flat))
                                        .and_then(|x| x.to_u64())
                                        .unwrap_or(0) as c_int
                                }
                                Some(UDim::Assoc) => sim.signals.elem_keys(&v.flat).len() as c_int,
                                _ => fixed_count(&t).map(|n| n as c_int).unwrap_or(c::UNDEFINED),
                            }
                        } else if matches!(t.kind, TKind::Event) {
                            c::UNDEFINED
                        } else if matches!(t.kind, TKind::String) {
                            // Characters, not the storage's bits.
                            v.sig
                                .and_then(|id| sim.signal_table.get(id))
                                .map(|x| x.to_sv_string().len() as c_int)
                                .unwrap_or(0)
                        } else if h.kind == VpiKind::Slice {
                            h.width as c_int
                        } else if let Some(id) = v.sig {
                            sim.signal_widths.get(id).copied().unwrap_or(0) as c_int
                        } else {
                            m.td_width(&t) as c_int
                        }
                    }
                    c::SIGNED => i32::from(t.signed),
                    c::VECTOR | c::SCALAR => {
                        let unpacked_struct = matches!(&t.kind,
                            TKind::Struct(s) if !m.structs[*s as usize].packed);
                        if array
                            || unpacked_struct
                            || matches!(
                                t.kind,
                                TKind::Real
                                    | TKind::ShortReal
                                    | TKind::String
                                    | TKind::Chandle
                                    | TKind::Event
                                    | TKind::Class(_)
                                    | TKind::VirtualIf(_)
                            )
                        {
                            0
                        } else {
                            let w = match v.sig {
                                Some(id) if h.kind != VpiKind::Slice => {
                                    sim.signal_widths.get(id).copied().unwrap_or(1)
                                }
                                _ if h.kind == VpiKind::Slice => h.width,
                                _ => m.td_width(&t),
                            };
                            let vector = w > 1 || !t.packed.is_empty();
                            i32::from(if prop == c::VECTOR { vector } else { !vector })
                        }
                    }
                    c::ARRAY => i32::from(array),
                    c::ARRAY_TYPE if array => match t.unpacked[0] {
                        UDim::Range(..) => c::STATIC_ARRAY,
                        UDim::Dynamic => c::DYNAMIC_ARRAY,
                        UDim::Queue => c::QUEUE_ARRAY,
                        UDim::Assoc => c::ASSOC_ARRAY,
                    },
                    c::IS_MEMORY => i32::from(
                        array
                            && v.net_type == 0
                            && t.unpacked.len() == 1
                            && matches!(t.unpacked[0], UDim::Range(..))
                            && matches!(t.kind, TKind::Logic | TKind::Reg | TKind::Implicit),
                    ),
                    c::ARRAY_MEMBER => 0,
                    c::NET_TYPE | c::RESOLVED_NET_TYPE if v.net_type != 0 => v.net_type,
                    c::IMPLICIT_DECL => i32::from(v.implicit),
                    c::DIRECTION => v.dir,
                    c::AUTOMATIC => i32::from(v.automatic),
                    c::CONSTANT_VARIABLE => i32::from(v.is_const),
                    c::VISIBILITY => c::PUBLIC_VIS,
                    c::STRUCT_UNION_MEMBER => i32::from(m.objs[o as usize].parent != NONE),
                    c::PACKED => match &t.kind {
                        TKind::Struct(s) => i32::from(m.structs[*s as usize].packed),
                        _ => c::UNDEFINED,
                    },
                    c::TAGGED => match &t.kind {
                        TKind::Struct(s) => i32::from(m.structs[*s as usize].tagged),
                        _ => c::UNDEFINED,
                    },
                    _ => c::UNDEFINED,
                }
            }
            OData::Param {
                td,
                local,
                const_type,
                sig,
                ..
            } => {
                let t = m.tdescs[*td as usize].clone();
                match prop {
                    c::LOCAL_PARAM => i32::from(*local),
                    c::CONST_TYPE => *const_type,
                    c::SIZE => match sig {
                        Some(id) => sim.signal_widths.get(*id).copied().unwrap_or(0) as c_int,
                        None => m.td_width(&t) as c_int,
                    },
                    c::SIGNED => match sig {
                        Some(id) => {
                            i32::from(sim.signal_signed.get(*id).copied().unwrap_or(t.signed))
                        }
                        None => i32::from(t.signed),
                    },
                    c::VECTOR | c::SCALAR => {
                        let w = match sig {
                            Some(id) => sim.signal_widths.get(*id).copied().unwrap_or(1),
                            None => m.td_width(&t),
                        };
                        let vector = w > 1;
                        i32::from(if prop == c::VECTOR { vector } else { !vector })
                    }
                    c::ARRAY => i32::from(!t.unpacked.is_empty()),
                    _ => c::UNDEFINED,
                }
            }
            OData::Process { always_type, .. } => match prop {
                c::ALWAYS_TYPE if ty == c::ALWAYS => *always_type,
                _ => c::UNDEFINED,
            },
            OData::ContAssign { lhs, net_decl, .. } => match prop {
                c::NET_DECL_ASSIGN => i32::from(*net_decl),
                c::SIZE => {
                    let l = *lhs;
                    match eval_obj(m, sim, l) {
                        Some(v) => v.width as c_int,
                        None => c::UNDEFINED,
                    }
                }
                _ => c::UNDEFINED,
            },
            OData::Prim {
                prim_type, inputs, ..
            } => match prop {
                c::PRIM_TYPE => *prim_type,
                c::SIZE => *inputs as c_int,
                _ => c::UNDEFINED,
            },
            OData::PrimTerm { index, dir, .. } => match prop {
                c::TERM_INDEX => *index as c_int,
                c::DIRECTION => *dir,
                c::SIZE => {
                    let e = o;
                    eval_obj(m, sim, e)
                        .map(|v| v.width as c_int)
                        .unwrap_or(c::UNDEFINED)
                }
                _ => c::UNDEFINED,
            },
            OData::PathTerm { dir, .. } => match prop {
                c::DIRECTION => *dir,
                c::EDGE => 0,
                _ => c::UNDEFINED,
            },
            OData::Port(p) => {
                let (index, dir, port_type, cbn, low) =
                    (p.index, p.dir, p.port_type, p.conn_by_name, p.low);
                match prop {
                    c::PORT_INDEX => index as c_int,
                    c::DIRECTION => dir,
                    c::PORT_TYPE => port_type,
                    c::CONN_BY_NAME => i32::from(cbn),
                    c::EXPLICIT_NAME => 0,
                    c::SIZE | c::SCALAR | c::VECTOR | c::SIGNED => {
                        if low == NONE {
                            c::UNDEFINED
                        } else {
                            let lh = obj_handle(m, sim, low);
                            return obj_get(m, sim, prop, low, &lh);
                        }
                    }
                    _ => c::UNDEFINED,
                }
            }
            OData::IoDecl { dir, td } => {
                let t = m.tdescs[*td as usize].clone();
                let w = m.td_width(&t);
                match prop {
                    c::DIRECTION => *dir,
                    c::SIZE => w as c_int,
                    c::SIGNED => i32::from(t.signed),
                    c::VECTOR => i32::from(w > 1 || !t.packed.is_empty()),
                    c::SCALAR => i32::from(!(w > 1 || !t.packed.is_empty())),
                    c::ARRAY => i32::from(!t.unpacked.is_empty()),
                    _ => c::UNDEFINED,
                }
            }
            OData::GenArray { elems } => match prop {
                c::SIZE => elems.len() as c_int,
                _ => c::UNDEFINED,
            },
            OData::Expr(ed) => match prop {
                c::OP_TYPE if ty == c::OPERATION => op_type(&ed.expr),
                c::SIZE => eval_obj(m, sim, o)
                    .map(|v| v.width as c_int)
                    .unwrap_or(c::UNDEFINED),
                _ => c::UNDEFINED,
            },
            OData::ModPath { ifnone, .. } => match prop {
                c::MOD_PATH_HAS_IF_NONE => i32::from(*ifnone),
                _ => c::UNDEFINED,
            },
            OData::Tchk { tchk_type, .. } => match prop {
                c::TCHK_TYPE => *tchk_type,
                _ => c::UNDEFINED,
            },
            OData::TchkTerm { edge, .. } => match prop {
                c::EDGE => *edge,
                _ => c::UNDEFINED,
            },
            OData::Typespec { td } | OData::TypespecMember { td } => {
                let t = m.tdescs[*td as usize].clone();
                let is_ts = matches!(m.objs[o as usize].d, OData::Typespec { .. });
                match prop {
                    c::SIZE if is_ts => match fixed_count(&t) {
                        Some(n) if !t.unpacked.is_empty() => n as c_int,
                        _ if !t.unpacked.is_empty() => c::UNDEFINED,
                        _ => m.td_width(&t) as c_int,
                    },
                    c::SIGNED if is_ts => i32::from(t.signed),
                    c::VECTOR if is_ts => i32::from(m.td_width(&t) > 1 || !t.packed.is_empty()),
                    c::PACKED if is_ts => match &t.kind {
                        TKind::Struct(s) => i32::from(m.structs[*s as usize].packed),
                        _ => c::UNDEFINED,
                    },
                    c::ARRAY_TYPE if is_ts && !t.unpacked.is_empty() => match t.unpacked[0] {
                        UDim::Range(..) => c::STATIC_ARRAY,
                        UDim::Dynamic => c::DYNAMIC_ARRAY,
                        UDim::Queue => c::QUEUE_ARRAY,
                        UDim::Assoc => c::ASSOC_ARRAY,
                    },
                    _ => c::UNDEFINED,
                }
            }
            OData::EnumConst { width, .. } => match prop {
                c::SIZE => *width as c_int,
                _ => c::UNDEFINED,
            },
            OData::Modport { .. } => c::UNDEFINED,
            OData::ModportIo { dir, expr } => match prop {
                c::DIRECTION => *dir,
                c::SIZE => {
                    let e = *expr;
                    eval_obj(m, sim, e)
                        .map(|v| v.width as c_int)
                        .unwrap_or(c::UNDEFINED)
                }
                _ => c::UNDEFINED,
            },
            OData::Range { left, right } => match prop {
                c::SIZE => ((left - right).unsigned_abs() + 1) as c_int,
                _ => c::UNDEFINED,
            },
        },
    })
}

// ---------------------------------------------------------------------------
// vpi_get_str
// ---------------------------------------------------------------------------

/// String properties answered without the simulator: every type name, and
/// the (absent) location of a `$systf` call, constant or iterator.
pub(super) fn get_str_static(prop: c_int, h: &VpiHandle) -> Option<Option<String>> {
    let unmodelled = matches!(
        h.kind,
        VpiKind::SysTfCall | VpiKind::Constant | VpiKind::Iterator
    ) || (h.kind == VpiKind::Obj && h.signal_id == usize::MAX);
    if !unmodelled {
        return None;
    }
    match prop {
        c::TYPE => Some(type_name(h.type_code).map(str::to_string)),
        c::FILE | c::DEF_FILE | c::DEF_NAME => Some(None),
        _ => None,
    }
}

/// String properties. `Some(None)` answers NULL; `None` leaves the call to
/// the flat-table code.
pub(super) fn get_str(sim: &mut Simulator, prop: c_int, h: &VpiHandle) -> Option<Option<String>> {
    if let Some(r) = get_str_static(prop, h) {
        return Some(r);
    }
    if h.kind == VpiKind::Obj && h.signal_id == usize::MAX {
        return Some(None);
    }
    if prop == c::TYPE {
        // The declared type the model knows, else the handle's own.
        let code = with_model(sim, |m, _| match ident(m, h) {
            Some(MRef::Scope(s)) => Some(m.scopes[s as usize].type_code),
            Some(MRef::Obj(o)) => Some(m.objs[o as usize].type_code),
            None => None,
        })
        .flatten()
        .unwrap_or(h.type_code);
        return Some(type_name(code).map(str::to_string));
    }
    let owned = matches!(h.kind, VpiKind::Obj | VpiKind::Scope);
    let r = with_model(sim, |m, _| {
        let file_of = |m: &VpiModel, f: u32| (f != NONE).then(|| m.files[f as usize].clone());
        let r = match ident(m, h) {
            Some(r) => r,
            None => {
                // A bit or element: its base object's location.
                let (o, _, _) = resolve_select(m, &h.full_name)?;
                let r = MRef::Obj(o);
                return match prop {
                    c::FILE => Some(match r {
                        MRef::Obj(o) => file_of(m, m.objs[o as usize].file),
                        MRef::Scope(s) => file_of(m, m.scopes[s as usize].file),
                    }),
                    c::NAME => Some(Some(h.name.clone())),
                    c::FULL_NAME => Some(Some(h.full_name.clone())),
                    _ => None,
                };
            }
        };
        Some(match r {
            MRef::Scope(s) => {
                let sc = &m.scopes[s as usize];
                let inst = matches!(
                    sc.kind,
                    SKind::Module | SKind::Interface | SKind::Program | SKind::Package
                );
                match prop {
                    c::NAME => Some(sc.name.clone()),
                    c::FULL_NAME => Some(sc.full.clone()),
                    c::DEF_NAME if inst => Some(sc.def_name.clone()),
                    c::FILE => file_of(m, sc.file),
                    c::DEF_FILE if inst => file_of(m, sc.def_file),
                    _ => None,
                }
            }
            MRef::Obj(o) => {
                let ob = &m.objs[o as usize];
                let opt = |s: &String| (!s.is_empty()).then(|| s.clone());
                match prop {
                    c::NAME => opt(&ob.name),
                    c::FULL_NAME => opt(&ob.full),
                    c::FILE => file_of(m, ob.file),
                    c::DEF_NAME => match &ob.d {
                        OData::Prim { def_name, .. } => Some(def_name.clone()),
                        _ => None,
                    },
                    _ => None,
                }
            }
        })
    })
    .flatten();
    match r {
        Some(v) => Some(v),
        None if owned => Some(None),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// `vpi_get_value` for model objects. `Some(filled)` when the handle is one
/// of ours (or a named event); `None` for everything else.
pub(super) fn get_value(sim: &mut Simulator, h: &VpiHandle, vp: &mut s_vpi_value) -> Option<bool> {
    if h.type_code == c::NAMED_EVENT && h.kind != VpiKind::Obj {
        vpi_error(
            vpi::ERROR,
            format!("vpi_get_value: named event '{}' has no value", h.full_name),
        );
        vp.format = vpi::SUPPRESS_VAL;
        return Some(false);
    }
    if !matches!(h.kind, VpiKind::Obj | VpiKind::Scope) {
        return None;
    }
    let val = if h.signal_id == usize::MAX {
        h.value.clone()
    } else if h.kind == VpiKind::Obj {
        with_model(sim, |m, sim| {
            let o = h.signal_id as u32;
            if (o as usize) >= m.objs.len() {
                return None;
            }
            eval_obj(m, sim, o)
        })
        .flatten()
    } else {
        None
    };
    let ok = match &val {
        Some(v) => {
            let t = sim.time;
            fill_vpi_value(v, t, Some(h.type_code), vp)
        }
        None => false,
    };
    if !ok {
        vpi_error(
            vpi::ERROR,
            format!(
                "vpi_get_value: {} '{}' has no readable value",
                type_name(h.type_code).unwrap_or("object"),
                if h.full_name.is_empty() {
                    h.name.as_str()
                } else {
                    h.full_name.as_str()
                }
            ),
        );
        vp.format = vpi::SUPPRESS_VAL;
    }
    Some(ok)
}

/// `vpi_put_value` on an object that cannot be written: true after
/// reporting the error.
pub(super) fn put_value_rejected(h: &VpiHandle) -> bool {
    if matches!(h.kind, VpiKind::Obj | VpiKind::Scope) || h.type_code == c::NAMED_EVENT {
        vpi_error(
            vpi::ERROR,
            format!(
                "vpi_put_value: {} '{}' cannot be written",
                type_name(h.type_code).unwrap_or("object"),
                h.full_name
            ),
        );
        return true;
    }
    false
}

/// Hand a string back through the `vpi_get_str` pool (see
/// `VPI_STR2_SCRATCH`).
pub(super) fn str_result(s: &str) -> *mut libc::c_char {
    VPI_STR2_SCRATCH.with(|cell| {
        let mut pool = cell.borrow_mut();
        let slot = pool.0;
        pool.0 = (slot + 1) % pool.1.len();
        let buf = &mut pool.1[slot];
        buf.clear();
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
        buf.as_mut_ptr() as *mut libc::c_char
    })
}
