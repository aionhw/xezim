#!/usr/bin/env python3
"""
xezim_trace_probe.py — Python ctypes probe for the xezim_trace C ABI.

Usage:
  python3 xezim_trace_probe.py GRAPH CENSUS SIGNAL DRIVER LOAD \
      EXPECTED_SIGNALS EXPECTED_CHANGES

Runs the same positive/negative checks as the C probe but via ctypes,
verifying the cross-language surface works.
"""

import sys
import os
import ctypes
import pathlib

# Load libxezim.so next to this script's run directory.
HERE = pathlib.Path(__file__).resolve().parent
LIB_DIR = HERE.parent / "target" / "debug"
LIB = ctypes.CDLL(str(LIB_DIR / "libxezim.so"))

# --- ctypes bindings ---------------------------------------------------------

# xezim_trace_graph *xezim_trace_graph_load(const char *path)
xezim_trace_graph_load = LIB.xezim_trace_graph_load
xezim_trace_graph_load.argtypes = [ctypes.c_char_p]
xezim_trace_graph_load.restype = ctypes.c_void_p

# void xezim_trace_graph_free(xezim_trace_graph *g)
xezim_trace_graph_free = LIB.xezim_trace_graph_free
xezim_trace_graph_free.argtypes = [ctypes.c_void_p]
xezim_trace_graph_free.restype = None

# int xezim_trace_graph_signal_count(const xezim_trace_graph *g, size_t *out)
xezim_trace_graph_signal_count = LIB.xezim_trace_graph_signal_count
xezim_trace_graph_signal_count.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t)]
xezim_trace_graph_signal_count.restype = ctypes.c_int

# int xezim_trace_graph_signal(const xezim_trace_graph *g, size_t index,
#                              char *buf, size_t *cap)
xezim_trace_graph_signal = LIB.xezim_trace_graph_signal
xezim_trace_graph_signal.argtypes = [
    ctypes.c_void_p, ctypes.c_size_t,
    ctypes.c_char_p, ctypes.POINTER(ctypes.c_size_t),
]
xezim_trace_graph_signal.restype = ctypes.c_int

# int xezim_trace_graph_driver_count(const xezim_trace_graph *g,
#                                    size_t signal_index, size_t *out)
xezim_trace_graph_driver_count = LIB.xezim_trace_graph_driver_count
xezim_trace_graph_driver_count.argtypes = [
    ctypes.c_void_p, ctypes.c_size_t,
    ctypes.POINTER(ctypes.c_size_t),
]
xezim_trace_graph_driver_count.restype = ctypes.c_int

# int xezim_trace_graph_driver_at(const xezim_trace_graph *g,
#                                 size_t signal_index, size_t driver_index,
#                                 char *buf, size_t *cap)
xezim_trace_graph_driver_at = LIB.xezim_trace_graph_driver_at
xezim_trace_graph_driver_at.argtypes = [
    ctypes.c_void_p, ctypes.c_size_t, ctypes.c_size_t,
    ctypes.c_char_p, ctypes.POINTER(ctypes.c_size_t),
]
xezim_trace_graph_driver_at.restype = ctypes.c_int

# int xezim_trace_graph_load_count(const xezim_trace_graph *g,
#                                  size_t signal_index, size_t *out)
xezim_trace_graph_load_count = LIB.xezim_trace_graph_load_count
xezim_trace_graph_load_count.argtypes = [
    ctypes.c_void_p, ctypes.c_size_t,
    ctypes.POINTER(ctypes.c_size_t),
]
xezim_trace_graph_load_count.restype = ctypes.c_int

# int xezim_trace_graph_load_at(const xezim_trace_graph *g,
#                               size_t signal_index, size_t load_index,
#                               char *buf, size_t *cap)
xezim_trace_graph_load_at = LIB.xezim_trace_graph_load_at
xezim_trace_graph_load_at.argtypes = [
    ctypes.c_void_p, ctypes.c_size_t, ctypes.c_size_t,
    ctypes.c_char_p, ctypes.POINTER(ctypes.c_size_t),
]
xezim_trace_graph_load_at.restype = ctypes.c_int

# ---- Census ----

# xezim_trace_census *xezim_trace_census_load(const char *path)
xezim_trace_census_load = LIB.xezim_trace_census_load
xezim_trace_census_load.argtypes = [ctypes.c_char_p]
xezim_trace_census_load.restype = ctypes.c_void_p

