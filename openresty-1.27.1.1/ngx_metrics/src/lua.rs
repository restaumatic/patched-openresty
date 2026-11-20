use std::io::Write;
use std::mem::MaybeUninit;

use crate::ffi::{lua_Debug, lua_State, lua_getinfo, lua_getstack};

/// # Safety
/// `l` is a live Lua state.
pub unsafe fn dump_stack(l: *mut lua_State, mut writer: impl Write) -> std::io::Result<()> {
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
