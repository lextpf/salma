/*!
 * @brief exposes the salma engine through a flat C ABI.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-memory: result ownership
 *
 * install results contain either the mod path or error text. call installSucceeded to
 * distinguish them. free every non-null string result with freeResult, except the static pointer
 * from getApiVersion.
 *
 * ### :material-lock-outline: thread safety
 *
 * every export catches rust panics. a host must serialize an install call and its status read
 * because install status and disk-full state are process-global. the log callback is also global
 * and can run concurrently.
 */

// the C ABI export names are camelCase because that is what both binders look up:
// scripts/mo2-salma.py through ctypes and src/SalmaEngine.cpp through GetProcAddress. the symbol
// emitted by #[unsafe(no_mangle)] is the rust identifier itself, so these identifiers cannot be
// renamed to snake_case without changing the exported symbol table.
#![allow(non_snake_case)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::Logger;

/**
 * @brief stable ABI version string, major.minor.patch.
 * @author Alex (https://github.com/lextpf)
 *
 * a different major is an incompatible ABI.
 */
pub const MO2_SALMA_API_VERSION: &str = "1.2.0";

/**
 * @brief leading major-version digit of MO2_SALMA_API_VERSION.
 * @author Alex (https://github.com/lextpf)
 */
pub const MO2_SALMA_API_MAJOR: &str = "1";

// nul-terminated form returned by getApiVersion.
// static storage: the pointer is valid for the whole process lifetime and must never be passed to
// `freeResult`.
static API_VERSION_C: &CStr = c"1.2.0";

// process-global "did the last install succeed" flag.
// the last install wins, and `SeqCst` store/load makes its value visible on every thread.
static LAST_INSTALL_SUCCESS: AtomicBool = AtomicBool::new(false);

// outcome of borrowing a C-string argument across the ABI boundary.
enum ArgStr<'a> {
    // the pointer was null.
    Null,
    // the caller uses the export's failure value for invalid UTF-8.
    InvalidUtf8,
    // a borrowed UTF-8 string; it may be empty.
    Str(&'a str),
}

// borrow a nul-terminated, UTF-8 C-string argument.
// `ptr` must be null, or point to a valid nul-terminated C string that stays alive for the returned
// borrow's lifetime `'a`.
unsafe fn borrow_arg<'a>(ptr: *const c_char) -> ArgStr<'a> {
    if ptr.is_null() {
        return ArgStr::Null;
    }
    // safety: the caller guarantees a valid nul-terminated string when non-null.
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => ArgStr::Str(s),
        Err(_) => ArgStr::InvalidUtf8,
    }
}

// allocate an owned C string the caller must release with freeResult.
// the returned pointer is non-null and heap-owned.
fn owned_cstring(s: &str) -> *const c_char {
    let cstring = match CString::new(s) {
        Ok(c) => c,
        Err(nul_err) => {
            let end = nul_err.nul_position();
            let mut bytes = nul_err.into_vec();
            bytes.truncate(end);
            // safety: bytes[..end] contains no interior NUL by construction.
            unsafe { CString::from_vec_unchecked(bytes) }
        }
    };
    cstring.into_raw().cast_const()
}

// run body behind catch_unwind so a rust panic never unwinds across the FFI boundary.
// on panic, `on_panic` produces the fallback value.
fn guard<R>(body: impl FnOnce() -> R, on_panic: impl FnOnce() -> R) -> R {
    match panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => on_panic(),
    }
}

fn set_last_install_success(value: bool) {
    LAST_INSTALL_SUCCESS.store(value, Ordering::SeqCst);
}

// shared body for install and installWithConfig, which differ only in the json_path they forward
// and the subsystem tag they log under.
// the null arm is matched first, so a null pointer wins over a non-UTF-8 one.
unsafe fn install_impl(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: &str,
    tag: &str,
) -> *const c_char {
    // safety: the caller of install_impl guarantees each pointer is null or a valid nul-terminated
    // string that stays alive for this call.
    match (unsafe { borrow_arg(archive_path) }, unsafe {
        borrow_arg(mod_path)
    }) {
        (ArgStr::Null, _) | (_, ArgStr::Null) => {
            set_last_install_success(false);
            owned_cstring("archivePath and modPath must not be null")
        }
        (ArgStr::InvalidUtf8, _) | (_, ArgStr::InvalidUtf8) => {
            // a path that is not valid UTF-8 is rejected here rather than forwarded as raw bytes,
            // and reports the same catch-all failure string a panic would.
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        }
        (ArgStr::Str(archive), ArgStr::Str(modp)) => {
            let mut service = crate::installation_service::InstallationService::new();
            match service.install_mod(archive, modp, json_path) {
                Ok(result) => {
                    set_last_install_success(true);
                    owned_cstring(&result)
                }
                Err(err) => {
                    Logger::instance().log_error(&format!("[{tag}] Fatal error: {err}"));
                    set_last_install_success(false);
                    // the caller sees the InstallError Display text verbatim.
                    owned_cstring(&err.to_string())
                }
            }
        }
    }
}

