/* §35.5.4 DPI export round-trip: an imported context function calls back
   into exported SystemVerilog functions/tasks across scalar type classes
   (int, longint, real, string-in, void task). Own names, not from any
   external source. */
#include <string.h>
extern int       sv_scale(int x);            /* x * 3            */
extern int       sv_combine(int a, int b);   /* a + b            */
extern void      sv_record(int v);           /* task: store v    */
extern long long sv_widen(long long x);      /* x + 2^32         */
extern double    sv_half(double x);          /* x / 2.0          */
extern void      sv_note(const char* s);     /* task: store str  */

int c_roundtrip(int x)
{
    int s = sv_scale(x);          /* 3x     */
    int c = sv_combine(s, x);     /* 3x + x */
    sv_record(c);                 /* task write-back */
    return c + 1;
}

long long c_widen(long long v) { return sv_widen(v); }

int c_real_and_str(void)
{
    double h = sv_half(9.0);      /* 4.5 -> encode as *10 = 45 */
    sv_note("world");
    return (int)(h * 10.0);
}
