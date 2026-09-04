/*!
 * @brief link the Win32 libraries required by unrar_sys.
 * @author Alex (https://github.com/lextpf)
 *
 * unrar_sys does not declare these libraries. the target OS gate supports cross-compilation.
 */

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=user32");
    }
}