# void xezim_trace_census_free(xezim_trace_census *c)
xezim_trace_census_free = LIB.xezim_trace_census_free
xezim_trace_census_free.argtypes = [ctypes.c_void_p]
xezim_trace_census_free.restype = None

# int xezim_trace_census_signal_count(const xezim_trace_census *c, size_t *out)
xezim_trace_census_signal_count = LIB.xezim_trace_census_signal_count
xezim_trace_census_signal_count.argtypes = [
    ctypes.c_void_p, ctypes.POINTER(ctypes.c_size_t),
]
xezim_trace_census_signal_count.restype = ctypes.c_int

# int xezim_trace_census_signal_at(const xezim_trace_census *c, size_t index,
#                                  char *path_buf, size_t *path_cap,
#                                  unsigned long long *width_out,
#                                  unsigned long long *change_count_out,
#                                  double *first_change_ns_out,
#                                  double *last_change_ns_out,
#                                  int *has_changed_out)
xezim_trace_census_signal_at = LIB.xezim_trace_census_signal_at
xezim_trace_census_signal_at.argtypes = [
    ctypes.c_void_p, ctypes.c_size_t,
    ctypes.c_char_p, ctypes.POINTER(ctypes.c_size_t),
    ctypes.POINTER(ctypes.c_ulonglong),
    ctypes.POINTER(ctypes.c_ulonglong),
    ctypes.POINTER(ctypes.c_double),
    ctypes.POINTER(ctypes.c_double),
    ctypes.POINTER(ctypes.c_int),
]
xezim_trace_census_signal_at.restype = ctypes.c_int

# const char *xezim_trace_last_error(void)
xezim_trace_last_error = LIB.xezim_trace_last_error
xezim_trace_last_error.argtypes = []
xezim_trace_last_error.restype = ctypes.c_char_p

# --- Helpers ----------------------------------------------------------------

def get_string1(fn, handle, index):
    """Two-call protocol for single-index functions."""
    need = ctypes.c_size_t(0)
    if fn(handle, index, None, ctypes.byref(need)) != 0 or need.value == 0:
        return None
    buf = ctypes.create_string_buffer(need.value)
    if fn(handle, index, buf, ctypes.byref(need)) != 0:
        return None
    return buf.value.decode("utf-8")

def get_string2(fn, handle, idx1, idx2):
    """Two-call protocol for two-index functions (driver_at, load_at)."""
    need = ctypes.c_size_t(0)
    if fn(handle, idx1, idx2, None, ctypes.byref(need)) != 0 or need.value == 0:
        return None
    buf = ctypes.create_string_buffer(need.value)
    if fn(handle, idx1, idx2, buf, ctypes.byref(need)) != 0:
        return None
    return buf.value.decode("utf-8")

def last_err():
    p = xezim_trace_last_error()
    return p.decode("utf-8") if p else None

_failures = 0

def check(cond, msg):
    global _failures
    if not cond:
        print(f"FAIL: {msg}", file=sys.stderr)
        _failures += 1
        return False
    return True

