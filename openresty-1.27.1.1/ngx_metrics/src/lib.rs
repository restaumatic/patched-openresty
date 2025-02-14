#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::ffi::{c_char, c_int};
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
        if unsafe { lua_getinfo(l, "Sln\0".as_ptr() as *const i8, ar_u.as_mut_ptr()) } == 0 {
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
