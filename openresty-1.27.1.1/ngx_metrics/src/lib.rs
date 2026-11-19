#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::cell::{Cell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::Write;
use std::mem::MaybeUninit;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[no_mangle]
pub extern "C" fn ngx_metrics_dump_lua_stack(
    l: *mut lua_State,
    buf: *mut c_char,
    bufsize: c_int,
) -> c_int {
    assert!(bufsize > 0);

    unsafe { libc::memset(buf as *mut _, 0, bufsize as usize) };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf as *mut u8, bufsize as usize - 1) };
    let mut writer = std::io::Cursor::new(slice);

    if let Err(e) = dump_stack(l, &mut writer) {
        match e.kind() {
            std::io::ErrorKind::WriteZero => {
                // Buffer too small, stack trace truncated
                // It's ok
            }
            _ => {
                // TODO: use ngx_log_error
                eprintln!("ngx_metrics_dump_lua_stack: {}", e);
                writer.into_inner()[0] = 0;
                return 1;
            }
        }
    }

    writer.position() as c_int
}

fn dump_stack(l: *mut lua_State, mut writer: impl Write) -> std::io::Result<()> {
    // Translate the above C to Rust
    let mut level = 0;
    loop {
        let mut ar_u: MaybeUninit<lua_Debug> = MaybeUninit::uninit();
        if unsafe { lua_getstack(l, level, ar_u.as_mut_ptr()) } == 0 {
            break;
        }
        if unsafe { lua_getinfo(l, c"Sln".as_ptr(), ar_u.as_mut_ptr()) } == 0 {
            break;
        }
        let ar = unsafe { ar_u.assume_init() };
        let name = if ar.name.is_null() {
            "[anonymous]"
        } else {
            unsafe { std::ffi::CStr::from_ptr(ar.name) }
                .to_str()
                .unwrap_or("[invalid utf8]")
        };
        let source = unsafe { std::ffi::CStr::from_ptr(ar.source) }
            .to_str()
            .unwrap_or("[invalid utf8]");
        if source == "=[C]" {
            level += 1;
            continue;
        }
        let line = ar.currentline;
        write!(
            writer,
            "{}{}@{}:{}",
            if level == 0 { "" } else { " " },
            name,
            get_filename(source),
            line
        )?;
        level += 1;
    }
    Ok(())
}

fn get_filename(source: &str) -> &str {
    if source == "=[C]" {
        return "[C]";
    }
    source.rsplitn(2, '/').next().unwrap_or(source)
}

const MAX_DYNAMIC_METRICS: usize = 128 * 4;

const SPAN_METRIC_NAMES: [&CStr; 4] = [
    c"event_handler_total_wall_time_ns",
    c"event_handler_self_wall_time_ns",
    c"event_handler_total_cpu_time_ns",
    c"event_handler_self_cpu_time_ns",
];

#[no_mangle]
pub static mut ngx_metric_any_event_handler_time_ns: ngx_metric_t =
    ngx_metric_t::new(c"any_event_handler_time_ns", std::ptr::null());
#[no_mangle]
pub static mut ngx_metric_open_and_stat_file_time_ns: ngx_metric_t =
    ngx_metric_t::new(c"open_and_stat_file_time_ns", std::ptr::null());
#[no_mangle]
pub static mut ngx_metric_event_loop_latency_ns: ngx_metric_t =
    ngx_metric_t::new(c"event_loop_latency_ns", std::ptr::null());

fn builtin_metrics() -> [*mut ngx_metric_t; 3] {
    [
        &raw mut ngx_metric_any_event_handler_time_ns,
        &raw mut ngx_metric_open_and_stat_file_time_ns,
        &raw mut ngx_metric_event_loop_latency_ns,
    ]
}

impl ngx_metric_t {
    const fn new(name: &'static CStr, tags: *const c_char) -> Self {
        Self { name: name.as_ptr(), tags, count: 0, sum: 0, min: 0, max: 0 }
    }

