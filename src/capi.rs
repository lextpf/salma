//! Flat C ABI boundary for the salma engine.
//!
//! The eight `#[unsafe(no_mangle)] pub extern "C"` functions below are the only
//! symbols this DLL exports. Two consumers bind them, and neither may drift
//! from the other:
//!
//! - `scripts/mo2-salma.py`, function `_configure_dll`, declares the `ctypes`
//!   `argtypes` / `restype` the MO2 plugin uses.
//! - `src/SalmaEngine.cpp` declares the matching `__cdecl` typedefs
//!   (`FnInstallWithConfig`, `FnInferSelections`, `FnResolveModArchive`,
//!   `FnFreeResult`, `FnInstallSucceeded`, `FnGetApiVersion`) and resolves the
//!   symbols with `LoadLibrary`. It binds six of the eight; it never calls
//!   `install` or `setLogCallback`.
//!
//! Those two files are the live, in-tree definition of this ABI. A signature
//! change here that is not mirrored in both breaks one consumer silently.
//!
//! ## Exports
//!
//! ```text
//!  export                | returns        | free with  | value on failure      | install flag
//!  ----------------------|----------------|------------|-----------------------|-------------
//!  getApiVersion         | static *const  | never free | cannot fail           | untouched
//!  setLogCallback        | ()             | -          | silently ignored      | untouched
//!  install               | owned *const   | freeResult | error text (see note) | writes
//!  installWithConfig     | owned *const   | freeResult | error text (see note) | writes
//!  inferFomodSelections  | owned *const   | freeResult | "" (empty string)     | untouched
//!  installSucceeded      | bool           | -          | false                 | reads
//!  freeResult            | ()             | -          | null is a no-op       | untouched
//!  resolveModArchive     | owned *const   | freeResult | "" (empty string)     | untouched
//! ```
//!
//! Note on the two install exports: the returned string is the installed mod
//! directory path on success and a human-readable error message on failure, and
//! the string alone does not say which. [`installSucceeded`] is the only
//! discriminator. The fixed failure strings are
//! `"archivePath and modPath must not be null"` when either path pointer is
//! null, and `"Unknown fatal error during installation"` when an argument is not
//! valid UTF-8 or a panic reaches [`guard`]; any other text comes from
//! `InstallError`. One ordering exception: [`installWithConfig`] classifies
//! `jsonPath` before the two path pointers, so a non-UTF-8 `jsonPath` yields the
//! catch-all string even when `archivePath` or `modPath` is also null.
//!
//! [`inferFomodSelections`] returns `""` for every failure except a null path
//! argument, which yields `"archivePath and modPath must not be null"`.
//! [`resolveModArchive`] returns `""` for every failure, null arguments
//! included. No `Result` and no error code crosses this boundary.
//!
//! ## Ownership
//!
//! Every non-null `*const c_char` handed out by [`install`],
//! [`installWithConfig`], [`inferFomodSelections`], and [`resolveModArchive`] is
//! heap-allocated and must be released with [`freeResult`]. Those four never
//! return null, not even on failure, so the caller always owns a string to free.
//! [`getApiVersion`] is the sole exception: it returns a pointer to static
//! storage that is never allocated and must never be freed.
//!
//! ## Panics
//!
//! No panic may unwind across the FFI boundary. A panic that escapes a
//! non-unwinding `extern "C"` function aborts the process. That is defined
//! behavior, not undefined behavior: it has been the rule since Rust 1.71
//! (RFC 2945), and this crate pins `edition = "2024"` with
//! `rust-version = "1.85"`. Every export therefore routes through [`guard`] and
//! returns that export's caught-error value instead of letting the process die.
//!
//! ## Thread safety
//!
//! Every export is safe to call from any thread, but two installs must never
//! overlap. Two pieces of process-global mutable state back the install path:
//!
//! - The success flag that [`installSucceeded`] reads. [`install`] and
//!   [`installWithConfig`] each build their own `InstallationService`, but both
//!   write that one flag, so the later install overwrites the earlier verdict.
//! - The sticky disk-full marker in [`crate::file_operations`], which
//!   [`crate::installation_service::InstallationService::install_mod`] resets on
//!   entry and reads on exit to turn a partially copied mod into a hard failure.
//!   Two overlapping installs clear and observe each other's disk pressure, so a
//!   disk-full failure is attributed to the wrong install or lost entirely and a
//!   half-copied mod is reported as installed.
//!
//! A host must therefore serialize the whole install call together with the flag
//! read, and that obligation holds even for a host that never calls
//! [`installSucceeded`]. `mo2-server` does exactly that with `g_install_mutex`
//! in `src/SalmaEngine.cpp`. This section is the rule for callers of the ABI;
//! [`crate::installation_service`] states the same hazard from the install side.
//!
//! The host log callback registered through [`setLogCallback`] is process-global
//! too, and it can be invoked concurrently from any thread that logs.
//!
//! `PARITY-NOTES.md` records why the odd-looking corners of this boundary are
//! shaped the way they are.

