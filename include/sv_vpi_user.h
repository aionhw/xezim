#ifndef SV_VPI_USER_H
#define SV_VPI_USER_H

/* Compatibility shim for the Accellera UVM reference implementation.
 *
 * Some UVM source files (`uvm_hdl_polling.c` and a vendor-specific backend)
 * `#include "sv_vpi_user.h"` directly. The full Accellera header
 * declares vpiTypes.h contents and the vlog_chk_error / io_printf
 * family of legacy PLI v1.0 helpers — xezim doesn't implement
 * those, but the C compile needs the include chain to resolve.
 *
 * The actual types UVM uses (`svScope`, `svLogicVecVal`) now live
 * in `svdpi.h` per IEEE 1800 §35.5.5. This file just re-includes
 * `svdpi.h` and provides a few extra typedefs the legacy header
 * is expected to define.
 */

#include "svdpi.h"
#include "vpi_user.h"

/* Legacy typedefs from the full sv_vpi_user.h that some UVM source
 * files still expect to be visible after `#include "sv_vpi_user.h"`.
 * The full Accellera header defines these via vpi_user.h + vpi_compatibility.h;
 * for xezim, both types are already available from vpi_user.h. */


/* Generic instance class (module/program/interface/package). Accepted by
 * vpi_iterate to enumerate the design root and a scope's child instances. */
#define vpiInstance 745

/* --- Object model (IEEE 1800-2017 chapter 37): SystemVerilog objects ---- */

/* Object types. */
#define vpiPackage           600
#define vpiInterface         601
#define vpiProgram           602
#define vpiTypespec          605   /* also the vpi_handle relation */
#define vpiModport           606   /* an interface's modport; vpiIODecl iterates its ports */
#define vpiRefObj            608   /* a name the object model cannot resolve */
#define vpiVarBit            vpiRegBit
#define vpiArrayVar          vpiRegArray
#define vpiClassVar          615
#define vpiChandleVar        622
#define vpiPackedArrayVar    623
#define vpiVirtualInterfaceVar 728
#define vpiLongIntTypespec   625
#define vpiShortRealTypespec 626
#define vpiByteTypespec      627
#define vpiShortIntTypespec  628
#define vpiIntTypespec       629
#define vpiClassTypespec     630
#define vpiStringTypespec    631
#define vpiChandleTypespec   632
#define vpiEnumTypespec      633
#define vpiEnumConst         634   /* also the vpi_iterate relation */
#define vpiIntegerTypespec   635
#define vpiTimeTypespec      636
#define vpiRealTypespec      637
#define vpiStructTypespec    638
#define vpiUnionTypespec     639
#define vpiBitTypespec       640
#define vpiLogicTypespec     641
#define vpiArrayTypespec     642
#define vpiVoidTypespec      643
#define vpiTypespecMember    644   /* also the vpi_iterate relation */
#define vpiPackedArrayTypespec 692
#define vpiEventTypespec     698
#define vpiFinal             676
#define vpiArrayNet          vpiNetArray
#define vpiEnumNet           680
#define vpiIntegerNet        681
#define vpiLogicNet          vpiNet
#define vpiTimeNet           682
#define vpiStructNet         683
#define vpiPackedArrayNet    693

/* Relations. */
#define vpiBaseTypespec      703
#define vpiElemTypespec      704
#define vpiMember            742

/* Properties and their values. */
#define vpiTop               600
#define vpiUnit              602
#define vpiJoinType          603
#define vpiJoin                0
#define vpiJoinNone            1
#define vpiJoinAny             2
#define vpiAccessType        604
#define vpiForkJoinAcc         1
#define vpiExternAcc           2
#define vpiDPIExportAcc        3
#define vpiDPIImportAcc        4
#define vpiArrayType         606
#define vpiStaticArray         1
#define vpiDynamicArray        2
#define vpiAssocArray          3
#define vpiQueueArray          4
#define vpiArrayMember       607
#define vpiPortType          611
#define vpiInterfacePort       1
#define vpiModportPort         2
#define vpiConstantVariable  612
#define vpiStructUnionMember 615
#define vpiVisibility        620
#define vpiPublicVis           1
#define vpiProtectedVis        2
#define vpiLocalVis            3
#define vpiAlwaysType        624
#define vpiAlwaysComb          2
#define vpiAlwaysFF            3
#define vpiAlwaysLatch         4
#define vpiPacked            630
#define vpiTagged            632
#define vpiRef                 6   /* vpiDirection of a ref port or argument */
#define vpiDPIPure           665
#define vpiDPIContext        666
#define vpiOtherFunc           6   /* vpiFuncType: returns a non-integral type */

/* vpiOpType values added by SystemVerilog. */
#define vpiPostIncOp          62
#define vpiPreIncOp           63
#define vpiPostDecOp          64
#define vpiPreDecOp           65
#define vpiWildEqOp           69
#define vpiWildNeqOp          70
#define vpiStreamLROp         71
#define vpiStreamRLOp         72
#define vpiAssignmentPatternOp 75
#define vpiInsideOp           95

/* --- Assertion API (IEEE 1800-2017 Clause 39, Annex M) ------------------ */