    fn report(&mut self, value: i64) {
        if self.count == 0 {
            self.min = value;
            self.max = value;
        } else {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
        self.count += 1;
        self.sum += value;
    }
}

struct Symbol {
    tags: CString,
    metrics: Option<[usize; 4]>,
}

struct Span {
    handler: usize,
    start_wall: i64,
    start_cpu: i64,
    children_wall: i64,
    children_cpu: i64,
}

struct State {
    base_address: usize,
    symbols: HashMap<usize, Symbol>,
    dynamic: Vec<ngx_metric_t>,
    spans: Vec<Span>,
}

struct Global(UnsafeCell<Option<State>>);
// SAFETY: accessed only through `with_state`, which admits only the main thread.
unsafe impl Sync for Global {}

static STATE: Global = Global(UnsafeCell::new(None));

thread_local! {
    static ON_MAIN_THREAD: Cell<bool> = const { Cell::new(false) };
}

fn on_main_thread() -> bool {
    ON_MAIN_THREAD.get()
}

/// `f` must not call back into anything that uses `with_state`.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    if !on_main_thread() {
        return None;
    }
    // SAFETY: single thread (checked above), and `f` doesn't re-enter.
    unsafe { (*STATE.0.get()).as_mut() }.map(f)
}

impl State {
    fn exit_span(&mut self) {
        let Some(span) = self.spans.pop() else { return };
        let wall = clock_ns(libc::CLOCK_MONOTONIC) - span.start_wall;
        let cpu = clock_ns(libc::CLOCK_THREAD_CPUTIME_ID) - span.start_cpu;
        if let Some(parent) = self.spans.last_mut() {
            parent.children_wall += wall;
            parent.children_cpu += cpu;
        }
        ngx_metric_report(&raw mut ngx_metric_any_event_handler_time_ns, wall);
        self.report_handler(
            span.handler.wrapping_sub(self.base_address),
            [wall, wall - span.children_wall, cpu, cpu - span.children_cpu],
        );
    }

    fn report_handler(&mut self, relative_addr: usize, values: [i64; 4]) {
        let Some(sym) = self.symbols.get_mut(&relative_addr) else { return };
        let ids = match sym.metrics {
            Some(ids) => ids,
            None => {
                if self.dynamic.len() + 4 > MAX_DYNAMIC_METRICS {
                    return;
                }
                let ids = SPAN_METRIC_NAMES.map(|name| {
                    self.dynamic.push(ngx_metric_t::new(name, sym.tags.as_ptr()));
                    self.dynamic.len() - 1
                });
                *sym.metrics.insert(ids)
            }
        };
        for (id, value) in ids.into_iter().zip(values) {
            self.dynamic[id].report(value);
        }
    }
}

#[no_mangle]
pub extern "C" fn ngx_metrics_init(log: *mut ngx_log_t) {
    let symbols = load_symbols().unwrap_or_else(|e| {
        log_error(log, &e);
        HashMap::new()
    });
    eprintln!("num symbols: {}", symbols.len());
    let state = State {
        base_address: base_address(),
        symbols,
        dynamic: Vec::with_capacity(MAX_DYNAMIC_METRICS),
        spans: Vec::new(),
    };
    ON_MAIN_THREAD.set(true);
    // SAFETY: called once from nginx's main(), before any other entry point.
    unsafe { *STATE.0.get() = Some(state) };
}

#[no_mangle]
pub extern "C" fn ngx_metric_report(metric: *mut ngx_metric_t, value: i64) {
    if !on_main_thread() {
        return;
    }
    // SAFETY: C passes a pointer to one of the builtin metrics.
    if let Some(metric) = unsafe { metric.as_mut() } {
        metric.report(value);
    }
}