// The C ABI export names are camelCase because that is what both binders look
// up: scripts/mo2-salma.py through ctypes and src/SalmaEngine.cpp through
// GetProcAddress. The symbol emitted by #[unsafe(no_mangle)] is the Rust
// identifier itself, so these identifiers cannot be renamed to snake_case
// without changing the exported symbol table.
#![allow(non_snake_case)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::Logger;

/// Stable ABI version string, `major.minor.patch`. A different major is an
/// incompatible ABI.
///
/// Four places must change together on a major bump, and the unit test
/// `version_constant_major_and_sync` covers only the two in this file:
///
/// 1. [`API_VERSION_C`], the nul-terminated form [`getApiVersion`] hands out.
///    The test asserts it holds the same bytes as this constant.
/// 2. [`MO2_SALMA_API_MAJOR`], the leading digit. The test asserts it equals
///    `1` and that this constant starts with it, so a bump edits the test too.
/// 3. `EXPECTED_API_MAJOR` in `scripts/mo2-salma.py`.
/// 4. `EXPECTED_API_MAJOR` in `scripts/common.py`.
///
/// Both Python consumers compare the DLL's reported major against their own
/// constant and refuse to load on a mismatch, so bumping the major here alone
/// ships a DLL that the MO2 plugin and the harness scripts both reject at load
/// time.
///
/// The literal spelling of this line is load-bearing as well: `CMakeLists.txt`
/// regex-matches `MO2_SALMA_API_VERSION: &str = "<x.y.z>"` out of this file for
/// the cpack package version, and raises a fatal error when the pattern stops
/// matching.
pub const MO2_SALMA_API_VERSION: &str = "1.2.0";

/// Leading major-version digit of [`MO2_SALMA_API_VERSION`]. Two different major
/// values are incompatible ABIs; see that constant for every place a bump has to
/// be mirrored into.
pub const MO2_SALMA_API_MAJOR: &str = "1";

/// Nul-terminated form returned by [`getApiVersion`]. Static storage: the
/// pointer is valid for the whole process lifetime and must never be passed to
/// [`freeResult`].
static API_VERSION_C: &CStr = c"1.2.0";

/// Process-global "did the last install succeed" flag. The last install wins,
/// and `SeqCst` store/load makes its value visible on every thread.
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
/// The returned pointer is non-null and heap-owned. A C string cannot carry an
/// interior NUL, so if `s` holds one the result is truncated at the first NUL.
/// Never panics.
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

