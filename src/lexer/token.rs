//! Token types for SystemVerilog (IEEE 1800-2017 §5, Annex B)

use crate::ast::Span;

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, text: String, span: Span) -> Self {
        Self { kind, text, span }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // Literals
    IntegerLiteral,
    RealLiteral,
    TimeLiteral,
    UnbasedUnsizedLiteral,
    StringLiteral,

    // Identifiers
    Identifier,
    EscapedIdentifier,
    SystemIdentifier,

    // Punctuation / Operators
    LParen, RParen, LBracket, RBracket, LBrace, RBrace,
    Semicolon, Colon, DoubleColon, Comma, Dot,
    Hash, HashHash, At, Dollar,
    Assign,         // =
    Question,       // ?
    Plus, Minus, Star, Slash, Percent,
    DoubleStar,     // **
    Increment, Decrement, // ++ --
    LogNot, LogAnd, LogOr, LogImplies, LogEquiv, // ! && || -> <->
    BitNot, BitAnd, BitOr, BitXor, BitNand, BitNor, BitXnor, // ~ & | ^ ~& ~| ~^
    Eq, Neq, CaseEq, CaseNeq, WildcardEq, WildcardNeq, // == != === !== ==? !=?
    Lt, Gt, Leq, Geq, // < > <= >=
    ShiftLeft, ShiftRight, ArithShiftLeft, ArithShiftRight, // << >> <<< >>>
    Arrow, DoubleArrow, FatArrow, // -> ->> =>
    PlusColon, MinusColon, // +: -:
    ColonSlash, // :/
    ApostropheLBrace, // '{

    // Assignment operators
    PlusAssign, MinusAssign, StarAssign, SlashAssign, PercentAssign,
    AndAssign, OrAssign, XorAssign,
    ShiftLeftAssign, ShiftRightAssign,
    ArithShiftLeftAssign, ArithShiftRightAssign,

    // Compiler directive
    Directive,

    // Keywords (IEEE 1800-2017 Annex B)
    KwAccept_on, KwAlias, KwAlways, KwAlways_comb, KwAlways_ff, KwAlways_latch,
    KwAnd, KwAssert, KwAssign, KwAssume, KwAutomatic,
    KwBefore, KwBegin, KwBind, KwBins, KwBinsof, KwBit, KwBreak, KwBuf, KwBufif0, KwBufif1, KwByte,
    KwCase, KwCasex, KwCasez, KwCell, KwChandle, KwChecker, KwClass, KwClocking,
    KwCmos, KwConfig, KwConst, KwConstraint, KwContext, KwContinue, KwCover,
    KwCovergroup, KwCoverpoint, KwCross,
    KwDeassign, KwDefault, KwDefparam, KwDesign, KwDisable, KwDist, KwDo,
    KwEdge, KwElse, KwEnd, KwEndcase, KwEndchecker, KwEndclass, KwEndclocking,
    KwEndconfig, KwEndfunction, KwEndgenerate, KwEndgroup, KwEndinterface,
    KwEndmodule, KwEndpackage, KwEndprimitive, KwEndprogram, KwEndproperty,
    KwEndspecify, KwEndsequence, KwEndtable, KwEndtask,
    KwEnum, KwEvent, KwEventually, KwExpect, KwExport, KwExtends, KwExtern,
    KwFinal, KwFirst_match, KwFor, KwForce, KwForeach, KwForever, KwFork,
    KwForkjoin, KwFunction,
    KwGenerate, KwGenvar, KwGlobal,
    KwHighz0, KwHighz1,
    KwIf, KwIff, KwIfnone, KwIgnore_bins, KwIllegal_bins, KwImplements, KwImplies,
    KwImport, KwIncdir, KwInclude, KwInitial, KwInout, KwInput, KwInside,
    KwInstance, KwInt, KwInteger, KwInterconnect, KwInterface, KwIntersect,
    KwJoin, KwJoin_any, KwJoin_none,
    KwLarge, KwLet, KwLiblist, KwLibrary, KwLocal, KwLocalparam, KwLogic, KwLongint,
    KwMacromodule, KwMatches, KwMedium, KwModport, KwModule,
    KwNand, KwNegedge, KwNettype, KwNew, KwNexttime, KwNmos, KwNor, KwNoshowcancelled, KwNot,
    KwNotif0, KwNotif1, KwNull,
    KwOr, KwOutput,
    KwPackage, KwPacked, KwParameter, KwPmos, KwPosedge, KwPrimitive, KwPriority,
    KwProgram, KwProperty, KwProtected, KwPull0, KwPull1, KwPulldown, KwPullup,
    KwPulsestyle_ondetect, KwPulsestyle_onevent, KwPure,
    KwRand, KwRandc, KwRandcase, KwRandsequence, KwRcmos,
    KwReal, KwRealtime, KwRef, KwReg, KwReject_on, KwRelease, KwRepeat,
    KwRestrict, KwReturn, KwRnmos, KwRpmos, KwRtran, KwRtranif0, KwRtranif1,
    KwS_always, KwS_eventually, KwS_nexttime, KwS_until, KwS_until_with,
    KwScalared, KwSequence, KwShortint, KwShortreal, KwShowcancelled, KwSigned,
    KwSmall, KwSoft, KwSolve, KwSpecify, KwSpecparam, KwStatic, KwString,
    KwStrong, KwStrong0, KwStrong1, KwStruct, KwSuper, KwSupply0, KwSupply1,
    KwSync_accept_on, KwSync_reject_on,
    KwTable, KwTagged, KwTask, KwThis, KwThroughout, KwTime, KwTimeprecision,
    KwTimeunit, KwTran, KwTranif0, KwTranif1, KwTri, KwTri0, KwTri1,
    KwTriand, KwTrior, KwTrireg, KwType, KwTypedef,
    KwUnion, KwUnique, KwUnique0, KwUnsigned, KwUntil, KwUntil_with, KwUntyped, KwUse, KwUwire,
    KwVar, KwVectored, KwVirtual, KwVoid,
    KwWait, KwWait_order, KwWand, KwWeak, KwWeak0, KwWeak1, KwWhile,
    KwWildcard, KwWire, KwWith, KwWithin, KwWor,
    KwXnor, KwXor,

    // Special
    Eof,
    Unknown,
}

