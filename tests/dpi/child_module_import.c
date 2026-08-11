#include <stdio.h>
void* child_probe_open(const char* name) {
    printf("[probe] open '%s'\n", name);
    fflush(stdout);
    return (void*)0x1234;
}
