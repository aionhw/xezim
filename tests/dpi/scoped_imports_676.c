/* IEEE 1800-2023 26.3, 35.5.3, 35.5.4: package-local SV names and C linkage. */
#include "svdpi.h"
#include <string.h>

extern int wait_step(void);

static int in_scope(const char *name) {
    const char *actual = svGetNameFromScope(svGetScope());
    return actual && strcmp(actual, name) == 0;
}

int c_first(int value) { return in_scope("first_pkg") ? value + 100 : -1; }
int c_second(int value) { return in_scope("second_pkg") ? value + 200 : -2; }
int c_design(int value) { return in_scope("top") ? value + 300 : -3; }

static int action(const char *scope, int expected, int *value) {
    *value = in_scope(scope) ? expected : -4;
    svScope saved = svSetScope(svGetScopeFromName("top"));
    wait_step();
    svSetScope(saved);
    return 0;
}

int c_first_task(int *value) { return action("first_pkg", 11, value); }
int c_second_task(int *value) { return action("second_pkg", 22, value); }
