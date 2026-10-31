/* A 4-state actual bound to a two-state `bit [N]` formal converts as by
 * assignment (IEEE 1800-2017 §35.6.1, §6.11.2): x and z bits reach C as 0,
 * so the svBitVecVal words hold only the actual's known 1 bits. */
#include "svdpi.h"

static svBitVecVal seen[3];

void bxz_in(const svBitVecVal *v) { seen[0] = v[0]; }
void bxz_in_wide(const svBitVecVal *v) { seen[0] = v[0]; seen[1] = v[1]; seen[2] = v[2]; }
void bxz_inout(svBitVecVal *v) { seen[0] = v[0]; v[0] = 0x5; }
int bxz_seen(int i) { return (int)seen[i]; }