/**
 * @fn getApiVersion() -> *const c_char
 * @brief the ABI version string, as a nul-terminated C string.
 * @author Alex (https://github.com/lextpf)
 *
 * the pointer addresses static storage holding [`MO2_SALMA_API_VERSION`], is valid for the whole
 * process lifetime, is never heap-allocated, and must not be passed to [`freeResult`].
 */
#[unsafe(no_mangle)]
pub extern "C" fn getApiVersion() -> *const c_char {
    // returning a static pointer cannot panic; the guard is here so every export has the same
    // shape.
    guard(|| API_VERSION_C.as_ptr(), || API_VERSION_C.as_ptr())
}

/**
 * @fn setLogCallback(Option<unsafe extern "C" fn(*const c_char)>)
 * @brief register the host log callback, or clear it by passing null.
 * @author Alex (https://github.com/lextpf)
 *
 * the console echo (stdout for info and warning, stderr for error) is unaffected either way.
 */
#[unsafe(no_mangle)]
pub extern "C" fn setLogCallback(callback: Option<unsafe extern "C" fn(*const c_char)>) {
    guard(
        || {
            // the logger stores the pointer in a lock-free atomic, so this never blocks a logging
            // thread. a null callback re-enables file logging.
            crate::logger::Logger::instance().set_callback(callback);
        },
        || {},
    );
}

/**
 * @fn install(*const c_char, *const c_char) -> *const c_char
 * @brief install archive_path into mod_path, naming no selections JSON.
 * @author Alex (https://github.com/lextpf)
 *
 * on success the string is the installed mod directory path; on failure it is a human-readable
 * error message.
 *
 * ### :material-shield-lock: **Safety**
 *
 * `archive_path` and `mod_path` must each be null or a valid nul-terminated C string. the returned
 * non-null pointer must be released with [`freeResult`].
 * @return a non-null, heap-allocated string that the caller must release with [`freeResult`].
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn install(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // safety: install_impl borrows pointers under install's nul-or-valid contract.
        // an empty json_path selects the documented sidecar lookup.
        || unsafe { install_impl(archive_path, mod_path, "", "install") },
        || {
            Logger::instance().log_error("[install] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/**
 * @fn installWithConfig(*const c_char, *const c_char, *const c_char) -> *const c_char
 * @brief install archive_path into mod_path, using the selections JSON at json_path.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-shield-lock: **Safety**
 *
 * `archive_path` and `mod_path` must each be null or a valid nul-terminated C string. `json_path`
 * may be null, which is coerced to `""`; an empty or null `json_path` still lets the service pick
 * up a sibling `<archive stem>.json`, see [`install`].
 * @return contract identical to [`install`]: always a non-null heap string that the caller must
 * release with [`freeResult`], carrying the installed mod directory path on success and an error
 * message on failure, with [`installSucceeded`] as the only discriminator.
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn installWithConfig(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: *const c_char,
) -> *const c_char {
    guard(
        // safety: same nul-or-valid contract as install().
        || {
            // a null jsonPath is coerced to the empty string. invalid UTF-8 is rejected, consistent
            // with the other two arguments.
            // safety: `json_path` is null or a valid nul-terminated string that the caller keeps
            // alive for this call.
            let json = match unsafe { borrow_arg(json_path) } {
                ArgStr::Null => "",
                ArgStr::Str(s) => s,
                ArgStr::InvalidUtf8 => {
                    set_last_install_success(false);
                    return owned_cstring("Unknown fatal error during installation");
                }
            };
            // safety: both pointers are this export's own arguments, forwarded unchanged under the
            // contract stated above.
            unsafe { install_impl(archive_path, mod_path, json, "installWithConfig") }
        },
        || {
            Logger::instance().log_error("[installWithConfig] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/**
 * @fn inferFomodSelections(*const c_char, *const c_char) -> *const c_char
 * @brief infer FOMOD selections from an archive and its installed files.
 * @author Alex (https://github.com/lextpf)
 *
 * on any failure it holds `""`, because no `Result` crosses this boundary; the one exception is a
 * null argument, which yields `"archivePath and modPath must not be null"`.
 *
 * ### :material-shield-lock: **Safety**
 *
 * `archive_path` and `mod_path` must each be null or a valid nul-terminated UTF-8 C string. the
 * returned non-null pointer must be released with [`freeResult`].
 * @return a non-null, heap-allocated string that the caller must release with [`freeResult`].
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn inferFomodSelections(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // safety: inferFomodSelections borrows both pointers under its nul-or-valid contract.
        || match (unsafe { borrow_arg(archive_path) }, unsafe {
            borrow_arg(mod_path)
        }) {
            (ArgStr::Null, _) | (_, ArgStr::Null) => {
                owned_cstring("archivePath and modPath must not be null")
            }
            // both non-null and valid UTF-8: run the orchestrator. it returns "" on any internal
            // failure, so no Result crosses FFI.
            (ArgStr::Str(archive), ArgStr::Str(modp)) => {
                let service = crate::fomod_inference_service::FomodInferenceService::new();
                owned_cstring(&service.infer_selections(archive, modp))
            }
            // a path that is not valid UTF-8 yields the same "" as any other failure.
            _ => owned_cstring(""),
        },
        || {
            // guard maps an unexpected panic to the documented empty result.
            Logger::instance().log_error("[infer] Fatal error: unknown exception");
            owned_cstring("")
        },
    )
}

/**
 * @fn installSucceeded() -> bool
 * @brief whether the most recent install or installWithConfig call in this process succeeded.
 * @author Alex (https://github.com/lextpf)
 *
 * the value is one process-global atomic flag, not a per-call and not a per-thread result.
 * @return true only when the latest install or installWithConfig call succeeded.
 */