/* Assertion object types. */
#define vpiPropertyDecl      655
#define vpiPropertySpec      656
#define vpiPropertyExpr      657
#define vpiPropertyInst      660
#define vpiSequenceDecl      661
#define vpiSequenceInst      664
#define vpiImmediateAssert   665
#define vpiAssert            686
#define vpiAssume            687
#define vpiCover             688
#define vpiDisableCondition  689
#define vpiClockingEvent     690
#define vpiImmediateAssume   694
#define vpiImmediateCover    695

/* Relations. */
#define vpiProperty          718
#define vpiConcurrentAssertion 740
#define vpiAssertion         744   /* a scope's assertions, property instances
                                      and property declarations */

/* Properties. */
#define vpiIsCoverSequence   659

/* Assertion coverage properties (Clause 40). */
#define vpiAssertAttemptCovered        770
#define vpiAssertSuccessCovered        771
#define vpiAssertFailureCovered        772
#define vpiAssertVacuousSuccessCovered 773
#define vpiAssertDisableCovered        774
#define vpiAssertKillCovered           777

/* Callback reasons of vpi_register_assertion_cb. */
#define cbAssertionStart                606
#define cbAssertionSuccess              607
#define cbAssertionFailure              608
#define cbAssertionStepSuccess          609
#define cbAssertionStepFailure          610
#define cbAssertionDisable              611
#define cbAssertionEnable               612
#define cbAssertionReset                613
#define cbAssertionKill                 614
#define cbAssertionEnablePassAction     645
#define cbAssertionEnableFailAction     646
#define cbAssertionDisablePassAction    647
#define cbAssertionDisableFailAction    648
#define cbAssertionEnableNonvacuousAction 649
#define cbAssertionDisableVacuousAction 650
#define cbAssertionVacuousSuccess       657
#define cbAssertionDisabledEvaluation   658
#define cbAssertionLock                 661
#define cbAssertionUnlock               662

/* Assertion system callback reasons (vpi_register_cb). */
#define cbAssertionSysInitialized       615
#define cbAssertionSysOn                616
#define cbAssertionSysOff               617
#define cbAssertionSysEnd               618
#define cbAssertionSysReset             619
#define cbAssertionSysKill              631
#define cbAssertionSysEnablePassAction  651
#define cbAssertionSysEnableFailAction  652
#define cbAssertionSysDisablePassAction 653
#define cbAssertionSysDisableFailAction 654
#define cbAssertionSysEnableNonvacuousAction 655
#define cbAssertionSysDisableVacuousAction 656
#define cbAssertionSysLock              659
#define cbAssertionSysUnlock            660

/* Assertion operations of vpi_control. */
#define vpiAssertionDisable             620
#define vpiAssertionEnable              621
#define vpiAssertionReset               622
#define vpiAssertionKill                623
#define vpiAssertionEnableStep          624
#define vpiAssertionDisableStep         625
#define vpiAssertionClockSteps          626   /* the step control constant */
#define vpiAssertionSysOn               627
#define vpiAssertionSysOff              628
#define vpiAssertionSysEnd              629
#define vpiAssertionSysReset            630
#define vpiAssertionSysKill             632
#define vpiAssertionDisablePassAction   633
#define vpiAssertionEnablePassAction    634
#define vpiAssertionDisableFailAction   635
#define vpiAssertionEnableFailAction    636
#define vpiAssertionDisableVacuousAction 637
#define vpiAssertionEnableNonvacuousAction 638
#define vpiAssertionSysDisablePassAction 639
#define vpiAssertionSysEnablePassAction 640
#define vpiAssertionSysDisableFailAction 641
#define vpiAssertionSysEnableFailAction 642
#define vpiAssertionSysDisableVacuousAction 643
#define vpiAssertionSysEnableNonvacuousAction 644
#define vpiAssertionLock                645
#define vpiAssertionUnlock              646
#define vpiAssertionSysLock             647
#define vpiAssertionSysUnlock           648

#ifdef __cplusplus
extern "C" {
#endif

/* Attempt information passed to an assertion callback. */
typedef struct t_vpi_assertion_step_info {
    PLI_INT32 matched_expression_count;
    vpiHandle *matched_exprs;          /* array of expressions */
    PLI_INT32 stateFrom, stateTo;      /* identify transition */
} s_vpi_assertion_step_info, *p_vpi_assertion_step_info;

typedef struct t_vpi_attempt_info {
    union {
        vpiHandle failExpr;
        p_vpi_assertion_step_info step;
    } detail;
    s_vpi_time attemptStartTime;       /* time attempt triggered */
} s_vpi_attempt_info, *p_vpi_attempt_info;

typedef PLI_INT32 (vpi_assertion_callback_func)(
    PLI_INT32 reason,                  /* callback reason */
    p_vpi_time cb_time,                /* callback time */
    vpiHandle assertion,               /* handle to assertion */
    p_vpi_attempt_info info,           /* attempt related information */
    PLI_BYTE8 *user_data);             /* user data entered upon registration */

/* Returns a callback handle, removed with vpi_remove_cb. */
vpiHandle vpi_register_assertion_cb(
    vpiHandle assertion,               /* handle to assertion */
    PLI_INT32 reason,                  /* reason for which callbacks needed */
    vpi_assertion_callback_func *cb_rtn,
    PLI_BYTE8 *user_data);

#ifdef __cplusplus
}
#endif

#endif /* SV_VPI_USER_H */