/// Shared body for [`install`] and [`installWithConfig`], which differ only in
/// the `json_path` they forward and the subsystem tag they log under.
///
/// Classifies both path arguments, then either runs an install or substitutes a
/// failure value. Every arm returns a non-null owned string and every arm writes
/// the process-global success flag:
///
/// ```text
///  archive_path  mod_path    -> returned string                            flag
///  ------------  ----------  --------------------------------------------  -----
///  null          any         "archivePath and modPath must not be null"    false
///  any           null        "archivePath and modPath must not be null"    false
///  bad UTF-8     any         "Unknown fatal error during installation"     false
///  any           bad UTF-8   "Unknown fatal error during installation"     false
///  valid         valid       Ok  -> the installed mod directory path       true
///                            Err -> the InstallError Display text          false
/// ```
///
/// The null arm is matched first, so a null pointer wins over a non-UTF-8 one.
/// That precedence covers these two arguments only: [`installWithConfig`]
/// classifies its `json_path` before calling in, so a non-UTF-8 `json_path`
/// returns the catch-all string without either path pointer being classified.
/// A panic anywhere inside produces the same
/// `"Unknown fatal error during installation"` plus a false flag, from the
/// caller's [`guard`] fallback.
///
/// The success flag is true if and only if `install_mod` returned `Ok(_)`. The
/// `Ok` value itself is never inspected, so `Ok("")`, reachable by passing an
/// empty `modPath`, also sets it true. The predicate is "the install returned
/// without error", not "the returned mod path is usable"; see PARITY-NOTES.md.
/// An `Err` return sets the flag false, as do a null or non-UTF-8 argument,
/// which never reach `install_mod` at all.
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
            // A path that is not valid UTF-8 is rejected here rather than
            // forwarded as raw bytes, and reports the same catch-all failure
            // string a panic would. See PARITY-NOTES.md.
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

/// The ABI version string, as a nul-terminated C string.
///
/// This is the one export whose return value the caller does not own. The
/// pointer addresses static storage holding [`MO2_SALMA_API_VERSION`], is valid
/// for the whole process lifetime, is never heap-allocated, and must not be
/// passed to [`freeResult`]. Doing so is undefined behavior.
///
/// Never returns null and cannot fail. It is safe to call from any thread at any
/// time, including before any other export.
#[unsafe(no_mangle)]
pub extern "C" fn getApiVersion() -> *const c_char {
    // Returning a static pointer cannot panic; the guard is here so every export
    // has the same shape.
    guard(|| API_VERSION_C.as_ptr(), || API_VERSION_C.as_ptr())
}

/// Register the host log callback, or clear it by passing null.
///
/// While a callback is registered it replaces file logging: the engine writes
/// nothing to `logs/salma.log` and delivers every message to the callback
/// instead. The console echo (stdout for info and warning, stderr for error) is
/// unaffected either way. Passing null restores file logging.
///
/// The stored callback takes effect for the whole process, not for one thread
/// and not for one call.
///
/// Caller obligations:
///
/// - **Thread safety.** The callback runs outside the logger's state mutex, on
///   whichever thread called into the engine, so two threads can be inside it at
///   the same time. The implementation must be thread-safe. salma does not
///   serialize it.
/// - **Pointer lifetime.** The function pointer is stored as a raw address and
///   is kept until it is replaced or cleared. It must stay valid until
///   `setLogCallback(null)` is called. Unloading the module that owns the
///   callback while it is still registered is undefined behavior.
/// - **No re-entrant logging.** A callback that logs back into the engine on the
///   same thread is dropped, not invoked: the logger prints
///   `[Logger] Re-entrant callback dropped: <message>` to stderr and returns.
///   This guard is what stops an infinite recursion.
///
/// The message arrives as a nul-terminated C string that is only valid for the
/// duration of the call, so a callback that keeps it must copy it. A message
/// containing an interior NUL is truncated at the first NUL, because a C string
/// cannot carry one. A callback that panics or throws is caught; the logger
/// prints `[Logger] Callback threw for: <message>` and the log call site
/// survives.
///
/// This export cannot fail and returns nothing.
#[unsafe(no_mangle)]
pub extern "C" fn setLogCallback(callback: Option<unsafe extern "C" fn(*const c_char)>) {
    guard(
        || {
            // The logger stores the pointer in a lock-free atomic, so this never
            // blocks a logging thread. A null callback re-enables file logging.
            crate::logger::Logger::instance().set_callback(callback);
        },
        || {},
    );
}

