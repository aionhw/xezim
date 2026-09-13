//! C ABI for the trace sidecars.
//!
//! Both trace sidecars are plain JSON on disk, so any tool that can read a
//! file can consume them. This module exposes the same query surface to C
//! (and, through ctypes, to Python) without any JSON parsing on the caller's
//! side: load a sidecar into an opaque handle, then ask for counts and for
//! strings by index. Strings use the caller-provided-buffer convention
//! described in `c-api/xezim_trace.h`.
//!
//! Every call is safe against null/invalid handles and out-of-range indexes:
//! it reports `-1` and a thread-local message instead of crashing. No panic
//! escapes across the boundary.
//!
//! Layouts: the census and graph structs are opaque. The header in
//! `c-api/xezim_trace.h` is the normative contract; these functions match it
//! exactly.
//!
//! # Safety
//!
//! All `xezim_trace_*` functions that accept raw pointers (`*const T` or
//! `*mut T`) are `unsafe extern "C"`: the caller must provide valid,
//! non-dangling pointers and respect the NUL-termination and buffer-size
//! contracts documented per function. Null-handle and null-output-pointer
//! cases are handled internally and return `-1` rather than UB.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_ulonglong};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::compiler::act_trace::TraceCensus;
use crate::compiler::act_trace::TraceGraph;

/// Opaque graph handle. The C header names the same type `xezim_trace_graph`;
/// the tag is not part of the ABI, so the Rust spelling can stay idiomatic.
#[repr(C)]
pub struct CTraceGraph {
    _graph: TraceGraph,
}

/// Opaque census handle (`xezim_trace_census` in the header).
#[repr(C)]
pub struct CTraceCensus {
    _census: TraceCensus,
}

// Thread-local message from the last failed call, if any. Cleared at the
// entry of every `xezim_trace_*` call, so it always reflects the most
// recent call.
thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Record a failure message and return the standard `-1` error code.
fn set_err(msg: impl AsRef<str>) -> c_int {
    let s = CString::new(msg.as_ref()).unwrap_or_else(|_| CString::new("error").unwrap());
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(s));
    -1
}

/// Reset the thread-local error slot at the entry of a public call.
fn clear_err() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// Fill a caller buffer with `text` (NUL-terminated).
///
/// * `cap` is an in/out size: on entry the caller's buffer capacity
///   (including room for the NUL), on return the size that was needed.
/// * `dst == NULL` asks only how much to allocate: the required size is
///   stored in `cap` and the call returns 0.
/// * If the buffer is too small, the required size is stored and `-1` is
///   returned with an error message.
///
/// # Safety
///
/// `cap` must be non-null. If `dst` is non-null, it must point to at least
/// `*cap` writable bytes.
unsafe fn fill_string(dst: *mut c_char, cap: *mut usize, text: &str) -> c_int {
    if cap.is_null() {
        return set_err("null capacity pointer");
    }
    let need = text.len().saturating_add(1);
    if dst.is_null() {
        unsafe { *cap = need };
        return 0;
    }
    if unsafe { *cap } < need {
        unsafe { *cap = need };
        return set_err("buffer too small");
    }
    // SAFETY: the caller guaranteed `*cap >= need` and `dst` writable for
    // `need` bytes; `text` is a valid Rust string.
    unsafe {
        ptr::copy_nonoverlapping(text.as_ptr(), dst.cast::<u8>(), text.len());
        *dst.add(text.len()) = 0;
        *cap = need;
    }
    0
}

/// Borrow a library-owned handle as `&T`, reporting a null-handle error
/// instead of dereferencing NULL.
fn as_ref_or_err<'a, T>(p: *const T) -> Result<&'a T, c_int> {
    if p.is_null() {
        Err(set_err("null handle"))
    } else {
        // SAFETY: the caller passed a non-null handle this library created.
        Ok(unsafe { &*p })
    }
}

/// Copy the `idx`th string out of `list`, or report an error.
fn copy_list_at(list: &[String], idx: usize, buf: *mut c_char, cap: *mut usize) -> c_int {
    match list.get(idx) {
        Some(s) => unsafe { fill_string(buf, cap, s) },
        None => set_err(format!("item index {idx} out of range")),
    }
}

/// Copy a signal path by row index via the two-call buffer convention.
fn graph_signal_at(g: &TraceGraph, idx: usize, buf: *mut c_char, cap: *mut usize) -> c_int {
    match g.signals.get(idx) {
        Some(row) => unsafe { fill_string(buf, cap, &row.path) },
        None => set_err(format!("signal index {idx} out of range")),
    }
}