/// Look up a keyword from an identifier string.
pub fn keyword(s: &str) -> Option<TokenKind> {
    use TokenKind::*;
    match s {
        "accept_on" => Some(KwAccept_on), "alias" => Some(KwAlias),
        "always" => Some(KwAlways), "always_comb" => Some(KwAlways_comb),
        "always_ff" => Some(KwAlways_ff), "always_latch" => Some(KwAlways_latch),
        "and" => Some(KwAnd), "assert" => Some(KwAssert), "assign" => Some(KwAssign),
        "assume" => Some(KwAssume), "automatic" => Some(KwAutomatic),
        "before" => Some(KwBefore), "begin" => Some(KwBegin), "bind" => Some(KwBind),
        "bins" => Some(KwBins), "binsof" => Some(KwBinsof), "bit" => Some(KwBit),
        "break" => Some(KwBreak), "buf" => Some(KwBuf), "bufif0" => Some(KwBufif0),
        "bufif1" => Some(KwBufif1), "byte" => Some(KwByte),
        "case" => Some(KwCase), "casex" => Some(KwCasex), "casez" => Some(KwCasez),
        "cell" => Some(KwCell), "chandle" => Some(KwChandle), "checker" => Some(KwChecker),
        "class" => Some(KwClass), "clocking" => Some(KwClocking), "cmos" => Some(KwCmos),
        "config" => Some(KwConfig), "const" => Some(KwConst), "constraint" => Some(KwConstraint),
        "context" => Some(KwContext), "continue" => Some(KwContinue), "cover" => Some(KwCover),
        "covergroup" => Some(KwCovergroup), "coverpoint" => Some(KwCoverpoint), "cross" => Some(KwCross),
        "deassign" => Some(KwDeassign), "default" => Some(KwDefault), "defparam" => Some(KwDefparam),
        "design" => Some(KwDesign), "disable" => Some(KwDisable), "dist" => Some(KwDist), "do" => Some(KwDo),
        "edge" => Some(KwEdge), "else" => Some(KwElse), "end" => Some(KwEnd),
        "endcase" => Some(KwEndcase), "endchecker" => Some(KwEndchecker), "endclass" => Some(KwEndclass),
        "endclocking" => Some(KwEndclocking), "endconfig" => Some(KwEndconfig),
        "endfunction" => Some(KwEndfunction), "endgenerate" => Some(KwEndgenerate),
        "endgroup" => Some(KwEndgroup), "endinterface" => Some(KwEndinterface),
        "endmodule" => Some(KwEndmodule), "endpackage" => Some(KwEndpackage),
        "endprimitive" => Some(KwEndprimitive), "endprogram" => Some(KwEndprogram),
        "endproperty" => Some(KwEndproperty), "endspecify" => Some(KwEndspecify),
        "endsequence" => Some(KwEndsequence), "endtable" => Some(KwEndtable), "endtask" => Some(KwEndtask),
        "enum" => Some(KwEnum), "event" => Some(KwEvent), "eventually" => Some(KwEventually),
        "expect" => Some(KwExpect), "export" => Some(KwExport), "extends" => Some(KwExtends), "extern" => Some(KwExtern),
        "final" => Some(KwFinal), "first_match" => Some(KwFirst_match), "for" => Some(KwFor),
        "force" => Some(KwForce), "foreach" => Some(KwForeach), "forever" => Some(KwForever),
        "fork" => Some(KwFork), "forkjoin" => Some(KwForkjoin), "function" => Some(KwFunction),
        "generate" => Some(KwGenerate), "genvar" => Some(KwGenvar), "global" => Some(KwGlobal),
        "highz0" => Some(KwHighz0), "highz1" => Some(KwHighz1),
        "if" => Some(KwIf), "iff" => Some(KwIff), "ifnone" => Some(KwIfnone),
        "ignore_bins" => Some(KwIgnore_bins), "illegal_bins" => Some(KwIllegal_bins),
        "implements" => Some(KwImplements), "implies" => Some(KwImplies), "import" => Some(KwImport),
        "incdir" => Some(KwIncdir), "include" => Some(KwInclude), "initial" => Some(KwInitial),
        "inout" => Some(KwInout), "input" => Some(KwInput), "inside" => Some(KwInside),
        "instance" => Some(KwInstance), "int" => Some(KwInt), "integer" => Some(KwInteger),
        "interconnect" => Some(KwInterconnect), "interface" => Some(KwInterface), "intersect" => Some(KwIntersect),
        "join" => Some(KwJoin), "join_any" => Some(KwJoin_any), "join_none" => Some(KwJoin_none),
        "large" => Some(KwLarge), "let" => Some(KwLet), "liblist" => Some(KwLiblist),
        "library" => Some(KwLibrary), "local" => Some(KwLocal), "localparam" => Some(KwLocalparam),
        "logic" => Some(KwLogic), "longint" => Some(KwLongint),
        "macromodule" => Some(KwMacromodule), "matches" => Some(KwMatches), "medium" => Some(KwMedium),
        "modport" => Some(KwModport), "module" => Some(KwModule),
        "nand" => Some(KwNand), "negedge" => Some(KwNegedge), "nettype" => Some(KwNettype),
        "new" => Some(KwNew), "nexttime" => Some(KwNexttime), "nmos" => Some(KwNmos),
        "nor" => Some(KwNor), "noshowcancelled" => Some(KwNoshowcancelled), "not" => Some(KwNot),
        "notif0" => Some(KwNotif0), "notif1" => Some(KwNotif1), "null" => Some(KwNull),
        "or" => Some(KwOr), "output" => Some(KwOutput),
        "package" => Some(KwPackage), "packed" => Some(KwPacked), "parameter" => Some(KwParameter),
        "pmos" => Some(KwPmos), "posedge" => Some(KwPosedge), "primitive" => Some(KwPrimitive),
        "priority" => Some(KwPriority), "program" => Some(KwProgram), "property" => Some(KwProperty),
        "protected" => Some(KwProtected), "pull0" => Some(KwPull0), "pull1" => Some(KwPull1),
        "pulldown" => Some(KwPulldown), "pullup" => Some(KwPullup),
        "pulsestyle_ondetect" => Some(KwPulsestyle_ondetect), "pulsestyle_onevent" => Some(KwPulsestyle_onevent),
        "pure" => Some(KwPure),
        "rand" => Some(KwRand), "randc" => Some(KwRandc), "randcase" => Some(KwRandcase),
        "randsequence" => Some(KwRandsequence), "rcmos" => Some(KwRcmos),
        "real" => Some(KwReal), "realtime" => Some(KwRealtime), "ref" => Some(KwRef),
        "reg" => Some(KwReg), "reject_on" => Some(KwReject_on), "release" => Some(KwRelease),
        "repeat" => Some(KwRepeat), "restrict" => Some(KwRestrict), "return" => Some(KwReturn),
        "rnmos" => Some(KwRnmos), "rpmos" => Some(KwRpmos), "rtran" => Some(KwRtran),
        "rtranif0" => Some(KwRtranif0), "rtranif1" => Some(KwRtranif1),
        "s_always" => Some(KwS_always), "s_eventually" => Some(KwS_eventually),
        "s_nexttime" => Some(KwS_nexttime), "s_until" => Some(KwS_until), "s_until_with" => Some(KwS_until_with),
        "scalared" => Some(KwScalared), "sequence" => Some(KwSequence), "shortint" => Some(KwShortint),
        "shortreal" => Some(KwShortreal), "showcancelled" => Some(KwShowcancelled), "signed" => Some(KwSigned),
        "small" => Some(KwSmall), "soft" => Some(KwSoft), "solve" => Some(KwSolve),
        "specify" => Some(KwSpecify), "specparam" => Some(KwSpecparam), "static" => Some(KwStatic),
        "string" => Some(KwString), "strong" => Some(KwStrong), "strong0" => Some(KwStrong0),
        "strong1" => Some(KwStrong1), "struct" => Some(KwStruct), "super" => Some(KwSuper),
        "supply0" => Some(KwSupply0), "supply1" => Some(KwSupply1),
        "sync_accept_on" => Some(KwSync_accept_on), "sync_reject_on" => Some(KwSync_reject_on),
        "table" => Some(KwTable), "tagged" => Some(KwTagged), "task" => Some(KwTask),
        "this" => Some(KwThis), "throughout" => Some(KwThroughout), "time" => Some(KwTime),
        "timeprecision" => Some(KwTimeprecision), "timeunit" => Some(KwTimeunit),
        "tran" => Some(KwTran), "tranif0" => Some(KwTranif0), "tranif1" => Some(KwTranif1),
        "tri" => Some(KwTri), "tri0" => Some(KwTri0), "tri1" => Some(KwTri1),
        "triand" => Some(KwTriand), "trior" => Some(KwTrior), "trireg" => Some(KwTrireg),
        "type" => Some(KwType), "typedef" => Some(KwTypedef),
        "union" => Some(KwUnion), "unique" => Some(KwUnique), "unique0" => Some(KwUnique0),
        "unsigned" => Some(KwUnsigned), "until" => Some(KwUntil), "until_with" => Some(KwUntil_with),
        "untyped" => Some(KwUntyped), "use" => Some(KwUse), "uwire" => Some(KwUwire),
        "var" => Some(KwVar), "vectored" => Some(KwVectored), "virtual" => Some(KwVirtual), "void" => Some(KwVoid),
        "wait" => Some(KwWait), "wait_order" => Some(KwWait_order), "wand" => Some(KwWand),
        "weak" => Some(KwWeak), "weak0" => Some(KwWeak0), "weak1" => Some(KwWeak1), "while" => Some(KwWhile),
        "wildcard" => Some(KwWildcard), "wire" => Some(KwWire), "with" => Some(KwWith),
        "within" => Some(KwWithin), "wor" => Some(KwWor),
        "xnor" => Some(KwXnor), "xor" => Some(KwXor),
        _ => None,
    }
}