#[unsafe(no_mangle)]
pub extern "C" fn installSucceeded() -> bool {
    guard(|| LAST_INSTALL_SUCCESS.load(Ordering::SeqCst), || false)
}

/**
 * @fn freeResult(*const c_char)
 * @brief release a string previously returned by this DLL's owned-string exports.
 * @author Alex (https://github.com/lextpf)
 *
 * a null pointer is a no-op.
 *
 * ### :material-shield-lock: **Safety**
 *
 * `result` must be null, or a pointer previously returned by one of this DLL's owned-string exports
 * ([`install`], [`installWithConfig`], [`inferFomodSelections`], [`resolveModArchive`]) and not yet
 * freed. passing any other pointer (a foreign allocation, a stack pointer, an already-freed
 * pointer, or the static [`getApiVersion`] pointer) is undefined behavior: allocation and release
 * must stay on one side of the boundary, and this DLL does both with rust's allocator.
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeResult(result: *const c_char) {
    guard(
        || {
            if result.is_null() {
                return;
            }
            // safety: per the contract above, `result` came from owned_cstring
            // in this DLL (CString::into_raw), so reclaiming it with the
            // matching CString::from_raw is sound.
            unsafe { drop(CString::from_raw(result.cast_mut())) };
        },
        || {},
    );
}

/**
 * @fn resolveModArchive(*const c_char, *const c_char, *const c_char) -> *const c_char
 * @brief resolve a mod archive through the installation-file fallback chain.
 * @author Alex (https://github.com/lextpf)
 *
 * it holds the resolved archive path on success and `""` on every failure, including a null or
 * non-UTF-8 `installation_file` or `mod_folder`.
 *
 * ### :material-shield-lock: **Safety**
 *
 * `installation_file`, `mod_folder`, and `mods_dir` must each be null or a valid nul-terminated
 * UTF-8 C string. the returned non-null pointer must be released with [`freeResult`].
 * @return a non-null, heap-allocated string that the caller must release with [`freeResult`].
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resolveModArchive(
    installation_file: *const c_char,
    mod_folder: *const c_char,
    mods_dir: *const c_char,
) -> *const c_char {
    guard(
        // safety: resolveModArchive borrows all three pointers under its nul-or-valid contract.
        || {
            // only installationFile and modFolder are null-checked; a null modsDir is coerced to an
            // empty path and merely skips candidates 4-6.
            // safety: each pointer is null or a valid nul-terminated string that the caller keeps
            // alive for this call.
            let (file, folder) = match (unsafe { borrow_arg(installation_file) }, unsafe {
                borrow_arg(mod_folder)
            }) {
                (ArgStr::Str(f), ArgStr::Str(d)) => (f, d),
                // null and non-UTF-8 both yield "".
                _ => return owned_cstring(""),
            };
            // safety: `mods_dir` carries the same null-or-valid contract.
            let mods = match unsafe { borrow_arg(mods_dir) } {
                ArgStr::Str(s) => s,
                ArgStr::Null | ArgStr::InvalidUtf8 => "",
            };

            let resolved = crate::archive_resolver::resolve_mod_archive(
                file,
                std::path::Path::new(folder),
                std::path::Path::new(mods),
            );
            // an unresolved path is empty and already stringifies to "", which is the documented
            // failure value; no extra branch is needed.
            owned_cstring(&resolved.to_string_lossy())
        },
        || {
            // guard maps an unexpected panic to the documented empty result.
            Logger::instance().log_error("[resolveModArchive] Fatal error: unknown exception");
            owned_cstring("")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn take_owned(ptr: *const c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }
            .to_str()
            .expect("owned string is valid UTF-8")
            .to_owned();
        unsafe { freeResult(ptr) };
        s
    }

    unsafe extern "C" fn noop_log(_msg: *const c_char) {}

    #[test]
    fn version_constant_major_and_sync() {
        assert_eq!(MO2_SALMA_API_MAJOR, "1");
        assert!(MO2_SALMA_API_VERSION.starts_with(MO2_SALMA_API_MAJOR));
        assert_eq!(MO2_SALMA_API_VERSION.as_bytes()[0], b'1');
        assert_eq!(API_VERSION_C.to_bytes(), MO2_SALMA_API_VERSION.as_bytes());
    }

    #[test]
    fn get_api_version_returns_stable_string() {
        let ptr = getApiVersion();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "1.2.0");
    }

    #[test]
    fn owned_cstring_round_trips() {
        let ptr = owned_cstring("hello world");
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        assert_eq!(s, "hello world");
        unsafe { freeResult(ptr) };

        // an empty string is still a valid non-null pointer.
        let ptr = owned_cstring("");
        assert!(!ptr.is_null());
        assert!(unsafe { CStr::from_ptr(ptr) }.to_bytes().is_empty());
        unsafe { freeResult(ptr) };
    }

    #[test]
    fn owned_cstring_truncates_interior_nul() {
        let ptr = owned_cstring("a\0b");
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        assert_eq!(s, "a");
        unsafe { freeResult(ptr) };
    }

    #[test]
    fn free_result_null_is_noop() {
        unsafe { freeResult(std::ptr::null()) };
    }

    #[test]
    fn borrow_arg_classifies_inputs() {
        assert!(matches!(
            unsafe { borrow_arg(std::ptr::null()) },
            ArgStr::Null
        ));

        let valid = CString::new("utf8-ok").unwrap();
        assert!(matches!(
            unsafe { borrow_arg(valid.as_ptr()) },
            ArgStr::Str("utf8-ok")
        ));

        let invalid: [c_char; 2] = [0xFFu8 as c_char, 0];
        assert!(matches!(
            unsafe { borrow_arg(invalid.as_ptr()) },
            ArgStr::InvalidUtf8
        ));
    }

    #[test]
    fn infer_null_and_failure_contract() {
        let msg = unsafe { take_owned(inferFomodSelections(std::ptr::null(), std::ptr::null())) };
        assert_eq!(msg, "archivePath and modPath must not be null");

        let archive = CString::new("archive.7z").unwrap();
        let mod_dir = CString::new("mod").unwrap();
        let out = unsafe { take_owned(inferFomodSelections(archive.as_ptr(), mod_dir.as_ptr())) };
        assert_eq!(out, "");
    }

    // this is the only test that touches LAST_INSTALL_SUCCESS, so the shared process-global flag
    // cannot race against a parallel test. keep it that way.
    #[test]
    fn install_wires_the_service_and_tracks_the_success_flag() {
        // null inputs -> the null-argument message; flag cleared.
        let msg = unsafe { take_owned(install(std::ptr::null(), std::ptr::null())) };
        assert_eq!(msg, "archivePath and modPath must not be null");
        assert!(!installSucceeded());

        // a real failure carries the InstallError text through unchanged.
        let archive = CString::new("definitely-absent-archive.7z").unwrap();
        let mod_dir = CString::new("mod").unwrap();
        let msg = unsafe { take_owned(install(archive.as_ptr(), mod_dir.as_ptr())) };
        assert_eq!(msg, "Archive file not found: definitely-absent-archive.7z");
        assert!(!installSucceeded(), "a failed install clears the flag");

        // installWithConfig reports the same failure and also clears the flag.
        let cfg = CString::new("selections.json").unwrap();
        let msg = unsafe {
            take_owned(installWithConfig(
                archive.as_ptr(),
                mod_dir.as_ptr(),
                cfg.as_ptr(),
            ))
        };
        assert_eq!(msg, "Archive file not found: definitely-absent-archive.7z");
        assert!(!installSucceeded());

        // a null jsonPath is coerced to "" and must not change the outcome.
        let msg = unsafe {
            take_owned(installWithConfig(
                archive.as_ptr(),
                mod_dir.as_ptr(),
                std::ptr::null(),
            ))
        };
        assert_eq!(msg, "Archive file not found: definitely-absent-archive.7z");
        assert!(!installSucceeded());

        let root =
            std::env::temp_dir().join(format!("salma-capi-{}", crate::utils::random_hex_string(8)));
        let src = root.join("src.zip");
        std::fs::create_dir_all(&root).expect("scratch");
        // a minimal but real zip: the archive layer must be able to open it.
        let svc = crate::archive_service::ArchiveService::new();
        std::fs::create_dir_all(root.join("payload")).expect("payload dir");
        std::fs::write(root.join("payload").join("readme.txt"), b"hi").expect("write");
        svc.create_zip(
            root.join("payload").to_str().unwrap(),
            src.to_str().unwrap(),
        )
        .expect("build a real zip fixture");

        let out_dir = root.join("out");
        let c_archive = CString::new(src.to_str().unwrap()).unwrap();
        let c_out = CString::new(out_dir.to_str().unwrap()).unwrap();
        let msg = unsafe { take_owned(install(c_archive.as_ptr(), c_out.as_ptr())) };
        assert_eq!(msg, out_dir.to_string_lossy(), "success returns mod_path");
        assert!(installSucceeded(), "a successful install sets the flag");
        assert!(out_dir.join("readme.txt").exists(), "content was installed");

        // the flag is sticky across other exports: neither inferFomodSelections nor
        // resolveModArchive writes it.
        let _ = unsafe { take_owned(inferFomodSelections(c_archive.as_ptr(), c_out.as_ptr())) };
        assert!(installSucceeded(), "infer must not touch the install flag");

        // restore the shared flag so test ordering cannot leak this success.
        let _ = unsafe { take_owned(install(std::ptr::null(), std::ptr::null())) };
        assert!(!installSucceeded());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_mod_archive_is_wired_to_the_resolver() {
        // null installationFile or modFolder -> "".
        let out = unsafe {
            take_owned(resolveModArchive(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, "");

        // an unresolvable name yields "" because the resolver missed every candidate, which is
        // indistinguishable from a bad argument.
        let file = CString::new("definitely-absent-mod.7z").unwrap();
        let folder = CString::new("C:/mods/MyMod").unwrap();
        let out = unsafe {
            take_owned(resolveModArchive(
                file.as_ptr(),
                folder.as_ptr(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, "");

        // a resolvable archive under the mod folder comes back, which is what the MO2 plugin
        // depends on: it gates on hasattr and never falls back once the symbol exists.
        let root = std::env::temp_dir().join(format!(
            "salma-capi-res-{}",
            crate::utils::random_hex_string(8)
        ));
        let mod_folder = root.join("MyMod");
        let name = format!("payload-{}.7z", crate::utils::random_hex_string(12));
        let archive = mod_folder.join(&name);
        std::fs::create_dir_all(&mod_folder).expect("scratch");
        std::fs::write(&archive, b"x").expect("write");

        let c_file = CString::new(name.as_str()).unwrap();
        let c_folder = CString::new(mod_folder.to_str().unwrap()).unwrap();
        let out = unsafe {
            take_owned(resolveModArchive(
                c_file.as_ptr(),
                c_folder.as_ptr(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, archive.to_string_lossy());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn set_log_callback_reaches_the_logger() {
        let logger = crate::logger::Logger::instance();
        setLogCallback(Some(noop_log));
        assert!(
            logger.has_callback(),
            "setLogCallback must register with the logger"
        );
        setLogCallback(None);
        assert!(!logger.has_callback(), "null must clear it");
    }
}
