use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = PathBuf::from(&crate_dir).join("generated");

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&out_dir).ok();

    // Generate C# bindings using csbindgen
    csbindgen::Builder::default()
        .input_extern_file("src/lib.rs")
        .input_extern_file("src/handles.rs")
        .input_extern_file("src/error.rs")
        .input_extern_file("src/database.rs")
        .input_extern_file("src/collection.rs")
        .input_extern_file("src/crud.rs")
        .input_extern_file("src/index.rs")
        .input_extern_file("src/aggregation.rs")
        .input_extern_file("src/transaction.rs")
        .input_extern_file("src/memory.rs")
        .csharp_dll_name("ironbase_ffi")
        .csharp_namespace("IronBase.Interop")
        .csharp_class_name("NativeMethods")
        .csharp_class_accessibility("internal")
        .csharp_dll_name_if("IRONBASE_WINDOWS", "ironbase_ffi.dll")
        .csharp_dll_name_if("IRONBASE_LINUX", "libironbase_ffi.so")
        .generate_csharp_file(out_dir.join("NativeMethods.g.cs"))
        .unwrap();
}
