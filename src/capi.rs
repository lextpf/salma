/*!
 * @brief Exposes the salma engine through a flat C ABI.
 * @author Alex (<https://github.com/lextpf>)
 *
 * ### :material-memory: Result ownership
 *
 * Install results contain either the mod path or error text. Call installSucceeded to
 * distinguish them. Free every non-null string result with freeResult, except the static pointer
 * from getApiVersion.
 *
 * ### :material-lock-outline: Thread safety
 *
 * Every export guards against Rust unwinding. A host must serialize an install and its status read
 * because install status and disk-full state are process-global. The log callback is also global
 * and can run concurrently.
 */

// The C ABI export names are camelCase because that is what both binders look up:
// scripts/mo2-salma.py through ctypes and src/SalmaEngine.cpp through GetProcAddress. The symbol
// emitted by #[unsafe(no_mangle)] is the rust identifier itself, so these identifiers cannot be
// renamed to snake_case without changing the exported symbol table.
#![allow(non_snake_case)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::Logger;

/**
 * @brief Stable ABI version string, major.minor.patch.
 * @author Alex (<https://github.com/lextpf>)
 *
 * A different major is an incompatible ABI.
 */
pub const MO2_SALMA_API_VERSION: &str = "1.2.0";

/**
 * @brief Leading major-version digit of MO2_SALMA_API_VERSION.
 * @author Alex (<https://github.com/lextpf>)
 */
pub const MO2_SALMA_API_MAJOR: &str = "1";

// nul-terminated form returned by getApiVersion.
// Static storage: the pointer is valid while the DLL is loaded and must never be passed to
// `freeResult`.
static API_VERSION_C: &CStr = c"1.2.0";

// Process-global "did the last install succeed" flag.
// The last install wins, and `SeqCst` store/load makes its value visible on every thread.
static LAST_INSTALL_SUCCESS: AtomicBool = AtomicBool::new(false);

// Outcome of borrowing a C-string argument across the ABI boundary.
enum ArgStr<'a> {
    // The pointer was null.
    Null,
    // The caller uses the export's failure value for invalid UTF-8.
    InvalidUtf8,
    // A borrowed UTF-8 string; it may be empty.
    Str(&'a str),
}

/**
 * @fn `borrow_arg<'a>(*const c_char) -> ArgStr<'a>`
 * @brief Borrow a C string without copying its bytes.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Null and invalid UTF-8 remain distinct so each export can choose its failure value.
 *
 * ### :material-shield-lock: **Safety**
 *
 * The caller must keep a non-null pointer readable through its NUL terminator for the
 * entire returned borrow. The bytes must not change during that borrow.
 *
 * @param ptr Null or a pointer to a NUL-terminated byte string.
 * @return A null marker, an invalid UTF-8 marker, or a borrowed string.
 */
unsafe fn borrow_arg<'a>(ptr: *const c_char) -> ArgStr<'a> {
    if ptr.is_null() {
        return ArgStr::Null;
    }
    // Safety: the caller guarantees a valid nul-terminated string when non-null.
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => ArgStr::Str(s),
        Err(_) => ArgStr::InvalidUtf8,
    }
}

/**
 * @fn `owned_cstring(&str) -> *const c_char`
 * @brief Allocate a result string for the host to release.
 * @author Alex (<https://github.com/lextpf>)
 *
 * An interior NUL truncates the result at its first occurrence.
 *
 * @param s Text to copy into DLL-owned storage.
 * @return A non-null pointer that must be passed to freeResult exactly once.
 */
fn owned_cstring(s: &str) -> *const c_char {
    let cstring = match CString::new(s) {
        Ok(c) => c,
        Err(nul_err) => {
            let end = nul_err.nul_position();
            let mut bytes = nul_err.into_vec();
            bytes.truncate(end);
            // Safety: bytes[..end] contains no interior NUL by construction.
            unsafe { CString::from_vec_unchecked(bytes) }
        }
    };
    cstring.into_raw().cast_const()
}

