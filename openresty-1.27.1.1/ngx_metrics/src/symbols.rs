#![forbid(unsafe_code)]

use std::collections::HashMap;

/// Reads `<executable>.symbols` (from `generate-symbols`): address relative to the executable's base → name.
pub fn load() -> Result<HashMap<usize, String>, String> {
    let mut path = std::env::current_exe()
        .map_err(|e| format!("Failed to read /proc/self/exe: {e}"))?
        .into_os_string();
    path.push(".symbols");
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {e}", path.to_string_lossy()))?;
    parse(&contents)
}

/// Parses `nm` output lines like `0000000000049393 t ngx_cleanup_environment_variable`.
fn parse(contents: &str) -> Result<HashMap<usize, String>, String> {
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
        symbols.insert(addr, name.to_string());
    }
    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nm_output() {
        let symbols =
            parse("0000000000049393 t ngx_cleanup_environment_variable\n00000000000a0000 T main\n").unwrap();
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[&0x49393], "ngx_cleanup_environment_variable");
        assert_eq!(symbols[&0xa0000], "main");
        assert!(!symbols.contains_key(&0x49394));
        assert!(parse("0000000000049393 t\n").is_err());
        assert!(parse("zz t foo\n").is_err());
    }
}
