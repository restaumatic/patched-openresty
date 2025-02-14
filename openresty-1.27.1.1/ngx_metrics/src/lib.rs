#[no_mangle]
pub extern "C" fn ngx_metrics_hello() {
    eprintln!("Hello from Rust!");
}
