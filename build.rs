//! Build script for `mo2_salma_rs`.
//!
//! `unrar_sys` compiles the bundled unRAR C sources but leaves their Win32
//! linkage to the downstream crate, so supply it here or the final link fails
//! with ~13 LNK2019 errors. Windows-only, gated on `CARGO_CFG_TARGET_OS`
//! because a build script must test the target, not the host `cfg!`.

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=user32");
    }
}
