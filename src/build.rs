use std::path::PathBuf;
use std::process::Command;

fn main() {
    // VPI symbols must be resolvable by dlopen'd DPI/VPI modules.
    //
    // The flag is spelled differently per linker: GNU/ELF ld takes
    // `-export-dynamic` (hyphen), Apple's ld takes `-export_dynamic`
    // (underscore) and — since the Xcode linker rewrite — rejects unknown
    // options as a hard error rather than warning, so the wrong spelling
    // breaks the build outright on macOS.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" | "ios" => {
            println!("cargo:rustc-link-arg=-Wl,-export_dynamic");
        }
        // Windows has no equivalent (symbols are exported via a .def /
        // dllexport), and passing an unknown flag would fail the link.
        "windows" => {}
        _ => {
            println!("cargo:rustc-link-arg=-Wl,-export-dynamic");
        }
    }

    // vpi_printf and friends are C-variadic, which Rust cannot define on
    // stable (`c_variadic` is unstable). Compile a small C shim and link it
    // in. Invoked through `cc` directly rather than via the `cc` crate so
    // this adds no build dependency.
    let src = "src/vpi_printf_shim.c";
    println!("cargo:rerun-if-changed={}", src);

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let obj = out_dir.join("vpi_printf_shim.o");
    let lib = out_dir.join("libvpishim.a");

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .args(["-c", "-fPIC", "-O2", src, "-o"])
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {}", cc, e));
    assert!(status.success(), "{} failed on {}", cc, src);

    let ar = std::env::var("AR").unwrap_or_else(|_| "ar".to_string());
    let _ = std::fs::remove_file(&lib);
    let status = Command::new(&ar)
        .arg("crs")
        .arg(&lib)
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {}", ar, e));
    assert!(status.success(), "{} failed", ar);

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    // `+whole-archive` so vpi_printf is kept even though no Rust code
    // references it — a dlopen'd VPI module resolves it at load time.
    println!("cargo:rustc-link-lib=static:+whole-archive=vpishim");
}
