//! Flat C ABI boundary for the salma `mo2-core` Rust port.
//!
//! This module mirrors `src/CApi.cpp` (namespace `CApi`) export-for-export.
//! The eight `#[unsafe(no_mangle)] pub extern "C"` functions below are the only
//! symbols the DLL exports, matching `src/CApi.hpp` exactly so the MO2 Python
//! plugin (`scripts/mo2-salma.py`) can load this DLL in place of the C++
//! `mo2-salma.dll`.
//!
//! ## Ownership
//!
//! Identical to the C++ `_strdup` / `free` pairing: every non-null
//! `*const c_char` handed out by [`install`], [`installWithConfig`],
//! [`inferFomodSelections`], and [`resolveModArchive`] is heap-allocated and
//! MUST be released with [`freeResult`]. [`getApiVersion`] is the sole
//! exception: it returns a pointer to static storage that is never allocated
//! and never freed.
//!
//! ## Panics
//!
//! No panic is allowed to unwind across the FFI boundary (that is undefined
//! behavior). Every export routes through [`guard`], mirroring the C++
//! `catch (...)` at each entry point, and returns that export's caught-error
//! value instead.

// The C ABI export names are camelCase to match src/CApi.hpp verbatim. The
// symbol emitted by #[unsafe(no_mangle)] is the Rust identifier itself, so
// these identifiers cannot be renamed to snake_case without changing the
// exported symbol table.
#![allow(non_snake_case)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::Logger;

/// Stable ABI version string, mirror of the C++ `MO2_SALMA_API_VERSION`.
///
/// Format is `MAJOR.MINOR.PATCH`; a different MAJOR is an incompatible ABI.
/// Kept in sync with [`API_VERSION_C`] (asserted by a unit test).
pub const MO2_SALMA_API_VERSION: &str = "1.2.0";

/// Leading major-version digit, mirror of the C++ `MO2_SALMA_API_MAJOR`.
pub const MO2_SALMA_API_MAJOR: &str = "1";

/// Nul-terminated form returned by [`getApiVersion`]. Static storage: the
/// pointer is valid for the whole process lifetime and must never be passed to
/// [`freeResult`].
static API_VERSION_C: &CStr = c"1.2.0";

/// Process-global "did the last install succeed" flag. Mirrors the
/// mutex-guarded `g_last_install_success` in `src/CApi.cpp`: the last install
/// wins and the value is visible across threads (`SeqCst` store/load). A single
/// atomic bool is sufficient - the C++ mutex only ever guarded a bool.
static LAST_INSTALL_SUCCESS: AtomicBool = AtomicBool::new(false);

/// Outcome of borrowing a C-string argument across the ABI boundary.
enum ArgStr<'a> {
    /// The incoming pointer was null.
    Null,
    /// The pointer was non-null but the bytes were not valid UTF-8. The ABI
    /// treats this like a caught exception: the caller substitutes its
    /// per-export failure value.
    InvalidUtf8,
    /// A valid borrowed UTF-8 string (may be empty).
    Str(&'a str),
}

/// Borrow a nul-terminated, UTF-8 C-string argument.
///
/// # Safety
///
/// `ptr` must be null, or point to a valid nul-terminated C string that stays
/// alive for the returned borrow's lifetime `'a`.
unsafe fn borrow_arg<'a>(ptr: *const c_char) -> ArgStr<'a> {
    if ptr.is_null() {
        return ArgStr::Null;
    }
    // SAFETY: the caller guarantees a valid nul-terminated string when non-null.
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => ArgStr::Str(s),
        Err(_) => ArgStr::InvalidUtf8,
    }
}

/// Allocate an owned C string the caller must release with [`freeResult`].
///
/// Mirrors the C++ `_strdup` contract: the returned pointer is non-null and
/// heap-owned. A C string cannot carry an interior NUL, so on the (practically
/// impossible for our data) chance `s` contains one, the string is truncated at
/// the first NUL exactly as C-string semantics would. Never panics.
fn owned_cstring(s: &str) -> *const c_char {
    let cstring = match CString::new(s) {
        Ok(c) => c,
        Err(nul_err) => {
            let end = nul_err.nul_position();
            let mut bytes = nul_err.into_vec();
            bytes.truncate(end);
            // SAFETY: bytes[..end] contains no interior NUL by construction.
            unsafe { CString::from_vec_unchecked(bytes) }
        }
    };
    cstring.into_raw().cast_const()
}

