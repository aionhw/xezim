#ifndef VPI_USER_H
#define VPI_USER_H

/* VPI (Verilog Procedural Interface) — IEEE 1800-2017 Annex K.
 *
 * Every constant below is the value the standard assigns it, so a C
 * file compiled against a vendor `vpi_user.h` and linked to xezim
 * agrees with xezim about what `vpiIntVal` or `cbValueChange` means.
 * An earlier version of this header invented its own numbering, which
 * meant any such file silently took the wrong branch.
 *
 * xezim implements the subset declared at the bottom of this file.
 * Functions the standard defines but xezim does not implement are NOT
 * declared here: a call to one is a compile error, which is the loud
 * failure we want, rather than a link-time surprise or a stub that
 * silently returns nothing. Every routine of IEEE 1800-2017 clause 38 is
 * declared; vpi_get_data / vpi_put_data are defined but always fail,
 * because xezim has no $save / $restart (see their declarations). The
 * SystemVerilog thread, frame and class-object callback reasons
 * (cbStartOfThread..cbEndOfObject) are rejected by vpi_register_cb.
 *
 * A VPI module is loaded with `--vpi-lib <so>` (or `-m`), after which its
 * `vlog_startup_routines` run once, before simulation. VPI is also callable
 * from a DPI shared object loaded with `--dpi-lib`.
 */

#include <stdint.h>
#include <stddef.h>
#include <stdarg.h>