/**
 * @fn `guard<R>(impl FnOnce() -> R, impl FnOnce() -> R) -> R`
 * @brief Map a caught Rust panic to an export-specific failure value.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The fallback runs outside the unwind guard and must not panic. This guard does not
 * make invalid pointers safe or recover from process aborts.
 *
 * @tparam R Export result type.
 * @param body Operation to execute within the unwind guard.
 * @param on_panic Non-panicking fallback used after an unwind.
 * @return The operation result or the fallback value.
 */
fn guard<R>(body: impl FnOnce() -> R, on_panic: impl FnOnce() -> R) -> R {
    match panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => on_panic(),
    }
}

/**
 * @fn `set_last_install_success(bool)`
 * @brief Publish the process-wide installation outcome.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param value Whether the install completed successfully.
 */
fn set_last_install_success(value: bool) {
    LAST_INSTALL_SUCCESS.store(value, Ordering::SeqCst);
}

/**
 * @fn `install_impl(*const c_char, *const c_char, &str, &str) -> *const c_char`
 * @brief Install an archive and publish the outcome before returning its text.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Null arguments take precedence over invalid UTF-8. The caller supplies the unwind
 * guard and serializes this operation with the host's installSucceeded read.
 *
 * ### :material-shield-lock: **Safety**
 *
 * Each non-null input pointer must remain readable through its NUL terminator and
 * unchanged for this call.
 *
 * @param archive_path Archive path, or null to report an argument error.
 * @param mod_path Destination path, or null to report an argument error.
 * @param json_path Selection file path; empty enables archive-side lookup.
 * @param tag Subsystem name used in failure logs.
 * @return An owned destination path on success or error text on failure.
 */
unsafe fn install_impl(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: &str,
    tag: &str,
) -> *const c_char {
    // Safety: the caller of install_impl guarantees each pointer is null or a valid nul-terminated
    // string that stays alive for this call.
    match (unsafe { borrow_arg(archive_path) }, unsafe {
        borrow_arg(mod_path)
    }) {
        (ArgStr::Null, _) | (_, ArgStr::Null) => {
            set_last_install_success(false);
            owned_cstring("archivePath and modPath must not be null")
        }
        (ArgStr::InvalidUtf8, _) | (_, ArgStr::InvalidUtf8) => {
            // A path that is not valid UTF-8 is rejected here rather than forwarded as raw bytes,
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
                    // The caller sees the InstallError Display text verbatim.
                    owned_cstring(&err.to_string())
                }
            }
        }
    }
}

/**
 * @fn `getApiVersion() -> *const c_char`
 * @brief Report the ABI version without transferring ownership.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @return A static NUL-terminated string valid while the DLL remains loaded.
 * @warning Do not pass this pointer to freeResult.
 */
#[unsafe(no_mangle)]
pub extern "C" fn getApiVersion() -> *const c_char {
    // returning a static pointer cannot panic; the guard is here so every export has the same
    // shape.
    guard(|| API_VERSION_C.as_ptr(), || API_VERSION_C.as_ptr())
}

/**
 * @fn `setLogCallback(Option<unsafe extern "C" fn(*const c_char)>)`
 * @brief Replace the process-wide host log callback.
 * @author Alex (<https://github.com/lextpf>)
 *
 * A callback replaces file output; console output continues. It receives borrowed,
 * NUL-terminated UTF-8 text that is valid only during the callback. Copy the text to
 * retain it. Calls can arrive concurrently from engine threads and must not unwind.
 *
 * Clearing or replacing the callback does not wait for calls already in progress.
 * Keep the callback and its host state alive until those calls finish.
 *
 * @param callback Host callback, or null to restore file logging.
 */
#[unsafe(no_mangle)]
pub extern "C" fn setLogCallback(callback: Option<unsafe extern "C" fn(*const c_char)>) {
    guard(
        || {
            // The logger stores the pointer in a lock-free atomic, so this never blocks a logging
            // thread. A null callback re-enables file logging.
            crate::logger::Logger::instance().set_callback(callback);
        },
        || {},
    );
}

