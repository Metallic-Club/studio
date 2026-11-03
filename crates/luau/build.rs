//! Gates the `jit` feature: Luau's native code generator may only be built for x86-64 and aarch64 Windows, Linux,
//! macOS and Android. iOS and consoles forbid generating machine code at run time, so enabling `jit` for them is a
//! build error rather than a binary that the platform would reject (ADR 0003).
//!
//! (The panic-strategy guard is a `compile_error!` in `src/lib.rs`: Cargo does not tell build scripts the profile's
//! panic strategy.)

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_JIT").is_none() {
        return;
    }
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os_allowed = matches!(os.as_str(), "windows" | "linux" | "macos" | "android");
    let arch_allowed = matches!(arch.as_str(), "x86_64" | "aarch64");
    if !(os_allowed && arch_allowed) {
        eprintln!(
            "metallic_luau: the `jit` feature is not allowed for {arch}-{os}. Luau's native code generation is \
             limited to x86-64 and aarch64 Windows, Linux, macOS and Android; iOS and consoles must use the \
             interpreter (docs/adr/0003-scripting.md)."
        );
        std::process::exit(1);
    }
}
