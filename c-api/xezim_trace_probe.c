/*
 * xezim_trace_probe.c — exercises the xezim_trace C ABI end to end.
 *
 * Usage:
 *   xezim_trace_probe GRAPH CENSUS SIGNAL DRIVER LOAD
 *                      EXPECTED_SIGNALS EXPECTED_CHANGES
 *
 * Runs the two-call string protocol over the graph and census handles and
 * checks positive and negative paths:
 *   - the sidecars load and report the expected row counts;
 *   - the named row exists and lists DRIVER among its drivers and LOAD
 *     among its loads;
 *   - census row SIGNAL reports EXPECTED_CHANGES changes;
 *   - a missing sidecar yields NULL and a non-empty last-error message;
 *   - null handles, out-of-range indexes, and too-small buffers all return
 *     -1 with a usable message.
 *
 * Exits 0 on every check passing, 1 on the first failure (message on
 * stderr). Compile and run, e.g.:
 *   cc -I c-api c-api/xezim_trace_probe.c -L target/debug -lxezim \
 *      -o /tmp/xezim_trace_probe
 *   LD_LIBRARY_PATH=target/debug /tmp/xezim_trace_probe ...
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "xezim_trace.h"

static int failures;

#define CHECK(cond, ...)                                                       \
    do {                                                                       \
        if (!(cond)) {                                                         \
            fprintf(stderr, "probe FAIL: " __VA_ARGS__);                       \
            fprintf(stderr, "\n");                                             \
            failures = 1;                                                      \
        }                                                                      \
    } while (0)

/* Two-call string protocol for single-argument functions like
 * graph_signal, driver_count, load_count. */
static char *get_string1(
    int (*fn)(const xezim_trace_graph *, size_t, char *, size_t *),
    const xezim_trace_graph *handle, size_t index) {
    size_t need = 0;
    if (fn(handle, index, NULL, &need) != 0 || need == 0) {
        return NULL;
    }
    char *buf = (char *)malloc(need);
    if (!buf) {
        return NULL;
    }
    if (fn(handle, index, buf, &need) != 0) {
        free(buf);
        return NULL;
    }
    return buf;
}

/* Two-call for driver_at / load_at (two indices). */
static char *get_string2(
    int (*fn)(const xezim_trace_graph *, size_t, size_t, char *, size_t *),
    const xezim_trace_graph *handle, size_t idx1, size_t idx2) {
    size_t need = 0;
    if (fn(handle, idx1, idx2, NULL, &need) != 0 || need == 0) {
        return NULL;
    }
    char *buf = (char *)malloc(need);
    if (!buf) {
        return NULL;
    }
    if (fn(handle, idx1, idx2, buf, &need) != 0) {
        free(buf);
        return NULL;
    }
    return buf;
}

/* Two-call for census signal_at: size query passes both path pointers NULL. */
static int census_size_query(const xezim_trace_census *c, size_t index) {
    return xezim_trace_census_signal_at(c, index, NULL, NULL, NULL, NULL,
                                        NULL, NULL, NULL);
}

/* Full census query for row `index`, filling all outputs. */
static int census_full_query(const xezim_trace_census *c, size_t index,
                             char *path_buf, size_t *path_cap,
                             unsigned long long *w,
                             unsigned long long *cc,
                             double *f, double *l,
                             int *changed) {
    return xezim_trace_census_signal_at(c, index, path_buf, path_cap,
                                        w, cc, f, l, changed);
}