/// Install `archive_path` into `mod_path`, naming no selections JSON.
///
/// Always returns a non-null, heap-allocated string that the caller must release
/// with [`freeResult`]. On success the string is the installed mod directory
/// path; on failure it is a human-readable error message. The string alone does
/// not say which, so read [`installSucceeded`] before interpreting it. The
/// failure strings are listed in this module's "Exports" section.
///
/// This call blocks: it extracts the archive to a temporary directory, replays
/// the install, and cleans up before returning. It is not async and it does I/O.
///
/// Naming no selections JSON does not mean installing with nothing selected.
/// This export forwards an empty selections path, and `InstallationService`
/// then derives a sibling `<archive stem>.json` and uses it when that file
/// exists, so an archive with a sidecar is installed with those selections.
///
/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated C
/// string. The returned non-null pointer must be released with [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn install(
    archive_path: *const c_char,
    mod_path: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: pointers are only borrowed inside install_impl, which upholds
        // the same nul-or-valid contract this function documents. The empty
        // json_path is the sidecar-lookup case described above.
        || unsafe { install_impl(archive_path, mod_path, "", "install") },
        || {
            Logger::instance().log_error("[install] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/// Install `archive_path` into `mod_path`, using the selections JSON at
/// `json_path`.
///
/// Return contract identical to [`install`]: always a non-null heap string that
/// the caller must release with [`freeResult`], carrying the installed mod
/// directory path on success and an error message on failure, with
/// [`installSucceeded`] as the only discriminator. The failure strings are
/// listed in this module's "Exports" section.
///
/// This is the export `mo2-server` uses for every install; it never calls
/// [`install`].
///
/// This call blocks and does I/O, exactly as [`install`] does.
///
/// # Safety
///
/// `archive_path` and `mod_path` must each be null or a valid nul-terminated C
/// string. `json_path` may be null, which is coerced to `""`; an empty or null
/// `json_path` still lets the service pick up a sibling `<archive stem>.json`,
/// see [`install`]. The returned non-null pointer must be released with
/// [`freeResult`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn installWithConfig(
    archive_path: *const c_char,
    mod_path: *const c_char,
    json_path: *const c_char,
) -> *const c_char {
    guard(
        // SAFETY: same nul-or-valid contract as install().
        || {
            // A null jsonPath is coerced to the empty string. Invalid UTF-8 is
            // rejected, consistent with the other two arguments.
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
            Logger::instance().log_error("[installWithConfig] Fatal error: unknown exception");
            set_last_install_success(false);
            owned_cstring("Unknown fatal error during installation")
        },
    )
}

/// Recover which FOMOD options the mod at `mod_path` was installed from, by
/// comparing it against the options in `archive_path`.
///
/// Always returns a non-null, heap-allocated string that the caller must release
/// with [`freeResult`]. On success it holds schema-v2 JSON. On any failure it
/// holds `""`, because no `Result` crosses this boundary; the one exception is a
/// null argument, which yields `"archivePath and modPath must not be null"`.
/// An empty string therefore means "could not infer", never "inferred nothing to
/// select".
///
/// This call blocks and does I/O: it lists the archive, parses
/// `fomod/ModuleConfig.xml`, scans the installed tree, and runs the CSP solver.
/// It does not write to `mod_path` and does not touch the install success flag.
///
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
            // on any internal failure, so no Result crosses FFI.
            (ArgStr::Str(archive), ArgStr::Str(modp)) => {
                let service = crate::fomod_inference_service::FomodInferenceService::new();
                owned_cstring(&service.infer_selections(archive, modp))
            }
            // A path that is not valid UTF-8 yields the same "" as any other
            // failure.
            _ => owned_cstring(""),
        },
        || {
            // Unreachable in practice: infer_selections swallows every internal
            // failure and returns "" rather than propagating.
            Logger::instance().log_error("[infer] Fatal error: unknown exception");
            owned_cstring("")
        },
    )
}

/// Whether the most recent [`install`] or [`installWithConfig`] call in this
/// process succeeded.
///
/// The value is one process-global atomic flag, not a per-call and not a
/// per-thread result. Every install on any thread overwrites it, last write
/// wins. Nothing else writes it: [`inferFomodSelections`], [`resolveModArchive`],
/// [`getApiVersion`], [`setLogCallback`] and [`freeResult`] all leave it
/// untouched, so the flag stays sticky until the next install.
///
/// A host that can run installs concurrently must serialize the install call
/// together with this read, or one install observes another install's result.
/// `mo2-server` runs installs on overlapping background jobs and does exactly
/// that with `g_install_mutex` in `src/SalmaEngine.cpp`.
///
/// The flag is stored and loaded with `SeqCst`, so the value an install writes
/// is visible to every other thread. Reading it before any install has run
/// yields `false`. Returns `false` if the atomic load panics, which cannot
/// happen in practice.
#[unsafe(no_mangle)]
pub extern "C" fn installSucceeded() -> bool {
    guard(|| LAST_INSTALL_SUCCESS.load(Ordering::SeqCst), || false)
}