/**
 * @fn `install(*const c_char, *const c_char) -> *const c_char`
 * @brief Install an archive with automatic selection-file lookup.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The service looks for a sibling JSON file with the archive stem. Missing or unreadable
 * selections permit default installation behavior. Failure can leave partial output.
 * Serialize the call and the following installSucceeded read against all other installs.
 *
 * ### :material-shield-lock: **Safety**
 *
 * Each non-null argument must remain readable through its NUL terminator and unchanged
 * for this call. Release the returned pointer exactly once with freeResult.
 *
 * @param archive_path UTF-8 archive path; null or invalid UTF-8 reports failure.
 * @param mod_path UTF-8 destination path; existing files can be overwritten.
 * @return An owned destination path on success or error text on failure.
 * @see installSucceeded
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn install(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // Safety: install_impl borrows pointers under install's nul-or-valid contract.
        // An empty json_path selects the documented sidecar lookup.
        || unsafe { install_impl(archive_path, mod_path, "", "install") },
        || {
            Logger::instance().log_error("[install] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/**
 * @fn `installWithConfig(*const c_char, *const c_char, *const c_char) -> *const c_char`
 * @brief Install an archive with an optional explicit selection file.
 * @author Alex (<https://github.com/lextpf>)
 *
 * A nonempty JSON path suppresses archive-side lookup, even if the file is unreadable.
 * Missing or unreadable selections permit default installation behavior. Failure can
 * leave partial output. Serialize the call and the following installSucceeded read
 * against all other installs.
 *
 * ### :material-shield-lock: **Safety**
 *
 * Each non-null argument must remain readable through its NUL terminator and unchanged
 * for this call. Release the returned pointer exactly once with freeResult.
 *
 * @param archive_path UTF-8 archive path; null or invalid UTF-8 reports failure.
 * @param mod_path UTF-8 destination path; existing files can be overwritten.
 * @param json_path UTF-8 selection-file path; null or empty enables archive-side lookup.
 * @return An owned destination path on success or error text on failure.
 * @see installSucceeded
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn installWithConfig(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: *const c_char,
) -> *const c_char {
    guard(
        // Safety: same nul-or-valid contract as install().
        || {
            // A null jsonPath is coerced to the empty string. Invalid UTF-8 is rejected, consistent
            // with the other two arguments.
            // Safety: `json_path` is null or a valid nul-terminated string that the caller keeps
            // alive for this call.
            let json = match unsafe { borrow_arg(json_path) } {
                ArgStr::Null => "",
                ArgStr::Str(s) => s,
                ArgStr::InvalidUtf8 => {
                    set_last_install_success(false);
                    return owned_cstring("Unknown fatal error during installation");
                }
            };
            // Safety: both pointers are this export's own arguments, forwarded unchanged under the
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
 * @fn `inferFomodSelections(*const c_char, *const c_char) -> *const c_char`
 * @brief Infer FOMOD selections from an archive and its installed files.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Successful inference returns schema-v2 JSON. A null argument returns an argument-error
 * message; other failures return an empty string. Validate the result before parsing it.
 *
 * ### :material-shield-lock: **Safety**
 *
 * Each non-null argument must remain readable through its NUL terminator and unchanged
 * for this call. Release the returned pointer exactly once with freeResult.
 *
 * @param archive_path UTF-8 path to the source archive.
 * @param mod_path UTF-8 path to the installed mod used as evidence.
 * @return Owned JSON, an empty string, or the null-argument error message.
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn inferFomodSelections(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // Safety: inferFomodSelections borrows both pointers under its nul-or-valid contract.
        || match (unsafe { borrow_arg(archive_path) }, unsafe {
            borrow_arg(mod_path)
        }) {
            (ArgStr::Null, _) | (_, ArgStr::Null) => {
                owned_cstring("archivePath and modPath must not be null")
            }
            // Both non-null and valid UTF-8: run the orchestrator. It returns "" on any internal
            // failure, so no Result crosses FFI.
            (ArgStr::Str(archive), ArgStr::Str(modp)) => {
                let service = crate::fomod_inference_service::FomodInferenceService::new();
                owned_cstring(&service.infer_selections(archive, modp))
            }
            // A path that is not valid UTF-8 yields the same "" as any other failure.
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
 * @fn `installSucceeded() -> bool`
 * @brief Read the most recently published installation outcome.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The flag is process-wide and initially false. It is neither a completion signal nor
 * a per-thread result. Read it after the install returns, within the same host lock.
 *
 * @return True when the last published install outcome was success.
 */
#[unsafe(no_mangle)]
pub extern "C" fn installSucceeded() -> bool {
    guard(|| LAST_INSTALL_SUCCESS.load(Ordering::SeqCst), || false)
}

