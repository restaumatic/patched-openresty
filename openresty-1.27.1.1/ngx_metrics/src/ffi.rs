use std::ffi::{c_char, c_int, c_void};
use std::io::Write;

use crate::{lua, metrics, symbols};

#[allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
pub use bindings::*;

#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut ngx_metric_any_event_handler_time_ns: ngx_metric_t =
    ngx_metric_t::new(c"any_event_handler_time_ns", std::ptr::null());
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut ngx_metric_open_and_stat_file_time_ns: ngx_metric_t =
    ngx_metric_t::new(c"open_and_stat_file_time_ns", std::ptr::null());
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static mut ngx_metric_event_loop_latency_ns: ngx_metric_t =
    ngx_metric_t::new(c"event_loop_latency_ns", std::ptr::null());

fn builtin_metrics() -> [*mut ngx_metric_t; 3] {
    [
        &raw mut ngx_metric_any_event_handler_time_ns,
        &raw mut ngx_metric_open_and_stat_file_time_ns,
        &raw mut ngx_metric_event_loop_latency_ns,
    ]
}

/// # Safety
/// `log` is an nginx log.
#[no_mangle]
pub unsafe extern "C" fn ngx_metrics_init(log: *mut ngx_log_t) {
    let symbols = symbols::load().unwrap_or_else(|e| {
        // SAFETY: `log` is an nginx log (this function's contract).
        unsafe { log_error(log, &e) };
        Default::default()
    });
    let _ = writeln!(std::io::stderr(), "num symbols: {}", symbols.len());
    metrics::init(base_address(), symbols);
}

/// # Safety
/// `metric` is null or points to a valid `ngx_metric_t` that nothing else references during the call.
#[no_mangle]
pub unsafe extern "C" fn ngx_metric_report(metric: *mut ngx_metric_t, value: i64) {
    if !metrics::active() {
        return;
    }
    // SAFETY: this function's contract.
    if let Some(metric) = unsafe { metric.as_mut() } {
        metric.report(value);
    }
}

#[no_mangle]
pub extern "C" fn ngx_get_num_metrics() -> c_int {
    if !metrics::active() {
        return 0;
    }
    (builtin_metrics().len() + metrics::num_dynamic()) as c_int
}

/// The pointer stays valid for the life of the process.
#[no_mangle]
pub extern "C" fn ngx_get_metric(index: c_int) -> *mut ngx_metric_t {
    let builtins = builtin_metrics();
    let index = match usize::try_from(index) {
        Ok(i) if metrics::active() => i,
        _ => return std::ptr::null_mut(),
    };
    match index.checked_sub(builtins.len()) {
        None => builtins[index],
        Some(d) => metrics::dynamic(d).unwrap_or(std::ptr::null_mut()),
    }
}

#[no_mangle]
pub extern "C" fn ngx_metrics_span_enter(handler: *const c_void) {
    metrics::enter_span(handler as usize);
}

#[no_mangle]
pub extern "C" fn ngx_metrics_span_exit() {
    if let Some(wall) = metrics::exit_span() {
        // SAFETY: a builtin metric; `ngx_metric_report` keeps its reference to the call.
        unsafe { ngx_metric_report(&raw mut ngx_metric_any_event_handler_time_ns, wall) };
    }
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

    unsafe { libc::memset(buf as *mut _, 0, bufsize as usize) };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf as *mut u8, bufsize as usize - 1) };
    let mut writer = std::io::Cursor::new(slice);

    // SAFETY: `l` is a live Lua state (this function's contract).
    if let Err(e) = unsafe { lua::dump_stack(l, &mut writer) } {
        match e.kind() {
            std::io::ErrorKind::WriteZero => {
                // Buffer too small, stack trace truncated
                // It's ok
            }
            _ => {
                // TODO: use ngx_log_error
                let _ = writeln!(std::io::stderr(), "ngx_metrics_dump_lua_stack: {}", e);
                writer.into_inner()[0] = 0;
                return 1;
            }
        }
    }

    writer.position() as c_int
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
                *(data as *mut usize) = (*info).dlpi_addr as usize;
                return 1;
            }
        }
        0
    }
    let mut base = 0usize;
    // SAFETY: `callback` matches the expected signature and only writes to `base`.
    unsafe { libc::dl_iterate_phdr(Some(callback), &mut base as *mut usize as *mut c_void) };
    base
}

extern "C" {
    fn ngx_log_error_core(level: usize, log: *mut ngx_log_t, err: c_int, fmt: *const c_char, ...);
}

const NGX_LOG_ERR: usize = 4;

/// # Safety
/// `log` is an nginx log.
unsafe fn log_error(log: *mut ngx_log_t, msg: &str) {
    let msg = std::ffi::CString::new(msg).unwrap_or_default();
    // SAFETY: `log` is an nginx log (this function's contract); `%s` takes a NUL-terminated string.
    unsafe { ngx_log_error_core(NGX_LOG_ERR, log, 0, c"ngx_metrics: %s".as_ptr(), msg.as_ptr()) };
}
