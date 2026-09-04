/* Compilation-unit DPI: imports and an export declared at $unit scope,
   small-integer return types, a chandle round trip, and a 1-bit 4-state
   argument (svLogic encoding: x is 3, z is 2). Own names. */
#include "svdpi.h"
#include <stdlib.h>
int us_add(int a, int b) { return a + b; }
void us_out_args(int a, int* sum, int* diff) { *sum = a + 1; *diff = a - 1; }
unsigned char us_ub(unsigned char b) { return (unsigned char)(b + 1); }
short us_sh(short s) { return (short)(s - 1); }
extern int us_sv_cb(int);
int us_call_back(int v) { return us_sv_cb(v) + 1000; }
void* us_mk_handle(void) { int* p = malloc(sizeof(int)); *p = 42; return p; }
int us_rd_handle(void* h) { return *(int*)h; }
int us_logic_arg(svLogic l) { return (int)l; }