/// Load a sidecar file. Shared by the graph and census loaders: the parse is
/// wrapped so a panic in the reader surfaces as an error message, not a crash.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated C string, or null (which is handled
/// as an error).
fn load_sidecar<T>(
    path: *const c_char,
    read: impl FnOnce(&std::path::Path) -> Result<T, String>,
) -> Result<T, c_int> {
    clear_err();
    if path.is_null() {
        return Err(set_err("null path"));
    }
    let path_str = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return Err(set_err("path is not valid UTF-8")),
    };
    catch_unwind(AssertUnwindSafe(|| read(std::path::Path::new(path_str))))
        .unwrap_or_else(|_| Err("panic while parsing sidecar".to_string()))
        .map_err(set_err)
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// Load `path` as a driver/load graph sidecar. Returns a new handle to free
/// with `xezim_trace_graph_free`, or NULL on read/parse error.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated C string, or null (which returns
/// NULL with an error).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_load(path: *const c_char) -> *mut CTraceGraph {
    match load_sidecar(path, TraceGraph::read) {
        Ok(g) => Box::into_raw(Box::new(CTraceGraph { _graph: g })),
        Err(_) => ptr::null_mut(),
    }
}

/// Release a graph handle. NULL is a no-op.
///
/// # Safety
///
/// `g` must be a handle previously returned by `xezim_trace_graph_load`, or
/// null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_free(g: *mut CTraceGraph) {
    if !g.is_null() {
        // SAFETY: this handle came from `xezim_trace_graph_load`.
        unsafe { drop(Box::from_raw(g)) };
    }
}

/// Store the number of graph rows in `out`. Returns 0, or -1 on a null
/// handle or null `out`.
///
/// # Safety
///
/// `g` must be a valid graph handle; `out` must be non-null and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_signal_count(
    g: *const CTraceGraph,
    out: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(g) {
        Ok(g) if !out.is_null() => {
            unsafe { *out = g._graph.signals.len() };
            0
        }
        Ok(_) => set_err("null out pointer"),
        Err(e) => e,
    }
}

/// Copy the path of row `index` into the caller's buffer.
///
/// # Safety
///
/// `g` must be a valid graph handle. If `buf` is non-null, `cap` must also
/// be non-null and `*cap` must reflect the buffer capacity.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_signal(
    g: *const CTraceGraph,
    index: usize,
    buf: *mut c_char,
    cap: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(g) {
        Ok(g) => graph_signal_at(&g._graph, index, buf, cap),
        Err(e) => e,
    }
}

/// Store the driver count of row `signal_index` in `out`.
///
/// # Safety
///
/// `g` must be a valid graph handle; `out` must be non-null and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_driver_count(
    g: *const CTraceGraph,
    signal_index: usize,
    out: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(g) {
        Ok(g) => match g._graph.signals.get(signal_index) {
            Some(row) if !out.is_null() => {
                unsafe { *out = row.drivers.len() };
                0
            }
            Some(_) => set_err("null out pointer"),
            None => set_err(format!("signal index {} out of range", signal_index)),
        },
        Err(e) => e,
    }
}

/// Copy the `driver_index`th driver of row `signal_index`.
///
/// # Safety
///
/// `g` must be a valid graph handle. If `buf` is non-null, `cap` must also
/// be non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_driver_at(
    g: *const CTraceGraph,
    signal_index: usize,
    driver_index: usize,
    buf: *mut c_char,
    cap: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(g) {
        Ok(g) => match g._graph.signals.get(signal_index) {
            Some(row) => copy_list_at(&row.drivers, driver_index, buf, cap),
            None => set_err(format!("signal index {} out of range", signal_index)),
        },
        Err(e) => e,
    }
}

/// Store the load count of row `signal_index` in `out`.
///
/// # Safety
///
/// `g` must be a valid graph handle; `out` must be non-null and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_load_count(
    g: *const CTraceGraph,
    signal_index: usize,
    out: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(g) {
        Ok(g) => match g._graph.signals.get(signal_index) {
            Some(row) if !out.is_null() => {
                unsafe { *out = row.loads.len() };
                0
            }
            Some(_) => set_err("null out pointer"),
            None => set_err(format!("signal index {} out of range", signal_index)),
        },
        Err(e) => e,
    }
}

