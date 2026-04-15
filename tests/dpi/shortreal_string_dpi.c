#include <stdio.h>

float add_sr(float a, float b) {
    return a + b;
}

void scale_sr(float *x, float s) {
    if (x) {
        *x = (*x) * s;
    }
}

void set_msg(const char **s) {
    static const char *msg = "hello_out";
    if (s) {
        *s = msg;
    }
}

void append_msg(const char **s) {
    static char buf[256];
    const char *in = (s && *s) ? *s : "";
    snprintf(buf, sizeof(buf), "%s_tail", in);
    if (s) {
        *s = buf;
    }
}