/// Release a string previously returned by one of this DLL's owned-string
/// exports.
///
/// A null pointer is a no-op. Each pointer must be freed exactly once; freeing
/// it twice is undefined behavior. This export never fails and returns nothing.
///
/// # Safety
///
/// `result` must be null, or a pointer previously returned by one of this
/// DLL's owned-string exports ([`install`], [`installWithConfig`],
/// [`inferFomodSelections`], [`resolveModArchive`]) and not yet freed. Passing
/// any other pointer (a foreign allocation, a stack pointer, an already-freed
/// pointer, or the static [`getApiVersion`] pointer) is undefined behavior:
/// allocation and release must stay on one side of the boundary, and this DLL
/// does both with Rust's allocator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn freeResult(result: *const c_char) {
    guard(
        || {
            if result.is_null() {
                return;
            }
            // SAFETY: per the contract above, `result` came from owned_cstring
            // in this DLL (CString::into_raw), so reclaiming it with the
            // matching CString::from_raw is sound.
            unsafe { drop(CString::from_raw(result.cast_mut())) };
        },
        || {},
    );
}

/// Resolve the archive a mod folder was installed from, by walking the
/// `installationFile` fallback chain.
///
/// Always returns a non-null, heap-allocated string that the caller must release
/// with [`freeResult`]. It holds the resolved archive path on success and `""`
/// on every failure, including a null or non-UTF-8 `installation_file` or
/// `mod_folder`. This export never touches the install success flag.
///
/// `mods_dir` is treated differently from the other two arguments on purpose: a
/// null or non-UTF-8 `mods_dir` is coerced to an empty path, which only skips
/// the candidates that need it instead of failing the whole call. A caller with
/// no mods directory still gets the candidates that do not need one.
///
/// This call blocks and touches the file system: it probes candidate paths for
/// existence.
///
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
            // 4-6. The asymmetry is deliberate, see the doc comment above.
            let (file, folder) = match (unsafe { borrow_arg(installation_file) }, unsafe {
                borrow_arg(mod_folder)
            }) {
                (ArgStr::Str(f), ArgStr::Str(d)) => (f, d),
                // Null and non-UTF-8 both yield "".
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
            // An unresolved path is empty and already stringifies to "", which
            // is the documented failure value; no extra branch is needed.
            owned_cstring(&resolved.to_string_lossy())
        },
        || {
            // Unreachable in practice: resolve_mod_archive returns an empty path
            // instead of panicking.
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
        // The leading digit is the major version.
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
        // Deliberately not freed: it points at static storage.
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

    // This is the only test that touches LAST_INSTALL_SUCCESS, so the shared
    // process-global flag cannot race against a parallel test. Keep it that way.
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

        // A succeeding install sets the flag and returns mod_path verbatim.
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

        // The flag is sticky across other exports: neither inferFomodSelections
        // nor resolveModArchive writes it.
        let _ = unsafe { take_owned(inferFomodSelections(c_archive.as_ptr(), c_out.as_ptr())) };
        assert!(installSucceeded(), "infer must not touch the install flag");

        // Restore the shared flag so test ordering cannot leak this success.
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

        // An unresolvable name yields "" because the resolver missed every
        // candidate, which is indistinguishable from a bad argument.
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

        // A resolvable archive under the mod folder comes back, which is what
        // the MO2 plugin depends on: it gates on hasattr and never falls back
        // once the symbol exists.
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

    /// The export must reach the real logger, not a private slot. This is the
    /// only test that touches the process-global callback; cargo runs the tests
    /// in this binary in parallel, so a second one would see this one's writes.
    /// Register, check and clear within this body.
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