/**
 * @fn `freeResult(*const c_char)`
 * @brief Release a string allocated by an owned-string export.
 * @author Alex (<https://github.com/lextpf>)
 *
 * ### :material-shield-lock: **Safety**
 *
 * The pointer must be null or an unchanged, unfreed result from install,
 * installWithConfig, inferFomodSelections, or resolveModArchive in this loaded DLL.
 * Do not pass foreign allocations or the static getApiVersion pointer.
 *
 * @param result Owned result to release; null has no effect.
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeResult(result: *const c_char) {
    guard(
        || {
            if result.is_null() {
                return;
            }
            // Safety: per the contract above, `result` came from owned_cstring
            // in this DLL (CString::into_raw), so reclaiming it with the
            // matching CString::from_raw is sound.
            unsafe { drop(CString::from_raw(result.cast_mut())) };
        },
        || {},
    );
}

/**
 * @fn `resolveModArchive(*const c_char, *const c_char, *const c_char) -> *const c_char`
 * @brief Resolve a metadata archive path through the shared search order.
 * @author Alex (<https://github.com/lextpf>)
 *
 * An absolute metadata path is checked as supplied. Relative paths use the candidate
 * order in archive_resolver. A null or invalid UTF-8 mods_dir skips its candidates;
 * the same errors in either required argument return an empty result.
 *
 * ### :material-shield-lock: **Safety**
 *
 * Each non-null argument must remain readable through its NUL terminator and unchanged
 * for this call. Release the returned pointer exactly once with freeResult.
 *
 * @param installation_file UTF-8 archive value from mod metadata.
 * @param mod_folder UTF-8 installed-mod directory used for relative candidates.
 * @param mods_dir UTF-8 MO2 mods directory; null or empty omits its candidates.
 * @return An owned resolved path, or an owned empty string on failure.
 */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn resolveModArchive(
    installation_file: *const c_char,
    mod_folder: *const c_char,
    mods_dir: *const c_char,
) -> *const c_char {
    guard(
        // Safety: resolveModArchive borrows all three pointers under its nul-or-valid contract.
        || {
            // Only installationFile and modFolder are null-checked; a null modsDir is coerced to an
            // empty path and merely skips candidates 4-6.
            // Safety: each pointer is null or a valid nul-terminated string that the caller keeps
            // alive for this call.
            let (file, folder) = match (unsafe { borrow_arg(installation_file) }, unsafe {
                borrow_arg(mod_folder)
            }) {
                (ArgStr::Str(f), ArgStr::Str(d)) => (f, d),
                // Null and non-UTF-8 both yield "".
                _ => return owned_cstring(""),
            };
            // Safety: `mods_dir` carries the same null-or-valid contract.
            let mods = match unsafe { borrow_arg(mods_dir) } {
                ArgStr::Str(s) => s,
                ArgStr::Null | ArgStr::InvalidUtf8 => "",
            };

            let resolved = crate::archive_resolver::resolve_mod_archive(
                file,
                std::path::Path::new(folder),
                std::path::Path::new(mods),
            );
            // An unresolved path is empty and already stringifies to "", which is the documented
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

        // An empty string is still a valid non-null pointer.
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

    // This is the only test that touches LAST_INSTALL_SUCCESS, so the shared process-global flag
    // cannot race against a parallel test. Keep it that way.
    #[test]
    fn install_wires_the_service_and_tracks_the_success_flag() {
        // Null inputs -> the null-argument message; flag cleared.
        let msg = unsafe { take_owned(install(std::ptr::null(), std::ptr::null())) };
        assert_eq!(msg, "archivePath and modPath must not be null");
        assert!(!installSucceeded());

        // A real failure carries the InstallError text through unchanged.
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

        // A null jsonPath is coerced to "" and must not change the outcome.
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

        // The flag is sticky across other exports: neither inferFomodSelections nor
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
        // Null installationFile or modFolder -> "".
        let out = unsafe {
            take_owned(resolveModArchive(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, "");

        // An unresolvable name yields "" because the resolver missed every candidate, which is
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

        // A resolvable archive under the mod folder comes back, which is what the MO2 plugin
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
