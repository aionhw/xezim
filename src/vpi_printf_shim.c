/* vpi_printf / vpi_vprintf — IEEE 1800-2017 section 38.34.
 *
 * These live in C rather than Rust because defining a C-variadic function
 * requires the unstable `c_variadic` feature. They are trivial, so the
 * shim stays trivial: forward to vprintf and flush, since VPI output is
 * expected to interleave with $display.
 *
 * Compiled and linked by build.rs.
 */
#include <stdio.h>
#include <stdarg.h>

int vpi_vprintf(char *format, va_list ap) {
    int n = vprintf(format, ap);
    fflush(stdout);
    return n;
}

int vpi_printf(char *format, ...) {
    va_list ap;
    va_start(ap, format);
    int n = vprintf(format, ap);
    va_end(ap);
    fflush(stdout);
    return n;
}

int vpi_mcd_printf(unsigned int mcd, char *format, ...) {
    (void)mcd;
    va_list ap;
    va_start(ap, format);
    int n = vprintf(format, ap);
    va_end(ap);
    fflush(stdout);
    return n;
}
