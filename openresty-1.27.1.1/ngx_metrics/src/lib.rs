#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::ffi::{c_char, c_int, CString};
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

type SymbolTable = HashMap<usize, ngx_metrics_symbol_t>;

static mut SYMBOLS: Option<SymbolTable> = None;

#[no_mangle]
pub extern "C" fn ngx_metrics_load_symbols(err: *mut c_char, errlen: c_int) -> c_int {
    match load_symbols() {
        Ok(symbols) => {
            let n = symbols.len() as c_int;
            unsafe { *&raw mut SYMBOLS = Some(symbols) };
            n
        }
        Err(e) => {
            if errlen > 0 {
                let n = e.len().min(errlen as usize - 1);
                unsafe {
                    std::ptr::copy_nonoverlapping(e.as_ptr(), err as *mut u8, n);
                    *err.add(n) = 0;
                }
            }
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn ngx_metrics_find_symbol(relative_addr: usize) -> *mut ngx_metrics_symbol_t {
    unsafe { (*&raw mut SYMBOLS).as_mut() }
        .and_then(|symbols| symbols.get_mut(&relative_addr))
        .map_or(std::ptr::null_mut(), |sym| sym as *mut _)
}

fn load_symbols() -> Result<SymbolTable, String> {
    let mut path = std::env::current_exe()
        .map_err(|e| format!("Failed to read /proc/self/exe: {e}"))?
        .into_os_string();
    path.push(".symbols");
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.to_string_lossy()))?;
    parse_symbols(&contents)
}

/// Parses `nm` output lines like `0000000000049393 t ngx_cleanup_environment_variable`.
fn parse_symbols(contents: &str) -> Result<SymbolTable, String> {
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
        let name = CString::new(name).map_err(|_| err())?.into_raw();
        symbols.insert(
            addr,
            ngx_metrics_symbol_t {
                name,
                metrics: [std::ptr::null_mut(); NGX_METRICS_NUM_SPAN_METRIC_TYPES as usize],
            },
        );
    }
    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn parses_nm_output() {
        let symbols = parse_symbols(
            "0000000000049393 t ngx_cleanup_environment_variable\n00000000000a0000 T main\n",
        )
        .unwrap();
        let name = |addr| unsafe { CStr::from_ptr(symbols[&addr].name) }.to_str().unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(name(0x49393), "ngx_cleanup_environment_variable");
        assert_eq!(name(0xa0000), "main");
        assert!(!symbols.contains_key(&0x49394));
        assert!(parse_symbols("0000000000049393 t\n").is_err());
        assert!(parse_symbols("zz t foo\n").is_err());
    }
}