int main(int argc, char **argv) {
    if (argc != 8) {
        fprintf(stderr,
                "usage: %s GRAPH CENSUS SIGNAL DRIVER LOAD "
                "EXPECTED_SIGNALS EXPECTED_CHANGES\n",
                argv[0]);
        return 2;
    }
    const char *graph_path = argv[1];
    const char *census_path = argv[2];
    const char *signal = argv[3];
    const char *driver = argv[4];
    const char *load = argv[5];
    size_t expected_signals = (size_t)strtoul(argv[6], NULL, 10);
    unsigned long long expected_changes =
        strtoull(argv[7], NULL, 10);

    /* ---- graph: positive ---- */
    xezim_trace_graph *g = xezim_trace_graph_load(graph_path);
    CHECK(g != NULL, "graph load failed");
    if (!g)
        return failures;

    size_t rows = 0;
    CHECK(xezim_trace_graph_signal_count(g, &rows) == 0,
          "signal_count failed");
    CHECK(rows == expected_signals,
          "signal_count %zu != expected %zu", rows, expected_signals);

    /* find the row for SIGNAL */
    size_t signal_index = (size_t)-1;
    for (size_t i = 0; i < rows; i++) {
        char *path = get_string1(xezim_trace_graph_signal, g, i);
        if (path && strcmp(path, signal) == 0) {
            signal_index = i;
            free(path);
            break;
        }
        free(path);
    }
    CHECK(signal_index != (size_t)-1, "row for %s not found", signal);

    /* DRIVER must appear somewhere among the row's drivers. */
    {
        size_t dc = 0;
        size_t found = 0;
        CHECK(xezim_trace_graph_driver_count(g, signal_index, &dc) == 0,
              "driver_count failed");
        for (size_t i = 0; i < dc; i++) {
            char *d = get_string2(xezim_trace_graph_driver_at, g, signal_index, i);
            if (d && strcmp(d, driver) == 0)
                found = 1;
            free(d);
        }
        CHECK(found, "%s not listed among the drivers of %s", driver, signal);
    }

    /* LOAD must appear somewhere among the row's loads. */
    {
        size_t lc = 0;
        size_t found = 0;
        CHECK(xezim_trace_graph_load_count(g, signal_index, &lc) == 0,
              "load_count failed");
        for (size_t i = 0; i < lc; i++) {
            char *l = get_string2(xezim_trace_graph_load_at, g, signal_index, i);
            if (l && strcmp(l, load) == 0)
                found = 1;
            free(l);
        }
        CHECK(found, "%s not listed among the loads of %s", load, signal);
    }

    /* ---- graph: negative ---- */
    {
        xezim_trace_graph *bad = xezim_trace_graph_load("___no_such_file.json");
        CHECK(bad == NULL, "loading a missing file must return NULL");
        const char *msg = xezim_trace_last_error();
        CHECK(msg != NULL && strlen(msg) > 0,
              "last_error after a failed load must be non-empty");

        size_t n;
        CHECK(xezim_trace_graph_signal_count(NULL, &n) == -1,
              "null handle must be an error");
        CHECK(xezim_trace_last_error() != NULL,
              "last_error after null-handle call must be set");

        CHECK(xezim_trace_graph_signal(g, rows + 5, NULL, &n) == -1,
              "out-of-range index must be an error");
        CHECK(xezim_trace_last_error() != NULL,
              "last_error after out-of-range call must be set");

        char tiny[1];
        size_t tinycap = sizeof(tiny);
        CHECK(xezim_trace_graph_signal(g, signal_index, tiny, &tinycap) == -1,
              "too-small buffer must be an error");
        CHECK(tinycap > sizeof(tiny),
              "too-small buffer must report the required size");
    }

    /* ---- census: positive ---- */
    xezim_trace_census *c = xezim_trace_census_load(census_path);
    CHECK(c != NULL, "census load failed");
    if (!c)
        return failures;

    size_t crows = 0;
    CHECK(xezim_trace_census_signal_count(c, &crows) == 0,
          "census signal_count failed");
    CHECK(crows == expected_signals,
          "census rows %zu != expected %zu", crows, expected_signals);

    {
        size_t cidx = (size_t)-1;
        for (size_t i = 0; i < crows; i++) {
            /* size query: both path pointers NULL */
            if (census_size_query(c, i) != 0) {
                continue;
            }
            size_t need = 0;
            /* first get the path so we can check it; the size query didn't
             * tell us the path length, so we have to do a full query with
             * a buffer. Simpler: just do the full query once with a
             * reasonably sized buffer. */
            char path[256];
            size_t pathcap = sizeof(path);
            unsigned long long w = 0, cc = 0;
            double f = 0, l = 0;
            int changed = 0;
            int rc = census_full_query(c, i, path, &pathcap, &w, &cc, &f, &l, &changed);
            if (rc != 0) {
                continue;
            }
            if (strcmp(path, signal) == 0) {
                cidx = i;
                CHECK(cc == expected_changes,
                      "%s change_count %llu != expected %llu",
                      signal, cc, expected_changes);
                CHECK(changed == (expected_changes > 0),
                      "%s has_changed mismatch", signal);
            }
        }
        CHECK(cidx != (size_t)-1, "census row for %s not found", signal);
    }

    /* ---- census: negative ---- */
    {
        xezim_trace_census *bad = xezim_trace_census_load("___no_such_file.json");
        CHECK(bad == NULL, "census load of a missing file must return NULL");
        CHECK(xezim_trace_last_error() != NULL,
              "last_error after failed census load must be set");
        size_t n;
        CHECK(xezim_trace_census_signal_count(NULL, &n) == -1,
              "null census handle must be an error");
    }

    xezim_trace_graph_free(g);
    xezim_trace_census_free(c);

    if (failures) {
        fprintf(stderr, "probe: FAILED\n");
        return 1;
    }
    printf("probe: all checks passed\n");
    return 0;
}