/* C linkage for a C++ client, as in the standard header (Annex K). */
#ifdef __cplusplus
extern "C" {
#endif

/* PLI type definitions (IEEE 1800-2017 Annex K.1). */
typedef int32_t  PLI_INT32;
typedef uint32_t PLI_UINT32;
typedef int64_t  PLI_INT64;
typedef uint64_t PLI_UINT64;
typedef char     PLI_BYTE8;
typedef short    PLI_INT16;
typedef unsigned short PLI_UINT16;

/* vpiHandle — opaque handle to a simulation object. */
typedef PLI_UINT32 *vpiHandle;

/* --- vpi_get(vpiType, ...) object types (Annex K, "Object types") ------
 * Only the codes xezim can actually return are listed. `vpiLogicVar` is
 * an alias of `vpiReg`, exactly as in the standard header. */
#define vpiIntegerVar         25   /* integer variable */
#define vpiIterator           27   /* iterator (vpi_iterate result) */
#define vpiMemory             29   /* unpacked array */
#define vpiMemoryWord         30   /* one word of an unpacked array */
#define vpiModule             32
#define vpiNet                36   /* scalar or vector net */
#define vpiNetBit             37
#define vpiParameter          41
#define vpiPartSelect         42   /* part-select / packed-struct member */
#define vpiPort               44   /* module port */
#define vpiPortBit            45   /* bit of vector module port */
#define vpiRealVar            47   /* real variable */
#define vpiReg                48   /* scalar or vector reg (4-state) */
#define vpiRegBit             49
#define vpiTimeVar            63

#define vpiConstant            7   /* a literal / computed argument value */
#define vpiSysFuncCall        56
#define vpiSysTaskCall        57
#define vpiCallback          107   /* a vpi_register_cb handle */

/* Traversal relations. */
#define vpiScope              84   /* containing scope */
#define vpiSysTfCall          85   /* the $systf call now executing */
#define vpiArgument           89   /* argument of a $systf call */
#define vpiInternalScope      92   /* internal scopes of a module */
#define vpiVariables         100   /* variables declared in a module */

/* vpi_get(vpiFuncType, sysTfCallHandle) -> the sysfunctype it was registered with. */
#define vpiFuncType           44
#define vpiSysFuncType        vpiFuncType
/* SystemVerilog object types (IEEE 1800-2017 sv_vpi_user.h). */
#define vpiLongIntVar        610
#define vpiShortIntVar       611
#define vpiIntVar            612
#define vpiShortRealVar      613
#define vpiByteVar           614
#define vpiStringVar         616
#define vpiEnumVar           617
#define vpiStructVar         618
#define vpiUnionVar          619
#define vpiBitVar            620   /* 2-state bit variable */
#define vpiLogicVar         vpiReg /* 4-state logic variable */

/* --- vpi_get_value / vpi_put_value format codes (Table 38-44) --------- */
#define vpiBinStrVal           1
#define vpiOctStrVal           2
#define vpiDecStrVal           3
#define vpiHexStrVal           4
#define vpiScalarVal           5
#define vpiIntVal              6
#define vpiRealVal             7
#define vpiStringVal           8
#define vpiVectorVal           9
#define vpiStrengthVal        10
#define vpiTimeVal            11
#define vpiObjTypeVal         12
#define vpiSuppressVal        13
#define vpiShortIntVal        14
#define vpiLongIntVal         15
#define vpiShortRealVal       16
#define vpiRawTwoStateVal     17
#define vpiRawFourStateVal    18

/* --- vpiScalarVal codes ----------------------------------------------- */
#define vpi0                   0
#define vpi1                   1
#define vpiZ                   2
#define vpiX                   3
#define vpiH                   4
#define vpiL                   5
#define vpiDontCare            6

/* --- vpi_put_value flags ---------------------------------------------- */
#define vpiNoDelay             1
#define vpiInertialDelay       2
#define vpiTransportDelay      3
#define vpiPureTransportDelay  4
#define vpiForceFlag           5
#define vpiReleaseFlag         6

/* --- vpi_get properties ----------------------------------------------- */
#define vpiUndefined         (-1)
#define vpiType                1
#define vpiName                2
#define vpiFullName            3
#define vpiSize                4
#define vpiDefName             9   /* module definition name */
#define vpiTimeUnit           11   /* module time unit; NULL: the simulation's */
#define vpiTimePrecision      12   /* module time precision; NULL: the simulation's */
#define vpiScalar             17
#define vpiVector             18
#define vpiDirection          20
#define vpiSigned             65

/* --- vpiDirection values --------------------------------------------- */
#define vpiInput               1
#define vpiOutput              2
#define vpiInout               3
#define vpiMixedIO             4
#define vpiNoDirection         5

/* --- Object model (IEEE 1800-2017 chapter 37) --------------------------
 * The elaborated design as VPI objects: scopes, declared objects, processes,
 * continuous assignments, primitives, specify paths and timing checks, and
 * their relations. docs/dpi-guide.md ("VPI object model") lists what each
 * object answers. The SystemVerilog objects (packages, interfaces, programs,
 * typespecs, ...) are in sv_vpi_user.h. */

/* Object types. */
#define vpiAlways              1   /* always procedure (see vpiAlwaysType) */
#define vpiContAssign          8   /* continuous assignment */
#define vpiFunction           20
#define vpiGate               21   /* gate primitive instance */
#define vpiInitial            24
#define vpiIODecl             28   /* task/function argument */
#define vpiModPath            31   /* specify path */
#define vpiNamedBegin         33
#define vpiNamedEvent         34   /* exists and is named; has no value */
#define vpiNamedFork          35
#define vpiOperation          39   /* an expression other than a name, literal or select */
#define vpiPathTerm           43   /* terminal of a vpiModPath */
#define vpiPrimTerm           46   /* terminal of a primitive */
#define vpiSwitch             55   /* switch primitive instance */
#define vpiTask               59
#define vpiTchk               61   /* timing check */
#define vpiTchkTerm           62   /* reference or data terminal of a vpiTchk */
#define vpiUdp                65   /* user-defined primitive instance */
#define vpiBitSelect         106
#define vpiNetArray          114
#define vpiRange             115
#define vpiRegArray          116   /* array of variables; vpiArrayVar */
#define vpiNamedEventArray   129
#define vpiGenScopeArray     133
#define vpiGenScope          134

/* One-to-one relations (vpi_handle). */
#define vpiCondition          71
#define vpiDelay              72   /* the delay expression, where one is written */
#define vpiHighConn           76
#define vpiLhs                77
#define vpiIndex              78
#define vpiLeftRange          79
#define vpiLowConn            80
#define vpiParent             81
#define vpiRhs                82
#define vpiRightRange         83
#define vpiTchkDataTerm       86
#define vpiTchkNotifier       87
#define vpiTchkRefTerm        88
#define vpiExpr              102
#define vpiStmt              104   /* a process's statement, when it is a named block */

/* One-to-many relations (vpi_iterate). */
#define vpiBit                90   /* bits of a vector port, net or variable */
#define vpiModPathIn          95
#define vpiModPathOut         96
#define vpiOperand            97
#define vpiPortInst           98   /* ports a net/variable connects to from above */
#define vpiProcess            99
#define vpiPrimitive         103
#define vpiPorts             125   /* ports a net/variable is the low connection of */
#define vpiTaskFunc          127

/* Properties. vpiFile and vpiDefFile are vpi_get_str properties; an object
 * whose source location is unknown reports vpiLineNo 0 and vpiFile NULL. */
#define vpiFile                5
#define vpiLineNo              6
#define vpiTopModule           7
#define vpiCellInstance        8
#define vpiProtected          10
#define vpiDefFile            15
#define vpiDefLineNo          16
#define vpiExplicitName       19
#define vpiConnByName         21
#define vpiNetType            22
#define vpiImplicitDecl       26
#define vpiArray              28
#define vpiPortIndex          29
#define vpiTermIndex          30
#define vpiPrimType           33
#define vpiEdge               36
#define vpiTchkType           38
#define vpiOpType             39
#define vpiConstType          40
#define vpiNetDeclAssign      43
#define vpiAutomatic          50
#define vpiResolvedNetType    61
#define vpiLocalParam         70
#define vpiModPathHasIfNone   71
#define vpiIsMemory           73
#define vpiIsProtected        74

/* vpiNetType / vpiResolvedNetType values. */
#define vpiWire                1
#define vpiWand                2
#define vpiWor                 3
#define vpiTri                 4
#define vpiTri0                5
#define vpiTri1                6
#define vpiTriReg              7
#define vpiTriAnd              8
#define vpiTriOr               9
#define vpiSupply1            10
#define vpiSupply0            11
#define vpiNone               12   /* interconnect and wreal nets */
#define vpiUwire              13

/* vpiPrimType values. */
#define vpiAndPrim             1
#define vpiNandPrim            2
#define vpiNorPrim             3
#define vpiOrPrim              4
#define vpiXorPrim             5
#define vpiXnorPrim            6
#define vpiBufPrim             7
#define vpiNotPrim             8
#define vpiBufif0Prim          9
#define vpiBufif1Prim         10
#define vpiNotif0Prim         11
#define vpiNotif1Prim         12
#define vpiNmosPrim           13
#define vpiPmosPrim           14
#define vpiCmosPrim           15
#define vpiRnmosPrim          16
#define vpiRpmosPrim          17
#define vpiRcmosPrim          18
#define vpiRtranPrim          19
#define vpiRtranif0Prim       20
#define vpiRtranif1Prim       21
#define vpiTranPrim           22
#define vpiTranif0Prim        23
#define vpiTranif1Prim        24
#define vpiPullupPrim         25
#define vpiPulldownPrim       26
#define vpiSeqPrim            27   /* sequential UDP */
#define vpiCombPrim           28   /* combinational UDP */

/* vpiEdge values (timing check terminals). */
#define vpiNoEdge           0x00
#define vpiEdge01           0x01
#define vpiEdge10           0x02
#define vpiEdge0x           0x04
#define vpiEdgex1           0x08
#define vpiEdge1x           0x10
#define vpiEdgex0           0x20
#define vpiPosedge          (vpiEdgex1 | vpiEdge01 | vpiEdge0x)
#define vpiNegedge          (vpiEdgex0 | vpiEdge10 | vpiEdge1x)
#define vpiAnyEdge          (vpiPosedge | vpiNegedge)

/* vpiTchkType values. */
#define vpiSetup               1
#define vpiHold                2
#define vpiPeriod              3
#define vpiWidth               4
#define vpiSkew                5
#define vpiRecovery            6
#define vpiNoChange            7
#define vpiSetupHold           8
#define vpiFullskew            9
#define vpiRecrem             10
#define vpiRemoval            11
#define vpiTimeskew           12

/* vpiOpType values. */
#define vpiMinusOp             1
#define vpiPlusOp              2
#define vpiNotOp               3
#define vpiBitNegOp            4
#define vpiUnaryAndOp          5
#define vpiUnaryNandOp         6
#define vpiUnaryOrOp           7
#define vpiUnaryNorOp          8
#define vpiUnaryXorOp          9
#define vpiUnaryXNorOp        10
#define vpiSubOp              11
#define vpiDivOp              12
#define vpiModOp              13
#define vpiEqOp               14
#define vpiNeqOp              15
#define vpiCaseEqOp           16
#define vpiCaseNeqOp          17
#define vpiGtOp               18
#define vpiGeOp               19
#define vpiLtOp               20
#define vpiLeOp               21
#define vpiLShiftOp           22
#define vpiRShiftOp           23
#define vpiAddOp              24
#define vpiMultOp             25
#define vpiLogAndOp           26
#define vpiLogOrOp            27
#define vpiBitAndOp           28
#define vpiBitOrOp            29
#define vpiBitXorOp           30
#define vpiBitXNorOp          31
#define vpiBitXnorOp          vpiBitXNorOp
#define vpiConditionOp        32
#define vpiConcatOp           33
#define vpiMultiConcatOp      34
#define vpiArithLShiftOp      41
#define vpiArithRShiftOp      42
#define vpiPowerOp            43

/* vpiConstType values. */
#define vpiDecConst            1
#define vpiRealConst           2
#define vpiBinaryConst         3
#define vpiOctConst            4
#define vpiHexConst            5
#define vpiStringConst         6
#define vpiIntConst            7
#define vpiTimeConst           8

/* --- vpi_time types --------------------------------------------------- */
#define vpiScaledRealTime      1
#define vpiSimTime             2
#define vpiSuppressTime        3

/* --- vpi_control operations ------------------------------------------- */
#define vpiStop               66   /* ends the run, like $stop */
#define vpiFinish             67   /* ends the run, like $finish */
#define vpiReset              68   /* NOT supported: xezim cannot rewind */
#define vpiSetInteractiveScope 69  /* accepts a module handle */

/* --- vpi_chk_error severity levels and states ------------------------- */
#define vpiNotice              1
#define vpiWarning             2
#define vpiError               3
#define vpiSystem              4
#define vpiInternal            5
#define vpiCompile             1
#define vpiPLI                 2
#define vpiRun                 3

/* --- callback reasons (IEEE 1800-2017 section 38.36) ------------------ */
/* Simulation events */
#define cbValueChange          1
#define cbStmt                 2
#define cbForce                3
#define cbRelease              4
/* Simulation time */
#define cbAtStartOfSimTime     5
#define cbReadWriteSynch       6
#define cbReadOnlySynch        7
#define cbNextSimTime          8
#define cbAfterDelay           9
/* Actions */
#define cbEndOfCompile        10
#define cbStartOfSimulation   11
#define cbEndOfSimulation     12
#define cbError               13
#define cbTchkViolation       14
#define cbStartOfSave         15   /* accepted, never fires: no save/restart */
#define cbEndOfSave           16   /* accepted, never fires */
#define cbStartOfRestart      17   /* accepted, never fires */
#define cbEndOfRestart        18   /* accepted, never fires */
#define cbStartOfReset        19   /* accepted, never fires: no $reset */
#define cbEndOfReset          20   /* accepted, never fires */
#define cbEnterInteractive    21   /* $stop / vpi_control(vpiStop) */
#define cbExitInteractive     22   /* accepted, never fires */
#define cbInteractiveScopeChange 23 /* vpi_control(vpiSetInteractiveScope) */
#define cbUnresolvedSystf     24
#define cbAssign              25
#define cbDeassign            26
#define cbDisable             27
#define cbPLIError            28
#define cbSignal              29
#define cbNBASynch            30
#define cbAtEndOfSimTime      31

/* s_vpi_vecval — 4-state vector element (IEEE 1800-2017 §38.10.1).
 * Layout-compatible with svLogicVecVal (§35.5.5), so UVM's HDL backdoor
 * can assign between the two without translation. Both words are
 * PLI_UINT32, as in the standard's vpi_user.h (Annex K).
 *
 * Bit encoding, per element bit i:
 *     aval bval   value
 *       0    0      0
 *       1    0      1
 *       0    1      Z
 *       1    1      X
 */
typedef struct t_vpi_vecval {
    PLI_UINT32 aval;
    PLI_UINT32 bval;
} s_vpi_vecval, *p_vpi_vecval;

/* s_vpi_time — time value. */
typedef struct t_vpi_time {
    PLI_INT32 type;    /* vpiSimTime / vpiScaledRealTime / vpiSuppressTime */
    PLI_UINT32 high;
    PLI_UINT32 low;
    double real;
} s_vpi_time, *p_vpi_time;

/* s_vpi_strengthval — one bit of a vpiStrengthVal value (§38.15). */
typedef struct t_vpi_strengthval {
    PLI_INT32 logic;   /* vpi0 / vpi1 / vpiX / vpiZ */
    PLI_INT32 s0, s1;  /* strength codes below */
} s_vpi_strengthval, *p_vpi_strengthval;

/* Strength codes. */
#define vpiSupplyDrive      0x80
#define vpiStrongDrive      0x40
#define vpiPullDrive        0x20
#define vpiWeakDrive        0x08
#define vpiLargeCharge      0x10
#define vpiMediumCharge     0x04
#define vpiSmallCharge      0x02
#define vpiHiZ              0x01

/* s_vpi_value — value in one of the formats above.
 *
 * Formats and the union member that carries them:
 *   vpi*StrVal, vpiStringVal       str
 *   vpiScalarVal                   scalar
 *   vpiIntVal                      integer
 *   vpiRealVal                     real
 *   vpiTimeVal                     time
 *   vpiVectorVal                   vector
 *   vpiStrengthVal                 strength: one s_vpi_strengthval per bit,
 *                                  LSB first. A variable reads strong; a net
 *                                  reads the drive strength of the continuous
 *                                  assignment driving it (what %v shows), strong
 *                                  when none was given; z reads vpiHiZ. Put only
 *                                  to a scalar: the logic value is written, and
 *                                  the strengths are checked but not stored.
 * The s_vpi_value union has no member for the formats below, so xezim uses
 * the standard's implementation-specific `misc` field where one is needed:
 *   vpiShortIntVal                 integer (sign-extended 16-bit value)
 *   vpiShortRealVal                real (value rounded to single precision)
 *   vpiLongIntVal                  misc -> one PLI_INT64
 *   vpiRawTwoStateVal              misc -> ceil(size/8) aval bytes
 *   vpiRawFourStateVal             misc -> ceil(size/8) aval bytes, then as
 *                                  many bval bytes (the vpi_get_value_array
 *                                  layout of one element)
 */
typedef struct t_vpi_value {
    PLI_INT32 format;
    union {
        PLI_BYTE8                *str;
        PLI_INT32                 scalar;
        PLI_INT32                 integer;
        double                    real;
        struct t_vpi_time        *time;
        struct t_vpi_vecval      *vector;
        struct t_vpi_strengthval *strength;
        PLI_BYTE8                *misc;
    } value;
} s_vpi_value, *p_vpi_value;

/* s_vpi_vlog_info — tool identification, filled by vpi_get_vlog_info. */
typedef struct t_vpi_vlog_info {
    PLI_INT32   argc;
    PLI_BYTE8 **argv;
    PLI_BYTE8  *product;
    PLI_BYTE8  *version;
} s_vpi_vlog_info, *p_vpi_vlog_info;

/* s_cb_data — callback registration and dispatch (IEEE 1800-2017 §38.7). */
typedef struct t_cb_data s_cb_data, *p_cb_data;
struct t_cb_data {
    PLI_INT32    reason;
    PLI_INT32  (*cb_rtn)(p_cb_data cb_data_p);
    vpiHandle    obj;
    p_vpi_time   time;
    p_vpi_value  value;
    PLI_INT32    index;
    PLI_BYTE8   *user_data;
};

/* ---------------------------------------------------------------------
 * Implemented by xezim. Signatures match IEEE 1800-2017 Annex K exactly.
 * ------------------------------------------------------------------ */

/* Resolve a hierarchical name: any object of the object model (instance,
 * package, generate scope, named block, task, function, variable, net,
 * parameter, named event, named primitive), an array element (`mem[1]`) or a
 * bit of a vector (`w[3]`). With a scope handle the name is first taken
 * relative to that scope. A package member is `pkg::name`. As a fallback,
 * each successively shorter suffix of the name is tried against the signal
 * table, so "top.dut.sig", "dut.sig" and "sig" all resolve. */
vpiHandle vpi_handle_by_name(PLI_BYTE8 *name, vpiHandle scope);

/* One-to-one traversal: vpiScope, vpiParent, vpiModule, vpiInstance, and the
 * object-model relations above (docs/dpi-guide.md, "VPI object model"). As an
 * xezim extension, vpi_handle(vpiScope, NULL) returns the top module — the
 * standard route is vpi_scan(vpi_iterate(vpiModule, NULL)), but enough code
 * spells it the short way that supporting it is worth more than returning
 * NULL. A relation the object does not have returns NULL. */
vpiHandle vpi_handle(PLI_INT32 type, vpiHandle refHandle);

/* One-to-many traversal. Returns NULL when the relation yields nothing.
 * From a NULL reference: vpiModule (top modules), vpiInstance (top-level
 * instances, then packages), vpiInterface, vpiProgram, vpiPackage. From a
 * scope and from the other objects: see docs/dpi-guide.md, "VPI object
 * model". Iteration is in declaration order. */
vpiHandle vpi_iterate(PLI_INT32 type, vpiHandle refHandle);

/* Hand out the next object. When the iterator is exhausted it returns NULL
 * and FREES the iterator (IEEE 1800-2017 section 38.32) — do not free it
 * yourself. */
vpiHandle vpi_scan(vpiHandle iterator);

/* Select one word of a vpiMemory object. NULL if out of range. */
vpiHandle vpi_handle_by_index(vpiHandle object, PLI_INT32 index);

/* vpiName, vpiFullName, vpiType (the type's name), vpiFile, and vpiDefName /
 * vpiDefFile (instances, packages and primitives). Returns NULL when the
 * object has no such string (an unnamed process has no vpiName). The string
 * is simulator-owned and stays valid across the next few vpi_get_str calls
 * on this thread (a small rotating pool). */
PLI_BYTE8 *vpi_get_str(PLI_INT32 property, vpiHandle object);

/* Formatted output, interleaved with $display. */
int vpi_printf(PLI_BYTE8 *format, ...);
int vpi_vprintf(PLI_BYTE8 *format, va_list ap);
int vpi_mcd_printf(PLI_UINT32 mcd, PLI_BYTE8 *format, ...);
int vpi_mcd_vprintf(PLI_UINT32 mcd, PLI_BYTE8 *format, va_list ap);

/* Multichannel descriptors, shared with $fopen: bit 0 is stdout, and each
 * vpi_mcd_open (or one-argument $fopen) takes a free bit for a file opened
 * for writing. vpi_mcd_open returns 0 when the file cannot be opened;
 * vpi_mcd_close and vpi_mcd_flush return 0; vpi_mcd_name returns the file
 * name ("stdout" for bit 0), or NULL for a channel that is not open. */
PLI_UINT32 vpi_mcd_open(PLI_BYTE8 *name);
PLI_UINT32 vpi_mcd_close(PLI_UINT32 mcd);
PLI_INT32 vpi_mcd_flush(PLI_UINT32 mcd);
PLI_BYTE8 *vpi_mcd_name(PLI_UINT32 cd);

/* Flushes stdout and every open file. Returns 0. */
PLI_INT32 vpi_flush(void);

/* Register a system task or function. `tfname` must begin with '$', and
 * `type` must be vpiSysTask or vpiSysFunc. `compiletf` runs once per call
 * instance (a call site in one module instance), immediately before that
 * instance's first `calltf` — xezim has no separate compile phase for it.
 *
 * A vpiSysFunc is dispatched when its `$name` appears in an expression. It
 * returns whatever it deposits with vpi_put_value on its own call handle
 * (vpi_handle(vpiSysTfCall, NULL)); a function that deposits nothing returns
 * 0. `sizetf` is called once per invocation for vpiSizedFunc/vpiSizedSignedFunc
 * to learn the return width; the other sysfunctypes size themselves. */
typedef struct t_vpi_systf_data {
    PLI_INT32   type;         /* vpiSysTask or vpiSysFunc */
    PLI_INT32   sysfunctype;  /* vpi[Int,Real,Time,Sized,SizedSigned]Func */
    PLI_BYTE8  *tfname;       /* first character must be '$' */
    PLI_INT32 (*calltf)(PLI_BYTE8 *);
    PLI_INT32 (*compiletf)(PLI_BYTE8 *);
    PLI_INT32 (*sizetf)(PLI_BYTE8 *);
    PLI_BYTE8  *user_data;
} s_vpi_systf_data, *p_vpi_systf_data;

#define vpiSysTask             1
#define vpiSysFunc             2
#define vpiIntFunc             1
#define vpiRealFunc            2
#define vpiTimeFunc            3
#define vpiSizedFunc           4
#define vpiSizedSignedFunc     5

/* Returns the registration's vpiUserSystf object (NULL on failure). */
vpiHandle vpi_register_systf(p_vpi_systf_data systf_data_p);

/* Fills *systf_data_p from a vpiUserSystf handle: the one vpi_register_systf
 * returned, vpi_handle(vpiUserSystf, callHandle), or a vpi_scan of
 * vpi_iterate(vpiUserSystf, NULL). `tfname` points at simulator-owned
 * storage valid for the whole run. */
#define vpiUserSystf          67
void vpi_get_systf_info(vpiHandle object, p_vpi_systf_data systf_data_p);

/* User data of a system task / function CALL INSTANCE: a call site in one
 * module instance, so the same site reached again (a loop, a later time
 * step) sees the same data, and another site or another instance of the
 * module has its own. `obj` is a call handle (vpi_handle(vpiSysTfCall,
 * NULL)). vpi_put_userdata returns 1 on success, 0 on failure;
 * vpi_get_userdata returns NULL when nothing was put, or on failure. */
PLI_INT32 vpi_put_userdata(vpiHandle obj, void *userdata);
void *vpi_get_userdata(vpiHandle obj);

/* vpi_get(vpiUserDefn, callHandle) is 1: every call handle is of a
 * user-defined system task or function. */
#define vpiUserDefn           45

/* s_vpi_error_info — filled by vpi_chk_error. `message` and `product` point at
 * simulator-owned storage valid until the next vpi_chk_error call. */
typedef struct t_vpi_error_info {
    PLI_INT32  state;    /* vpiCompile / vpiPLI / vpiRun */
    PLI_INT32  level;    /* vpiNotice / vpiWarning / vpiError / ... */
    PLI_BYTE8 *message;
    PLI_BYTE8 *product;
    PLI_BYTE8 *code;
    PLI_BYTE8 *file;
    PLI_INT32  line;
} s_vpi_error_info, *p_vpi_error_info;

/* Reports the last VPI diagnostic and CLEARS it, returning its severity level,
 * or 0 when nothing has failed since the previous call. `error_info_p` may be
 * NULL if you only want the level. This is the way to notice that a
 * vpi_get_value / vpi_put_value / vpi_register_* call did not do what you
 * asked — most of them cannot report failure any other way. */
PLI_INT32 vpi_chk_error(p_vpi_error_info error_info_p);

/* vpiStop and vpiFinish end the run once the calling routine returns, like
 * $stop / $finish; both accept the usual diagnostic-level argument, which
 * xezim ignores. xezim has no interactive mode, so vpiStop (like $stop) ends
 * the run too, and is reported through cbEnterInteractive.
 * vpiSetInteractiveScope takes a module handle and fires
 * cbInteractiveScopeChange with it. vpiReset is rejected: xezim cannot rewind.
 * Returns 1 on success, 0 on failure (see vpi_chk_error). */
PLI_INT32 vpi_control(PLI_INT32 operation, ...);

/* The entry point xezim calls for every `--vpi-lib` module: a NULL-terminated
 * array of registration routines (IEEE 1800-2017 section 38.2). Define it in
 * your VPI module; do not call it yourself. */
extern void (*vlog_startup_routines[])(void);

/* Returns vpiUndefined (-1) for a property the object does not have.
 * vpiType, vpiSize, vpiSigned, vpiScalar, vpiVector, vpiTimeUnit /
 * vpiTimePrecision (of a module, or of the simulation for NULL), and the
 * object-model properties above (docs/dpi-guide.md, "VPI object model"). */
PLI_INT32 vpi_get(PLI_INT32 property, vpiHandle object);
PLI_INT64 vpi_get64(PLI_INT32 property, vpiHandle object);

/* 1 when both handles refer to the same object, else 0. */
PLI_INT32 vpi_compare_objects(vpiHandle object1, vpiHandle object2);

/* On success, fills *value_p in the requested format. On failure — a bad
 * handle, or a format xezim cannot supply — sets value_p->format to
 * vpiSuppressVal and writes nothing else (IEEE 1800-2017 §38.15), which
 * is the ONLY way a caller can detect the failure. Always check it.
 *
 * Every format above is supported (see s_vpi_value for where each one's
 * value goes). vpiObjTypeVal picks vpiIntVal for an integer-typed object of
 * up to 32 bits, vpiRealVal for a real, vpiTimeVal for a time variable,
 * vpiStringVal for a string, vpiScalarVal for any other 1-bit object and
 * vpiVectorVal for any other vector. A real object read in any format but
 * vpiRealVal, vpiShortRealVal or vpiStringVal is first rounded to an
 * integer; vpiStringVal of a real is its decimal text (16 significant
 * digits). Octal and hex digits read x / z when all of their bits are,
 * X / Z when only some are.
 *
 * For the pointer-valued formats (vectors, strings, times, strengths and
 * the misc formats), the returned pointer addresses simulator-owned storage
 * that is valid only until the next vpi_get_value call on this thread.
 * Copy it out. */
void vpi_get_value(vpiHandle expr, p_vpi_value value_p);

/* Writes value_p to the object. flags selects vpiNoDelay (immediate),
 * vpiForceFlag or vpiReleaseFlag; the delay flags behave as vpiNoDelay
 * because xezim has no VPI event scheduling. Returns NULL. Every format
 * but vpiObjTypeVal and vpiSuppressVal is accepted (vpiStrengthVal only for
 * a scalar object); an integral value put to a real object converts to
 * real, a real put to an integral object is rounded. A format xezim cannot
 * decode writes nothing and is reported through vpi_chk_error. */
vpiHandle vpi_put_value(vpiHandle object, p_vpi_value value_p,
                        p_vpi_time time_p, PLI_INT32 flags);

/* Reads the current simulation time. `object` is accepted for IEEE
 * compatibility and currently ignored. `time_p->type` selects the format:
 * vpiSimTime fills high/low, vpiScaledRealTime fills real. */
void vpi_get_time(vpiHandle object, p_vpi_time time_p);

/* Releases a handle obtained from vpi_handle, vpi_iterate or vpi_scan.
 * Returns 1 on success, 0 on failure (including a NULL handle). */
PLI_INT32 vpi_free_object(vpiHandle object);
PLI_INT32 vpi_release_handle(vpiHandle object);
PLI_INT32 vpi_get_vlog_info(p_vpi_vlog_info vlog_info_p);

/* Registers a callback (IEEE 1800-2017 section 38.36) and returns a
 * vpiCallback handle, or NULL with a vpi_chk_error diagnostic. Every reason
 * above is accepted. The data passed to the routine is a fresh s_cb_data:
 * obj is the handle registered (or as noted), user_data as registered, time
 * the current time in the registered time->type (vpiSimTime when time was
 * NULL), and value is always a valid pointer (vpiSuppressVal when there is
 * nothing to report). The handle stays valid until vpi_remove_cb or
 * vpi_free_object; freeing it does NOT remove the callback.
 *
 * Simulation time (one-shot; removed once fired). The time of
 * cbAtStartOfSimTime / cbAtEndOfSimTime is ABSOLUTE; that of cbAfterDelay,
 * cbNBASynch, cbReadWriteSynch and cbReadOnlySynch is a delay from now (a NULL
 * time is a zero delay); cbNextSimTime ignores the time value. vpiScaledRealTime
 * is in the time unit of obj (a module), else in simulation ticks. Any of them
 * holds the scheduler at its time even when no HDL event is due there. Within
 * a time step they run in this order:
 *   cbNextSimTime, cbAtStartOfSimTime   before any event of the step
 *   cbAfterDelay                        with the step's first events
 *   cbNBASynch                          before the NBA region
 *   cbReadWriteSynch                    after the NBA region, once no
 *                                       process of the step is left to run
 *   cbAtEndOfSimTime                    after every other region
 *   cbReadOnlySynch                     last; vpi_put_value is refused
 * A value written from any of them but cbReadOnlySynch opens a fresh delta
 * in the same step, so edge-sensitive processes see it.
 *
 * Simulation events (fire until removed). cbValueChange takes a net,
 * variable, part-select, port or memory (index = the word that changed) and
 * fires once per change, whichever path made it; value is in the registered
 * value->format (vpiIntVal if value was NULL; vpiSuppressVal for none).
 * cbForce/cbRelease (nets and variables) and cbAssign/cbDeassign (variables)
 * take a net/variable or NULL for all; they fire after the SystemVerilog
 * statement or the vpi_put_value vpiForceFlag/vpiReleaseFlag has taken
 * effect, cbRelease/cbDeassign once the object has been re-driven. value is
 * the object's resulting value. xezim has no statement objects, so obj is the
 * affected net/variable (a handle valid only during the call when NULL was
 * registered). cbDisable takes the vpiSysTfCall handle of a running $systf;
 * it fires after a `disable` of a named block, fork or task that encloses the
 * call terminates it. cbStmt takes a module handle and fires before each
 * statement that a process of that instance executes in the interpreter
 * (statements compiled to native or bytecode form are not reported); obj is
 * the module handle.
 *
 * Actions. cbEndOfCompile then cbStartOfSimulation, before time 0;
 * cbEndOfSimulation after the last time step (preceded by cbEnterInteractive
 * when $stop or vpi_control(vpiStop) ended the run). cbError on each run-time
 * error ($error, $fatal, a reported timing violation, an illegal bin),
 * cbPLIError on each VPI routine error; inside either, vpi_chk_error reports
 * the error. cbTchkViolation on each timing-check violation (obj NULL, value
 * the violation text as vpiStringVal). cbSignal when SIGINT/SIGTERM stops the
 * run (index = the signal number). cbUnresolvedSystf the first time an
 * unknown $name is called (value->value.str = the name); if the routine
 * registers it with vpi_register_systf, the call goes to it. */
vpiHandle vpi_register_cb(p_cb_data cb_data_p);
/* Fills `cb_data_p` with the registration data: time and value point at
 * simulator-owned copies of what was registered (NULL if nothing was).
 * Returns 1, or 0 when the handle is not a registered callback (including a
 * one-shot that has fired). */
PLI_INT32 vpi_get_cb_info(vpiHandle cb_obj, p_cb_data cb_data_p);
/* Removes the callback and frees its handle; allowed from any callback
 * routine, the callback's own included. Returns 1, or 0 for a handle that is
 * not a callback. */
PLI_INT32 vpi_remove_cb(vpiHandle cb_obj);

/* --- Arrays (IEEE 1800-2017 sections 38.16, 38.20, 38.35) ------------- */

/* Array object types. vpi_handle_by_name answers vpiMemory for a
 * one-dimensional array, vpiRegArray / vpiNetArray for a multi-dimensional
 * one and for a sub-array of one (`top.m[1]` of `logic [7:0] m[0:2][0:3]`). */

/* The subobject selected by `num_index` indices, leftmost first: one per
 * unpacked dimension still open on `obj` gives an element, fewer a
 * sub-array. Further indices select through the element's packed
 * dimensions: a part-select (vpiPartSelect) until the last one, which
 * selects a bit (vpiRegBit / vpiNetBit). A plain vector takes only packed
 * indices. NULL when the indices do not form a legal select.
 * vpi_handle_by_index on an array is the one-index case. */
vpiHandle vpi_handle_by_multi_index(vpiHandle obj, PLI_INT32 num_index,
                                    PLI_INT32 *index_array);

typedef struct t_vpi_arrayvalue {
    PLI_UINT32 format;  /* vpi[Int,Real,Time,ShortInt,LongInt,ShortReal,
                           RawTwoState,RawFourState,Vector]Val */
    PLI_UINT32 flags;   /* vpiUserAllocFlag; vpiOneValue, vpiPropagateOff */
    union {
        PLI_INT32           *integers;
        PLI_INT16           *shortints;
        PLI_INT64           *longints;
        PLI_BYTE8           *rawvals;
        struct t_vpi_vecval *vectors;
        struct t_vpi_time   *times;
        double              *reals;
        float               *shortreals;
    } value;
} s_vpi_arrayvalue, *p_vpi_arrayvalue;

#define vpiUserAllocFlag      0x2000   /* get: value points at caller memory */
#define vpiOneValue           0x4000   /* put: one value for every element */
#define vpiPropagateOff       0x8000   /* put: do not wake the readers */

/* Read / write `num` consecutive elements of a static unpacked array (or a
 * sub-array) starting at index_p — one index per open dimension, leftmost
 * first. The rightmost dimension varies fastest, and every dimension runs
 * from its declared left bound towards its right one; a section that runs
 * past the end of the array is an error.
 *
 * Formats: vpiIntVal, vpiTimeVal, vpiVectorVal, vpiRawTwoStateVal and
 * vpiRawFourStateVal for any integral element type; vpiRealVal for real
 * elements; vpiShortRealVal for shortreal ones. vpiShortIntVal reads
 * shortint / byte elements and writes shortint / int / longint ones;
 * vpiLongIntVal reads longint / shortint / byte elements and writes
 * longint ones. Raw layout per element: ceil(size/8) aval bytes, then (four
 * state) as many bval bytes, bit 0 in the LSB of the first byte.
 *
 * vpi_get_value_array stores into simulator-owned memory, valid until the
 * next call, unless vpiUserAllocFlag says value points at the caller's
 * buffer. On any error it sets the value pointer to NULL (and reports
 * through vpi_chk_error). vpi_put_value_array writes like vpi_put_value
 * with vpiNoDelay; vpiOneValue writes the first value to every element,
 * and vpiPropagateOff stores the values without waking the processes and
 * callbacks that watch them. Any other flag is an error, and on any error
 * nothing is written. */
void vpi_get_value_array(vpiHandle object, p_vpi_arrayvalue arrayvalue_p,
                         PLI_INT32 *index_p, PLI_UINT32 num);
void vpi_put_value_array(vpiHandle object, p_vpi_arrayvalue arrayvalue_p,
                         PLI_INT32 *index_p, PLI_UINT32 num);

/* --- Delays (IEEE 1800-2017 sections 38.10, 38.22, 38.32) -------------- */

#define vpiInterModPath       26   /* intermodule path (port to port) */

/* vpi_get(vpiTchkType, tchk). */

typedef struct t_vpi_delay {
    struct t_vpi_time *da;   /* caller-allocated array of delay values */
    PLI_INT32 no_of_delays;
    PLI_INT32 time_type;     /* vpiScaledRealTime or vpiSimTime */
    PLI_INT32 mtm_flag;      /* min:typ:max triples */
    PLI_INT32 append_flag;   /* put: add to the current delays */
    PLI_INT32 pulsere_flag;  /* delay, reject limit, error limit triples */
} s_vpi_delay, *p_vpi_delay;

/* vpi_iterate(vpiModPath, module) and vpi_iterate(vpiTchk, module) give the
 * module paths and timing checks of a module instance (vpiFullName of a
 * module path is its output net's; of a timing check, its scope plus the
 * check's name). vpi_handle_multi(vpiInterModPath, outPort, inPort) gives
 * the interconnect between an output port and an input port of the same
 * size; only those two reference handles are read. */
vpiHandle vpi_handle_multi(PLI_INT32 type, vpiHandle refHandle1,
                           vpiHandle refHandle2, ...);

/* The delays of an object, and the values the simulator then uses:
 *   - a net or port: the delay of the gate or continuous assignment that
 *     drives it, or its SDF / VPI annotation — 1, 2 (rise, fall) or 3
 *     (rise, fall, turn-off) delays. xezim lowers every gate and continuous
 *     assignment onto the net it drives, so the net is where their delays
 *     are read and (as an extension) written. A net whose driver was built
 *     without a delay in a form that cannot take one, or that is driven
 *     through module paths, refuses vpi_put_delays;
 *   - vpiModPath: 1, 2, 3, 6 or 12 transition delays;
 *   - vpiTchk: its limits, as many as the check has (one, or two for
 *     $setuphold, $recrem, $fullskew and $nochange), in source order;
 *   - vpiInterModPath: 2 or 3 delays, stored where SDF INTERCONNECT delays
 *     land: on the input port's net.
 * time_type vpiSimTime counts simulation ticks (a negative timing check
 * limit is two's complement in high/low); vpiScaledRealTime counts the
 * object's module time unit. xezim keeps one value per delay (the active
 * min:typ:max selection) and models inertial delays with no separate pulse
 * limits: with mtm_flag, min, typ and max all read that value and a put
 * takes the active selection's; with pulsere_flag, the reject and error
 * limits read the delay, and a put whose limits differ from its delay is
 * refused. append_flag adds the given values to the current ones. Errors
 * (and refusals, which write nothing) are reported through vpi_chk_error. */
void vpi_get_delays(vpiHandle object, p_vpi_delay delay_p);
void vpi_put_delays(vpiHandle object, p_vpi_delay delay_p);

/* --- Save / restart (IEEE 1800-2017 sections 38.9, 38.31) -------------- */

/* The standard allows these only from cbStartOfSave / cbEndOfSave (put) and
 * cbStartOfRestart / cbEndOfRestart (get) callbacks. xezim has no $save or
 * $restart, so those callbacks never fire and both routines always fail:
 * they return 0 (no bytes transferred) and report the error through
 * vpi_chk_error. */
PLI_INT32 vpi_get_data(PLI_INT32 id, PLI_BYTE8 *dataLoc, PLI_INT32 numOfBytes);
PLI_INT32 vpi_put_data(PLI_INT32 id, PLI_BYTE8 *dataLoc, PLI_INT32 numOfBytes);

#ifdef __cplusplus
}
#endif

/* DPI scope/runtime primitives live in svdpi.h with their proper
 * `svScope` type. Included here so both are visible together. */
#include "svdpi.h"

#endif /* VPI_USER_H */
