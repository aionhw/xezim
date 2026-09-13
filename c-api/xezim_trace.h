/*
 * xezim_trace.h — C ABI for the xezim trace sidecars.
 *
 * The activity tracer writes two small JSON files at the end of a
 * simulation: the per-signal census (via `--debug-access=+r` or
 * `XEZIM_ACT_TRACE_CENSUS=1`, default `trace_census.json`) and the
 * driver/load graph (via `--debug-signals` `--debug-graph-file` or
 * `--debug+all`, default `trace_graph.json`). This header gives C (and, via ctypes, Python) the
 * same query surface without parsing JSON on the caller's side:
 *
 *     xezim_trace_graph *g = xezim_trace_graph_load("trace_graph.json");
 *     size_t n = 0;
 *     xezim_trace_graph_signal_count(g, &n);           // rows in the graph
 *     size_t need = 0;
 *     xezim_trace_graph_signal(g, 0, NULL, &need);     // two-call string protocol
 *     char *buf = malloc(need);
 *     xezim_trace_graph_signal(g, 0, buf, &need);      // now buf holds the path
 *
 * Every function returns 0 on success and -1 on error (null handle, null
 * output pointer, out-of-range index, buffer too small). On failure, a
 * thread-local message describes the problem:
 *
 *     const char *msg = xezim_trace_last_error();      // NULL if last call succeeded
 *
 * The message stays valid until the next xezim_trace_* call on the same
 * thread.
 *
 * Strings use the standard two-call protocol: pass NULL as the buffer and
 * the address of a size_t to learn the required size (including the NUL),
 * then pass a buffer of that size. Handles are opaque; free them with the
 * matching *_free. All handles may be NULL (a NULL handle is an error, and
 * _free(NULL) is a no-op).
 */

#ifndef XEZIM_TRACE_H
#define XEZIM_TRACE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handles; created by the *_load functions, released by *_free. */
typedef struct xezim_trace_graph xezim_trace_graph;
typedef struct xezim_trace_census xezim_trace_census;

/* ---- Graph (driver/load cones) ---- */

/* Load `path` as a graph sidecar. Returns a handle, or NULL on read/parse
 * error (see xezim_trace_last_error). */
xezim_trace_graph *xezim_trace_graph_load(const char *path);

/* Release a graph handle. NULL is a no-op. */
void xezim_trace_graph_free(xezim_trace_graph *g);

/* Store the number of rows in *out. If the sidecar is missing its row
 * array the count is 0. */
int xezim_trace_graph_signal_count(const xezim_trace_graph *g, size_t *out);

/* Copy the path of row `index` into the caller's buffer (see the two-call
 * protocol above). */
int xezim_trace_graph_signal(const xezim_trace_graph *g, size_t index,
                             char *buf, size_t *cap);

/* Store the number of drivers (fan-in cone members) of row `signal_index`. */
int xezim_trace_graph_driver_count(const xezim_trace_graph *g,
                                   size_t signal_index, size_t *out);

/* Copy the `driver_index`th driver of row `signal_index`. */
int xezim_trace_graph_driver_at(const xezim_trace_graph *g,
                                size_t signal_index, size_t driver_index,
                                char *buf, size_t *cap);

/* Store the number of loads (fan-out cone members) of row `signal_index`. */
int xezim_trace_graph_load_count(const xezim_trace_graph *g,
                                 size_t signal_index, size_t *out);

/* Copy the `load_index`th load of row `signal_index`. */
int xezim_trace_graph_load_at(const xezim_trace_graph *g,
                              size_t signal_index, size_t load_index,
                              char *buf, size_t *cap);

/* ---- Census (per-signal value-change summary) ---- */

/* Load `path` as a census sidecar. Returns a handle, or NULL on read/parse
 * error (see xezim_trace_last_error). */
xezim_trace_census *xezim_trace_census_load(const char *path);

/* Release a census handle. NULL is a no-op. */
void xezim_trace_census_free(xezim_trace_census *c);

/* Store the number of rows in *out. */
int xezim_trace_census_signal_count(const xezim_trace_census *c, size_t *out);

/* Query row `index`. The path is copied into the caller's buffer using the
 * two-call protocol (pass NULL/NULL with both to use them with `path_buf`);
 * every other output pointer may be NULL to skip that field.
 *   *width_out            declared width of the signal in bits
 *   *change_count_out     how many times the signal changed value
 *   *first_change_ns_out  time of the first change, or -1.0
 *   *last_change_ns_out   time of the last change, or -1.0
 *   *has_changed_out      1 if the signal changed at least once, else 0
 * Returns -1 for a bad handle, mismatched path pointer pair, or an
 * out-of-range index. */
int xezim_trace_census_signal_at(const xezim_trace_census *c, size_t index,
                                 char *path_buf, size_t *path_cap,
                                 unsigned long long *width_out,
                                 unsigned long long *change_count_out,
                                 double *first_change_ns_out,
                                 double *last_change_ns_out,
                                 int *has_changed_out);

/* ---- Diagnostics ---- */

/* Thread-local message from the last xezim_trace_* call on this thread, or
 * NULL if that call succeeded. Valid until the next xezim_trace_* call on
 * the same thread. */
const char *xezim_trace_last_error(void);

#ifdef __cplusplus
}
#endif

#endif /* XEZIM_TRACE_H */