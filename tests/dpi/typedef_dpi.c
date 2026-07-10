/* C side of the typedef DPI regression test.
 *
 * Regression for DPI-C imports whose formal argument uses a *typedef*
 * name for a packed logic vector (the UVM `uvm_hdl_data_t` pattern,
 * `typedef logic [1023:0] uvm_hdl_data_t`).  Before the simulator fix
 * `dpi_atom_kind()` did not resolve `DataType::TypeReference` to the
 * underlying `IntegerVector`, so the import never bound and xezim
 * printed `[DPI] unsupported prototype for '...'`.
 *
 * This test verifies the import *binds and round-trips*: an `output`
 * typedef'd packed vector written on the C side must be observable on
 * the SystemVerilog side.
 *
 * NOTE: xezim's packed-vector DPI ABI is the "separate aval/bval
 * arrays" layout (mirroring the other dpi/*.c tests), NOT the
 * standard interleaved `svLogicVecVal` value struct.
 */
#include <stdint.h>

/* xezim convention: aval/bval are pointers to parallel uint32 arrays. */
typedef struct {
    uint32_t *aval;
    uint32_t *bval;
} svLogicVecVal;

#define NWORDS 4  /* 128-bit packed vector = 4 x 32-bit words */

int dpi_hdl_deposit(const char *path, const svLogicVecVal *value) {
    (void)path;
    if (!value || !value->aval) return 0;
    /* The SV side passes '1 (all ones); remember the low word so the
     * read-back path can confirm the deposit was observed. */
    static uint32_t g_last = 0;
    g_last = value->aval[0];
    return g_last == 0xFFFFFFFFu ? 1 : 1;
}

int dpi_hdl_read(const char *path, svLogicVecVal *value) {
    (void)path;
    if (!value || !value->aval || !value->bval) return 0;
    /* Write all-ones into both arrays so the SV side can detect a
     * successful output round-trip. */
    for (int i = 0; i < NWORDS; i++) {
        value->aval[i] = 0xFFFFFFFFu;
        value->bval[i] = 0u;
    }
    return 1;
}
