use std::ffi::{c_char, c_int, c_void, CString};
use std::io::Write;

use crate::{lua, metrics, symbols};

#[allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code, clippy::all, clippy::pedantic)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
pub use bindings::*;

const _: () = assert!(
    ngx_metric_e_NGX_METRIC_ANY_EVENT_HANDLER_TIME_NS == 0
        && ngx_metric_e_NGX_METRIC_OPEN_AND_STAT_FILE_TIME_NS == 1
        && ngx_metric_e_NGX_METRIC_EVENT_LOOP_LATENCY_NS == 2
        && metrics::BUILTIN_NAMES.len() == 3
);

/// # Safety
/// `log` is an nginx log.
#[no_mangle]
pub unsafe extern "C" fn ngx_metrics_init(log: *mut ngx_log_t) {
    let symbols = symbols::load().unwrap_or_else(|e| {
        // SAFETY: `log` is an nginx log (this function's contract).
        unsafe { log_error(log, &e) };
        std::collections::HashMap::default()
    });
    let _ = writeln!(std::io::stderr(), "num symbols: {}", symbols.len());
    metrics::init(base_address(), symbols);
}

#[no_mangle]
pub extern "C" fn ngx_metric_report(metric: ngx_metric_e, value: i64) {
    metrics::report_builtin(metric as usize, value);
}

#[no_mangle]
pub extern "C" fn ngx_metrics_prometheus(reset: c_int) -> *mut c_char {
    metrics::prometheus(reset != 0)
        .and_then(|s| CString::new(s).ok())
        .map_or(std::ptr::null_mut(), CString::into_raw)
}

/// # Safety
/// `metrics` is null or a not yet freed result of `ngx_metrics_prometheus`.
#[no_mangle]
pub unsafe extern "C" fn ngx_metrics_free_result(metrics: *mut c_char) {
    if !metrics.is_null() {
        // SAFETY: this function's contract.
        drop(unsafe { CString::from_raw(metrics) });
    }
}

#[no_mangle]
pub extern "C" fn ngx_metrics_span_enter(handler: *const c_void) {
    metrics::enter_span(handler as usize);
}

#[no_mangle]
pub extern "C" fn ngx_metrics_span_exit() {
    metrics::exit_span();
}

#[no_mangle]
pub extern "C" fn ngx_precise_time() -> i64 {
    wall_time_ns()
}

/// # Safety
/// `l` is a live Lua state, and `buf` is writable for `bufsize` bytes.
#[no_mangle]
pub unsafe extern "C" fn ngx_metrics_dump_lua_stack(
    l: *mut lua_State,
    buf: *mut c_char,
    bufsize: c_int,
) -> c_int {
    assert!(bufsize > 0);

    let len = bufsize.unsigned_abs() as usize;
    unsafe { libc::memset(buf.cast(), 0, len) };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), len - 1) };
    let mut writer = std::io::Cursor::new(slice);

    // SAFETY: `l` is a live Lua state (this function's contract).
    if let Err(e) = unsafe { lua::dump_stack(l, &mut writer) } {
        // WriteZero: buffer too small, stack trace truncated. It's ok
        if e.kind() != std::io::ErrorKind::WriteZero {
            // TODO: use ngx_log_error
            let _ = writeln!(std::io::stderr(), "ngx_metrics_dump_lua_stack: {e}");
            writer.into_inner()[0] = 0;
            return 1;
        }
    }

    c_int::try_from(writer.position()).unwrap_or(bufsize - 1)
}

pub fn wall_time_ns() -> i64 {
    clock_ns(libc::CLOCK_MONOTONIC)
}

pub fn cpu_time_ns() -> i64 {
    clock_ns(libc::CLOCK_THREAD_CPUTIME_ID)
}

fn clock_ns(clock: libc::clockid_t) -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid out-pointer.
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

fn base_address() -> usize {
    unsafe extern "C" fn callback(
        info: *mut libc::dl_phdr_info,
        _size: usize,
        data: *mut c_void,
    ) -> c_int {
        // SAFETY: glibc passes a valid `info`; `data` is the `usize` below.
        unsafe {
            if *(*info).dlpi_name == 0 {
                *data.cast::<usize>() = usize::try_from((*info).dlpi_addr).unwrap_or(0);
                return 1;
            }
        }
        0
    }
    let mut base = 0usize;
    // SAFETY: `callback` matches the expected signature and only writes to `base`.
    unsafe { libc::dl_iterate_phdr(Some(callback), (&raw mut base).cast()) };
    base
}

extern "C" {
    fn ngx_log_error_core(level: usize, log: *mut ngx_log_t, err: c_int, fmt: *const c_char, ...);
}

const NGX_LOG_ERR: usize = 4;

/// # Safety
/// `log` is an nginx log.
unsafe fn log_error(log: *mut ngx_log_t, msg: &str) {
    let msg = CString::new(msg).unwrap_or_default();
    // SAFETY: `log` is an nginx log (this function's contract); `%s` takes a NUL-terminated string.
    unsafe { ngx_log_error_core(NGX_LOG_ERR, log, 0, c"ngx_metrics: %s".as_ptr(), msg.as_ptr()) };
}
