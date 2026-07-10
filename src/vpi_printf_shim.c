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

/* vpi_control — IEEE 1800-2017 section 38.14. Variadic, so it lives here too;
 * the operation's optional argument (the $finish/$stop diagnostic level) is
 * unpacked and forwarded to the Rust backend. */
extern int xezim_vpi_control(int operation, int arg);

#define XEZIM_vpiStop   66
#define XEZIM_vpiFinish 67

int vpi_control(int operation, ...) {
    int arg = 0;
    if (operation == XEZIM_vpiStop || operation == XEZIM_vpiFinish) {
        va_list ap;
        va_start(ap, operation);
        arg = va_arg(ap, int);
        va_end(ap);
    }
    return xezim_vpi_control(operation, arg);
}
