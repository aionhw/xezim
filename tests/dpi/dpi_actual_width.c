/* C side of dpi_actual_width_test.sv: each function returns (or writes back)
 * what it received, so the SV side sees the value the actual passed. */
#include "svdpi.h"

int dpi_echo_int(int v) { return v; }
short dpi_echo_short(short v) { return v; }
long long dpi_echo_long(long long v) { return v; }
int dpi_echo_td(int v) { return v; }
int dpi_echo_u16(const svBitVecVal *v) { return (int)(v[0] & 0xffff); }
int dpi_echo_b16(const svBitVecVal *v) { return (int)(v[0] & 0xffff); }
int dpi_echo_l16(const svLogicVecVal *v) { return (int)(v[0].aval & 0xffff); }
int dpi_echo_dflt(int v) { return v; }
int dpi_add2(int a, int b) { return a + b; }
svBit dpi_echo_bit(svBit v) { return v; }
void dpi_inc(int *v) { *v += 1; }
int dpi_task_echo(int v, int *o) { *o = v; return 0; }