/// Copy the `load_index`th load of row `signal_index`.
///
/// # Safety
///
/// `g` must be a valid graph handle. If `buf` is non-null, `cap` must also
/// be non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_graph_load_at(
    g: *const CTraceGraph,
    signal_index: usize,
    load_index: usize,
    buf: *mut c_char,
    cap: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(g) {
        Ok(g) => match g._graph.signals.get(signal_index) {
            Some(row) => copy_list_at(&row.loads, load_index, buf, cap),
            None => set_err(format!("signal index {} out of range", signal_index)),
        },
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------
// Census
// ---------------------------------------------------------------------------

/// Load `path` as a census sidecar. Returns a new handle to free with
/// `xezim_trace_census_free`, or NULL on read/parse error.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated C string, or null (which returns
/// NULL with an error).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_census_load(path: *const c_char) -> *mut CTraceCensus {
    match load_sidecar(path, TraceCensus::read) {
        Ok(c) => Box::into_raw(Box::new(CTraceCensus { _census: c })),
        Err(_) => ptr::null_mut(),
    }
}

/// Release a census handle. NULL is a no-op.
///
/// # Safety
///
/// `c` must be a handle previously returned by `xezim_trace_census_load`, or
/// null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_census_free(c: *mut CTraceCensus) {
    if !c.is_null() {
        // SAFETY: this handle came from `xezim_trace_census_load`.
        unsafe { drop(Box::from_raw(c)) };
    }
}

/// Store the number of census rows in `out`. Returns 0, or -1 on a null
/// handle or null `out`.
///
/// # Safety
///
/// `c` must be a valid census handle; `out` must be non-null and writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_trace_census_signal_count(
    c: *const CTraceCensus,
    out: *mut usize,
) -> c_int {
    clear_err();
    match as_ref_or_err(c) {
        Ok(c) if !out.is_null() => {
            unsafe { *out = c._census.rows.len() };
            0
        }
        Ok(_) => set_err("null out pointer"),
        Err(e) => e,
    }
}

/// Query one census row: its path (into the caller's buffer) and its scalar
/// fields. Any output pointer except the path buffer/cap pair may be NULL to
/// skip that field. `has_changed_out` is 1 when the signal changed at least
/// once; `first_change_ns`/`last_change_ns` are -1.0 for never-changed
/// signals. Returns 0, or -1 on a null handle, bad path pointer, or
/// out-of-range index.
///
/// # Safety
///
/// `c` must be a valid census handle. If `path_buf` is non-null, `path_cap`
/// must also be non-null; if `path_buf` is null, `path_cap` must also be null.
/// Any non-null output pointer must be writable.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)] // fixed cell-query signature the header defines
pub unsafe extern "C" fn xezim_trace_census_signal_at(
    c: *const CTraceCensus,
    index: usize,
    path_buf: *mut c_char,
    path_cap: *mut usize,
    width_out: *mut c_ulonglong,
    change_count_out: *mut c_ulonglong,
    first_change_ns_out: *mut c_double,
    last_change_ns_out: *mut c_double,
    has_changed_out: *mut c_int,
) -> c_int {
    clear_err();
    let crowd = match as_ref_or_err(c) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let row = match crowd._census.rows.get(index) {
        Some(r) => r,
        None => return set_err(format!("census index {} out of range", index)),
    };
    if (path_buf.is_null()) != (path_cap.is_null()) {
        return set_err("path_buf and path_cap must both be provided, or both NULL");
    }
    if !path_buf.is_null() && unsafe { fill_string(path_buf, path_cap, &row.path) } != 0 {
        return -1;
    }
    if !width_out.is_null() {
        unsafe { *width_out = row.width as c_ulonglong };
    }
    if !change_count_out.is_null() {
        unsafe { *change_count_out = row.change_count as c_ulonglong };
    }
    if !first_change_ns_out.is_null() {
        unsafe { *first_change_ns_out = row.first_change_s.map(|s| s * 1e9).unwrap_or(-1.0) };
    }
    if !last_change_ns_out.is_null() {
        unsafe { *last_change_ns_out = row.last_change_s.map(|s| s * 1e9).unwrap_or(-1.0) };
    }
    if !has_changed_out.is_null() {
        unsafe { *has_changed_out = if row.change_count > 0 { 1 } else { 0 } };
    }
    0
}

/// Thread-local message from the last `xezim_trace_*` call on this thread, or
/// NULL if that call succeeded. Valid until the next `xezim_trace_*` call on
/// the same thread.
#[unsafe(no_mangle)]
pub extern "C" fn xezim_trace_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| match &*slot.borrow() {
        Some(s) => s.as_ptr(),
        None => ptr::null(),
    })
}
