#include <stdint.h>
#include <stdio.h>

int64_t add64(int64_t a, int64_t b) {
    return a + b;
}

double scale(double a, double b) {
    return a * b;
}

void bump(int *x) {
    if (x) {
        *x += 1;
    }
}

void split(int a, int b, int *s) {
    if (s) {
        *s = a + b;
    }
}

void *make_handle(int x) {
    return (void *)(intptr_t)(x * 3);
}

int use_handle(void *h) {
    return (int)(intptr_t)h;
}

const char *greet(const char *s) {
    static char buf[128];
    snprintf(buf, sizeof(buf), "hello_%s", s ? s : "");
    return buf;
}
