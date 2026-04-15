#include <stdint.h>

typedef struct {
    uint32_t *aval;
    uint32_t *bval;
} svLogicVecVal;

static uint32_t g_seen_lsb = 0;

void vec_in(const svLogicVecVal *x) {
    if (!x || !x->aval) {
        g_seen_lsb = 0;
        return;
    }
    g_seen_lsb = x->aval[0];
}

void vec_flip(svLogicVecVal *x) {
    if (!x || !x->aval || !x->bval) {
        return;
    }
    x->aval[0] ^= 0x000000FFu;
    x->bval[0] = 0u;
}

void vec_set(svLogicVecVal *x) {
    if (!x || !x->aval || !x->bval) {
        return;
    }
    x->aval[0] = 0xAABBCCDDu;
    x->aval[1] = 0x55667788u;
    x->aval[2] = 0x11223344u;
    x->bval[0] = 0u;
    x->bval[1] = 0u;
    x->bval[2] = 0u;
}

int vec_seen_lsb(void) {
    return (int)g_seen_lsb;
}
