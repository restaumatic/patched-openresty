use std::env;
use std::path::PathBuf;

use bindgen::RustTarget;

fn main() {
    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let bindings = bindgen::Builder::default()
        // The input header we would like to generate
        // bindings for.
        .header("wrapper.h")
        .rust_target(RustTarget::stable(70, 0).unwrap_or_else(|_| panic!("Invalid Rust version")))
        .clang_arg("-I../build/nginx-1.27.1/src/core")
        .clang_arg("-I../build/nginx-1.27.1/src/event")
        .clang_arg("-I../build/nginx-1.27.1/src/os/unix")
        .clang_arg("-I../build/nginx-1.27.1/objs")
        .blocklist_function("ngx_metrics_dump_lua_stack")
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