def main():
    if len(sys.argv) != 8:
        print(
            "usage: python3 xezim_trace_probe.py GRAPH CENSUS SIGNAL DRIVER "
            "LOAD EXPECTED_SIGNALS EXPECTED_CHANGES",
            file=sys.stderr,
        )
        return 2

    graph_path = sys.argv[1]
    census_path = sys.argv[2]
    signal = sys.argv[3]
    driver = sys.argv[4]
    load = sys.argv[5]
    expected_signals = int(sys.argv[6])
    expected_changes = int(sys.argv[7])

    # ---- graph: positive ----
    g = xezim_trace_graph_load(graph_path.encode("utf-8"))
    if not check(g is not None, "graph load failed"):
        return 1

    rows = ctypes.c_size_t(0)
    if not check(xezim_trace_graph_signal_count(g, ctypes.byref(rows)) == 0,
                 "signal_count failed"):
        return 1
    if not check(rows.value == expected_signals,
                 f"signal_count {rows.value} != expected {expected_signals}"):
        return 1

    signal_index = None
    for i in range(rows.value):
        path = get_string1(xezim_trace_graph_signal, g, i)
        if path and path == signal:
            signal_index = i
            break
    if not check(signal_index is not None,
                 f"row for {signal} not found"):
        return 1

    # driver must appear somewhere among the row's drivers
    dc = ctypes.c_size_t(0)
    if not check(xezim_trace_graph_driver_count(g, signal_index, ctypes.byref(dc)) == 0,
                 "driver_count failed"):
        return 1
    found = False
    for i in range(dc.value):
        d = get_string2(xezim_trace_graph_driver_at, g, signal_index, i)
        if d == driver:
            found = True
            break
    if not check(found, f"{signal} does not list {driver} among its drivers"):
        return 1

    # load must appear somewhere among the row's loads
    lc = ctypes.c_size_t(0)
    if not check(xezim_trace_graph_load_count(g, signal_index, ctypes.byref(lc)) == 0,
                 "load_count failed"):
        return 1
    found = False
    for i in range(lc.value):
        l = get_string2(xezim_trace_graph_load_at, g, signal_index, i)
        if l == load:
            found = True
            break
    if not check(found, f"{signal} does not list {load} among its loads"):
        return 1

    # ---- graph: negative ----
    bad = xezim_trace_graph_load(b"___no_such_file.json")
    if not check(bad is None, "loading a missing file must return NULL"):
        return 1
    if not check(last_err() is not None and len(last_err()) > 0,
                 "last_error after a failed load must be non-empty"):
        return 1

    n = ctypes.c_size_t(0)
    if not check(xezim_trace_graph_signal_count(None, ctypes.byref(n)) == -1,
                 "null handle must be an error"):
        return 1
    if not check(last_err() is not None,
                 "last_error after null-handle call must be set"):
        return 1

    if not check(xezim_trace_graph_signal(g, rows.value + 5, None, ctypes.byref(n)) == -1,
                 "out-of-range index must be an error"):
        return 1
    if not check(last_err() is not None,
                 "last_error after out-of-range call must be set"):
        return 1

    tiny = ctypes.create_string_buffer(1)
    tinycap = ctypes.c_size_t(len(tiny))
    if not check(xezim_trace_graph_signal(g, signal_index, tiny, ctypes.byref(tinycap)) == -1,
                 "too-small buffer must be an error"):
        return 1
    if not check(tinycap.value > len(tiny),
                 "too-small buffer must report the required size"):
        return 1

    # ---- census: positive ----
    c = xezim_trace_census_load(census_path.encode("utf-8"))
    if not check(c is not None, "census load failed"):
        return 1

    crows = ctypes.c_size_t(0)
    if not check(xezim_trace_census_signal_count(c, ctypes.byref(crows)) == 0,
                 "census signal_count failed"):
        return 1
    if not check(crows.value == expected_signals,
                 f"census rows {crows.value} != expected {expected_signals}"):
        return 1

    cidx = None
    for i in range(crows.value):
        # size query: both path pointers NULL
        if xezim_trace_census_signal_at(c, i, None, None, None, None,
                                        None, None, None) != 0:
            continue
        # full query with a fixed buffer (simple approach)
        path = ctypes.create_string_buffer(256)
        pathcap = ctypes.c_size_t(len(path))
        w = ctypes.c_ulonglong(0)
        cc = ctypes.c_ulonglong(0)
        f = ctypes.c_double(0.0)
        l = ctypes.c_double(0.0)
        changed = ctypes.c_int(0)
        if xezim_trace_census_signal_at(c, i, path, ctypes.byref(pathcap),
                                        ctypes.byref(w), ctypes.byref(cc),
                                        ctypes.byref(f), ctypes.byref(l),
                                        ctypes.byref(changed)) != 0:
            continue
        if path.value.decode("utf-8") == signal:
            cidx = i
            if not check(cc.value == expected_changes,
                         f"{signal} change_count {cc.value} != expected {expected_changes}"):
                return 1
            if not check(changed.value == (1 if expected_changes > 0 else 0),
                         f"{signal} has_changed mismatch"):
                return 1

    if not check(cidx is not None, f"census row for {signal} not found"):
        return 1

    # ---- census: negative ----
    bad = xezim_trace_census_load(b"___no_such_file.json")
    if not check(bad is None, "census load of missing file must return NULL"):
        return 1
    if not check(last_err() is not None,
                 "last_error after failed census load must be set"):
        return 1
    if not check(xezim_trace_census_signal_count(None, ctypes.byref(n)) == -1,
                 "null census handle must be an error"):
        return 1

    xezim_trace_graph_free(g)
    xezim_trace_census_free(c)

    if _failures:
        print(f"probe: FAILED with {_failures} failed checks", file=sys.stderr)
        return 1
    print("probe: all checks passed")
    return 0

if __name__ == "__main__":
    sys.exit(main())