/// Run `body` behind `catch_unwind` so a Rust panic never unwinds across the
/// FFI boundary. On panic, `on_panic` produces the fallback value. `on_panic`
/// is evaluated only on panic, so callers can allocate their error string
/// lazily without leaking on the success path.
fn guard<R>(body: impl FnOnce() -> R, on_panic: impl FnOnce() -> R) -> R {
    match panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => on_panic(),
    }
}

/// Store the process-global last-install-success flag (last write wins).
fn set_last_install_success(value: bool) {
    LAST_INSTALL_SUCCESS.store(value, Ordering::SeqCst);
}

/// Shared body for [`install`] and [`installWithConfig`], mirroring
/// `CApi::install` (`src/CApi.cpp:39-70`) and `CApi::installWithConfig`
/// (`:72-107`), which differ only in the `json_path` they forward.
///
/// The success flag becomes true if and only if `install_mod` RETURNED, without
/// inspecting what it returned. That is the C++ code's predicate
/// (`CApi.cpp:51-53`, `:87-89`), not the looser one `CApi.hpp:252-256`
/// describes; see PARITY-NOTES "Task 15".
///
/// `tag` is the subsystem tag the two exports log under (`install` /
/// `installWithConfig`); it is the only other difference between them.
///
/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated
/// C string.
unsafe fn install_impl(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: &str,
    tag: &str,
) -> *const c_char {
    match (unsafe { borrow_arg(archive_path) }, unsafe {
        borrow_arg(mod_path)
    }) {
        (ArgStr::Null, _) | (_, ArgStr::Null) => {
            set_last_install_success(false);
            owned_cstring("archivePath and modPath must not be null")
        }
        (ArgStr::InvalidUtf8, _) | (_, ArgStr::InvalidUtf8) => {
            // Invalid UTF-8 is handled like a caught exception: the C++
            // catch-all failure string, flag cleared. The C++ has no such
            // branch (it forwards the raw bytes); documented in PARITY-NOTES.
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
                    // Mirror of the C++ `catch (const std::exception& e)`: the
                    // error this port raises stands in for the thrown exception.
                    Logger::instance().log_error(&format!("[{tag}] Fatal error: {err}"));
                    set_last_install_success(false);
                    // The C++ returns `e.what()`; InstallError::Display carries
                    // exactly that text for every salma-authored message.
                    owned_cstring(&err.to_string())
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn getApiVersion() -> *const c_char {
    // Returns a pointer to static storage; never allocated, never freed. The
    // uniform panic guard is applied for consistency with the ABI contract,
    // although returning a static pointer cannot panic. Callers must NOT pass
    // this pointer to freeResult().
    guard(|| API_VERSION_C.as_ptr(), || API_VERSION_C.as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn setLogCallback(callback: Option<unsafe extern "C" fn(*const c_char)>) {
    guard(
        || {
            // Mirror of `CApi::setLogCallback` (`src/CApi.cpp:32-37`): forward
            // straight to the logger, whose store is a lock-free atomic. A null
            // callback re-enables file logging (logs/salma.log).
            crate::logger::Logger::instance().set_callback(callback);
        },
        || {},
    );
}

/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated
/// UTF-8 C string. The returned non-null pointer must be released with
/// [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn install(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: pointers are only borrowed inside install_impl, which upholds
        // the same nul-or-valid contract this function documents.
        // An empty json_path is NOT "no selections": the service derives a
        // sibling `<archive stem>.json` and uses it when present, so an archive
        // with a sidecar is installed WITH those selections. `CApi.hpp:177-182`
        // states otherwise; the code wins (see PARITY-NOTES "Task 15").
        || unsafe { install_impl(archive_path, mod_path, "", "install") },
        || {
            // Mirror of the C++ `catch (...)`.
            Logger::instance().log_error("[install] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated
/// UTF-8 C string. `json_path` may be null, which is coerced to `""` exactly as
/// the C++ does (`src/CApi.cpp:86`). The returned non-null pointer must be
/// released with [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn installWithConfig(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: same nul-or-valid contract as install().
        || {
            // Null jsonPath is coerced to the empty string (CApi.cpp:82,86).
            // Invalid UTF-8 is treated like a caught exception, consistent with
            // the other two arguments.
            let json = match unsafe { borrow_arg(json_path) } {
                ArgStr::Null => "",
                ArgStr::Str(s) => s,
                ArgStr::InvalidUtf8 => {
                    set_last_install_success(false);
                    return owned_cstring("Unknown fatal error during installation");
                }
            };
            unsafe { install_impl(archive_path, mod_path, json, "installWithConfig") }
        },
        || {
            // Mirror of the C++ `catch (...)`.
            Logger::instance().log_error("[installWithConfig] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated
/// UTF-8 C string. The returned non-null pointer must be released with
/// [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn inferFomodSelections(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: pointers are only borrowed for the null / UTF-8 classification
        // below; both are null-or-valid per this function's contract.
        || match (unsafe { borrow_arg(archive_path) }, unsafe {
            borrow_arg(mod_path)
        }) {
            (ArgStr::Null, _) | (_, ArgStr::Null) => {
                owned_cstring("archivePath and modPath must not be null")
            }
            // Both non-null and valid UTF-8: run the orchestrator. It returns ""
            // on ANY internal failure (mirror of the C++ outer try/catch), so no
            // Result crosses FFI.
            (ArgStr::Str(archive), ArgStr::Str(modp)) => {
                let service = crate::fomod_inference_service::FomodInferenceService::new();
                owned_cstring(&service.infer_selections(archive, modp))
            }
            // Invalid UTF-8 is handled like a caught exception -> "" (the C++
            // treats a bad path as a throw caught by the outer handler).
            _ => owned_cstring(""),
        },
        || {
            // Mirror of the C++ `catch (...)`. The sibling C++ handler,
            // `catch (const std::exception&)`, logs "[infer] Fatal error: {}"
            // but is unreachable in BOTH languages: `infer_selections` swallows
            // every internal failure and returns "" rather than propagating.
            Logger::instance().log_error("[infer] Fatal error: unknown exception");
            owned_cstring("")
        },
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn installSucceeded() -> bool {
    guard(|| LAST_INSTALL_SUCCESS.load(Ordering::SeqCst), || false)
}

/// # Safety
///
/// `result` must be null, or a pointer previously returned by one of this
/// DLL's owned-string exports ([`install`], [`installWithConfig`],
/// [`inferFomodSelections`], [`resolveModArchive`]) and not yet freed. Passing
/// any other pointer (a foreign allocation, a stack pointer, an already-freed
/// pointer, or the static [`getApiVersion`] pointer) is undefined behavior -
/// exactly as the C++ `free()` / `_strdup` pairing assumes a single allocator
/// on both sides. Because this DLL both allocates and frees these strings via
/// Rust's allocator, the pairing is internally consistent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeResult(result: *const c_char) {
    guard(
        || {
            if result.is_null() {
                return;
            }
            // SAFETY: per the contract above, `result` came from owned_cstring
            // in THIS DLL (CString::into_raw), so reclaiming it with the
            // matching CString::from_raw is sound.
            unsafe { drop(CString::from_raw(result.cast_mut())) };
        },
        || {},
    );
}

/// # Safety
///
/// `installation_file`, `mod_folder`, and `mods_dir` must each be null or a
/// valid nul-terminated UTF-8 C string. The returned non-null pointer must be
/// released with [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resolveModArchive(
    installation_file: *const c_char,
    mod_folder: *const c_char,
    mods_dir: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: all three are null-or-valid per this function's contract and
        // are only borrowed for the classification below.
        || {
            // Only installationFile and modFolder are null-checked; a null
            // modsDir is coerced to an empty path and merely skips candidates
            // 4-6. That asymmetry is intentional in the C++
            // (`CApi.cpp:147-150` vs `:155`) and is reproduced.
            let (file, folder) = match (unsafe { borrow_arg(installation_file) }, unsafe {
                borrow_arg(mod_folder)
            }) {
                (ArgStr::Str(f), ArgStr::Str(d)) => (f, d),
                // Null -> "" per CApi.cpp:147-150. Invalid UTF-8 is treated like
                // a caught exception, which also yields "" (CApi.cpp:158-169).
                _ => return owned_cstring(""),
            };
            let mods = match unsafe { borrow_arg(mods_dir) } {
                ArgStr::Str(s) => s,
                ArgStr::Null | ArgStr::InvalidUtf8 => "",
            };

            let resolved = crate::archive_resolver::resolve_mod_archive(
                file,
                std::path::Path::new(folder),
                std::path::Path::new(mods),
            );
            // The C++ `resolved.empty() ? "" : resolved.string().c_str()` is a
            // no-op ternary: an empty path already stringifies to "".
            owned_cstring(&resolved.to_string_lossy())
        },
        || {
            // Mirror of the C++ `catch (...)`. Its sibling
            // `catch (const std::exception&)` logs "[resolveModArchive] Fatal
            // error: {}" and is unreachable here: `resolve_mod_archive` returns
            // an empty path instead of raising.
            Logger::instance().log_error("[resolveModArchive] Fatal error: unknown exception");
            owned_cstring("")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode an owned-string return into a `String` and free the underlying
    /// allocation, proving the round-trip does not leak or crash.
    ///
    /// # Safety
    ///
    /// `ptr` must be a non-null pointer produced by one of this DLL's
    /// owned-string exports.
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
        // Mirror of the C++ static_assert: leading digit is the major version.
        assert_eq!(MO2_SALMA_API_VERSION.as_bytes()[0], b'1');
        // The C string handed to callers stays in sync with the &str constant.
        assert_eq!(API_VERSION_C.to_bytes(), MO2_SALMA_API_VERSION.as_bytes());
    }

    #[test]
    fn get_api_version_returns_stable_string() {
        let ptr = getApiVersion();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "1.2.0");
        // Deliberately NOT freed: it points at static storage.
    }

    #[test]
    fn owned_cstring_round_trips() {
        let ptr = owned_cstring("hello world");
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        assert_eq!(s, "hello world");
        unsafe { freeResult(ptr) };

        // Empty string is a valid non-null pointer (mirrors _strdup("")).
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

        // 0xFF is not valid UTF-8; the buffer is nul-terminated.
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

    // This is the ONLY test that touches LAST_INSTALL_SUCCESS, so the shared
    // process-global flag cannot race against a parallel test. Keep it that way.
    #[test]
    fn install_wires_the_service_and_tracks_the_success_flag() {
        // Null inputs -> the exact C++ null-argument message; flag cleared.
        let msg = unsafe { take_owned(install(std::ptr::null(), std::ptr::null())) };
        assert_eq!(msg, "archivePath and modPath must not be null");
        assert!(!installSucceeded());

        // A real failure now comes from InstallationService, and the returned
        // string is its `what()` equivalent rather than a placeholder.
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

        // A NULL jsonPath is coerced to "" and must not change the outcome.
        let msg = unsafe {
            take_owned(installWithConfig(
                archive.as_ptr(),
                mod_dir.as_ptr(),
                std::ptr::null(),
            ))
        };
        assert_eq!(msg, "Archive file not found: definitely-absent-archive.7z");
        assert!(!installSucceeded());

        // A SUCCEEDING install sets the flag and returns mod_path verbatim.
        // Empty archive + empty mod tree is the cheapest success: the non-FOMOD
        // flat-copy fallback over an empty directory.
        let root =
            std::env::temp_dir().join(format!("salma-capi-{}", crate::utils::random_hex_string(8)));
        let src = root.join("src.zip");
        std::fs::create_dir_all(&root).expect("scratch");
        // A minimal but real zip: the archive layer must be able to open it.
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

        // The flag is STICKY across other exports: neither inferFomodSelections
        // nor resolveModArchive clears it (CApi.cpp:109-170 never writes it).
        let _ = unsafe { take_owned(inferFomodSelections(c_archive.as_ptr(), c_out.as_ptr())) };
        assert!(installSucceeded(), "infer must not touch the install flag");

        // Restore the shared flag so test ordering cannot leak this success.
        let _ = unsafe { take_owned(install(std::ptr::null(), std::ptr::null())) };
        assert!(!installSucceeded());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_mod_archive_is_wired_to_the_resolver() {
        // Null installationFile or modFolder -> "" (CApi.cpp:147-150).
        let out = unsafe {
            take_owned(resolveModArchive(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, "");

        // An unresolvable name still yields "" - but now because the resolver
        // missed, not because the export is a stub.
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

        // A resolvable archive under the mod folder now comes back, which is
        // the behavior the MO2 plugin depends on: it gates on hasattr and never
        // falls back once the symbol exists.
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

    /// The export must reach the real logger now, not a private slot. This and
    /// `logger::tests::set_callback_registers_and_clears` are the only tests
    /// that touch the process-global callback, and both run in this binary, so
    /// neither may assert on state the other could be holding: each registers,
    /// checks, and clears within its own body.
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
