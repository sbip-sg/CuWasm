use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../..")
        .canonicalize()
        .unwrap();
    let build = root.join("build/rust/release");
    println!("cargo:rustc-link-search=native={}", build.display());

    cc::Build::new()
        .cpp(true)
        .file(root.join("src/capi.cpp"))
        .file(root.join("src/run.cpp"))
        .file(root.join("src/translate.cpp"))
        .file(root.join("src/verify.cpp"))
        .file(root.join("src/disasm.cpp"))
        .include(root.join("include"))
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-O2")
        .compile("cuwasm_runtime");

    // Link order: runtime objects reference symbols in cuwasm_translate.
    println!("cargo:rustc-link-lib=static=cuwasm_translate");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-link-lib=dylib=gcc_s");

    for f in [
        "src/capi.cpp",
        "src/run.cpp",
        "src/translate.cpp",
        "src/verify.cpp",
        "include/cuwasm/capi.h",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(f).display());
    }
}