#[no_mangle]
pub extern "C" fn ngx_get_num_metrics() -> c_int {
    with_state(|s| (builtin_metrics().len() + s.dynamic.len()) as c_int).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn ngx_get_metric(index: c_int) -> *mut ngx_metric_t {
    with_state(|s| {
        let builtins = builtin_metrics();
        let i = usize::try_from(index).ok()?;
        match i.checked_sub(builtins.len()) {
            None => Some(builtins[i]),
            Some(d) => s.dynamic.get_mut(d).map(|m| m as *mut _),
        }
    })
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn ngx_metrics_span_enter(handler: *const c_void) {
    with_state(|s| {
        s.spans.push(Span {
            handler: handler as usize,
            start_wall: clock_ns(libc::CLOCK_MONOTONIC),
            start_cpu: clock_ns(libc::CLOCK_THREAD_CPUTIME_ID),
            children_wall: 0,
            children_cpu: 0,
        })
    });
}

#[no_mangle]
pub extern "C" fn ngx_metrics_span_exit() {
    with_state(State::exit_span);
}

#[no_mangle]
pub extern "C" fn ngx_precise_time() -> i64 {
    clock_ns(libc::CLOCK_MONOTONIC)
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

fn log_error(log: *mut ngx_log_t, msg: &str) {
    let msg = CString::new(msg).unwrap_or_default();
    // SAFETY: `log` comes from nginx; `%s` takes a NUL-terminated string.
    unsafe { ngx_log_error_core(NGX_LOG_ERR, log, 0, c"ngx_metrics: %s".as_ptr(), msg.as_ptr()) };
}

fn load_symbols() -> Result<HashMap<usize, Symbol>, String> {
    let mut path = std::env::current_exe()
        .map_err(|e| format!("Failed to read /proc/self/exe: {e}"))?
        .into_os_string();
    path.push(".symbols");
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.to_string_lossy()))?;
    parse_symbols(&contents)
}

/// Parses `nm` output lines like `0000000000049393 t ngx_cleanup_environment_variable`.
fn parse_symbols(contents: &str) -> Result<HashMap<usize, Symbol>, String> {
    let mut symbols = HashMap::new();
    for (i, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let err = || format!("Failed to parse symbols file at line {}: {line}", i + 1);
        let mut fields = line.split_whitespace();
        let (Some(addr), Some(_type), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(err());
        };
        let addr = usize::from_str_radix(addr, 16).map_err(|_| err())?;
        let tags = CString::new(format!("{{handler=\"{name}\"}}")).map_err(|_| err())?;
        symbols.insert(addr, Symbol { tags, metrics: None });
    }
    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nm_output() {
        let symbols = parse_symbols(
            "0000000000049393 t ngx_cleanup_environment_variable\n00000000000a0000 T main\n",
        )
        .unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(
            symbols[&0x49393].tags.to_str().unwrap(),
            r#"{handler="ngx_cleanup_environment_variable"}"#
        );
        assert_eq!(symbols[&0xa0000].tags.to_str().unwrap(), r#"{handler="main"}"#);
        assert!(!symbols.contains_key(&0x49394));
        assert!(parse_symbols("0000000000049393 t\n").is_err());
        assert!(parse_symbols("zz t foo\n").is_err());
    }

    #[test]
    fn spans_report_self_and_total_time() {
        let mut s = State {
            base_address: 0x1000,
            symbols: parse_symbols("10 t outer\n20 t inner\n").unwrap(),
            dynamic: Vec::with_capacity(MAX_DYNAMIC_METRICS),
            spans: Vec::new(),
        };
        let span = |handler| Span { handler, start_wall: 0, start_cpu: 0, children_wall: 0, children_cpu: 0 };
        s.spans.push(span(0x1010));
        s.spans.push(span(0x1020));
        s.exit_span();
        s.exit_span();
        assert_eq!(s.dynamic.len(), 8);
        let [inner_total, _, _, _] = [0, 1, 2, 3].map(|i| s.dynamic[i].sum);
        let [outer_total, outer_self, _, _] = [4, 5, 6, 7].map(|i| s.dynamic[i].sum);
        assert_eq!(outer_self, outer_total - inner_total);
        assert!(s.dynamic.iter().all(|m| m.count == 1));
        assert!(s.spans.is_empty());
    }
}
