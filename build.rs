//! Build script for `mo2_salma_rs`.
//!
//! Its sole job is to add the Win32 import libraries the `unrar_sys` 0.5.8
//! crate needs but does not emit itself. The bundled unRAR C sources call into
//! `advapi32` (registry + crypto: `RegOpenKeyExW`, `CryptAcquireContextW`,
//! `SetFileSecurityW`, ...) and `user32`; without these link directives the
//! final link of any binary that pulls in this crate fails with ~13 LNK2019
//! "unresolved external symbol" errors. `unrar_sys` compiles the C with `cc`
//! but leaves the system-library linkage to the downstream crate, so we supply
//! it here.
//!
//! Gated to Windows: on other targets these libraries do not exist and the
//! unRAR sources take a different code path. The gate reads
//! `CARGO_CFG_TARGET_OS` (set by cargo for the TARGET, the correct signal in a
//! build script) rather than the host `cfg!`.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=user32");
    }
}
