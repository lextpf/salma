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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// Registered log callback stored as a raw function-pointer address (0 = none).
/// Mirrors `Logger::set_callback`'s lock-free atomic store. No logger is wired
/// up yet (that arrives in a later task); the pointer is simply retained so
/// registering and clearing are observable and crash-free.
static LOG_CALLBACK: AtomicUsize = AtomicUsize::new(0);

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

/// Shared body for [`install`] and [`installWithConfig`]. In Milestone 1 both
/// behave identically with respect to the archive/mod inputs; the install
/// replay (and `jsonPath` handling for `installWithConfig`) lands in a later
/// task.
///
/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated
/// C string.
unsafe fn install_stub(archive_path: *const c_char, mod_path: *const c_char) -> *const c_char {
    match (unsafe { borrow_arg(archive_path) }, unsafe {
        borrow_arg(mod_path)
    }) {
        (ArgStr::Null, _) | (_, ArgStr::Null) => {
            set_last_install_success(false);
            owned_cstring("archivePath and modPath must not be null")
        }
        (ArgStr::InvalidUtf8, _) | (_, ArgStr::InvalidUtf8) => {
            // Invalid UTF-8 is handled like a caught exception: the C++
            // catch-all failure string, flag cleared.
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        }
        (ArgStr::Str(_archive), ArgStr::Str(_mod)) => {
            // TODO(task 14/15): forward to InstallationService::install_mod.
            set_last_install_success(false);
            owned_cstring("install not yet implemented in mo2_salma_rs")
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
            // Store the callback address (0 == cleared). Lock-free, mirroring
            // Logger::set_callback. No logger consumes it yet.
            let addr = match callback {
                Some(f) => f as usize,
                None => 0,
            };
            LOG_CALLBACK.store(addr, Ordering::SeqCst);
            // Nothing reads LOG_CALLBACK until the logger lands (Task 17), so
            // the release optimizer would otherwise elide the store above as
            // dead. black_box keeps the round-tripped value live, so the
            // pointer is genuinely retained in the DLL as the ABI contract
            // requires. Removed once a real reader exists.
            std::hint::black_box(LOG_CALLBACK.load(Ordering::SeqCst));
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
        // SAFETY: pointers are only borrowed inside install_stub, which upholds
        // the same nul-or-valid contract this function documents.
        || unsafe { install_stub(archive_path, mod_path) },
        || {
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated
/// UTF-8 C string. `json_path` may be null (ignored in Milestone 1). The
/// returned non-null pointer must be released with [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn installWithConfig(
    archive_path: *const c_char,
    mod_path: *const c_char,
    _json_path: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: same nul-or-valid contract as install(). json_path is unused
        // until the install replay is ported.
        || unsafe { install_stub(archive_path, mod_path) },
        || {
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
        || owned_cstring(""),
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
    _installation_file: *const c_char,
    _mod_folder: *const c_char,
    _mods_dir: *const c_char,
) -> *const c_char {
    // The C++ implementation returns "" both when installationFile or modFolder
    // is null and on a resolution miss, so every input maps to "" in this stub.
    // TODO(task 15): forward valid inputs to mo2core::resolve_mod_archive.
    guard(|| owned_cstring(""), || owned_cstring(""))
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
    fn infer_stub_null_and_placeholder() {
        let msg = unsafe { take_owned(inferFomodSelections(std::ptr::null(), std::ptr::null())) };
        assert_eq!(msg, "archivePath and modPath must not be null");

        let archive = CString::new("archive.7z").unwrap();
        let mod_dir = CString::new("mod").unwrap();
        let out = unsafe { take_owned(inferFomodSelections(archive.as_ptr(), mod_dir.as_ptr())) };
        assert_eq!(out, "");
    }

    // This is the ONLY test that touches LAST_INSTALL_SUCCESS, so the shared
    // flag cannot race against a parallel test.
    #[test]
    fn install_stub_reports_not_implemented_and_flag_stays_false() {
        // Null inputs -> the exact C++ null-argument message; flag stays false.
        let msg = unsafe { take_owned(install(std::ptr::null(), std::ptr::null())) };
        assert_eq!(msg, "archivePath and modPath must not be null");
        assert!(!installSucceeded());

        let archive = CString::new("archive.7z").unwrap();
        let mod_dir = CString::new("mod").unwrap();

        // Valid inputs -> Milestone 1 placeholder; flag still false.
        let msg = unsafe { take_owned(install(archive.as_ptr(), mod_dir.as_ptr())) };
        assert_eq!(msg, "install not yet implemented in mo2_salma_rs");
        assert!(!installSucceeded());

        // installWithConfig shares the same stub; jsonPath is ignored for now.
        let cfg = CString::new("selections.json").unwrap();
        let msg = unsafe {
            take_owned(installWithConfig(
                archive.as_ptr(),
                mod_dir.as_ptr(),
                cfg.as_ptr(),
            ))
        };
        assert_eq!(msg, "install not yet implemented in mo2_salma_rs");
        assert!(!installSucceeded());
    }

    #[test]
    fn resolve_mod_archive_stub_returns_empty() {
        let out = unsafe {
            take_owned(resolveModArchive(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, "");

        let file = CString::new("mod.7z").unwrap();
        let folder = CString::new("C:/mods/MyMod").unwrap();
        let out = unsafe {
            take_owned(resolveModArchive(
                file.as_ptr(),
                folder.as_ptr(),
                std::ptr::null(),
            ))
        };
        assert_eq!(out, "");
    }

    // This is the ONLY test that touches LOG_CALLBACK, so the shared slot
    // cannot race against a parallel test.
    #[test]
    fn set_log_callback_stores_and_clears() {
        setLogCallback(Some(noop_log));
        assert_ne!(LOG_CALLBACK.load(Ordering::SeqCst), 0);
        setLogCallback(None);
        assert_eq!(LOG_CALLBACK.load(Ordering::SeqCst), 0);
    }
}
