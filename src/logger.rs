//! Process-wide logger: `logs/salma.log` next to the DLL, plus a host callback.
//!
//! One [`OnceLock`]-backed instance per process, reached through
//! [`Logger::instance`]. Construction never fails, so no call site has to handle
//! a missing logger.
//!
//! ## Output routing
//!
//! A registered callback replaces file logging; it does not duplicate it.
//! `setLogCallback(null)` restores file logging. The console echo happens either
//! way.
//!
//! ```text
//!  callback registered? | console                   | logs/salma.log | callback
//!  ---------------------|---------------------------|----------------|------------
//!  no                   | stdout (info, warning),   | formatted line | -
//!                       | stderr (error)            |                |
//!  yes                  | stdout (info, warning),   | nothing        | raw message
//!                       | stderr (error)            |                |
//! ```
//!
//! Two rules the table cannot carry:
//!
//! - The console echo and the callback both receive the raw message, with no
//!   timestamp and no level prefix. Only the file line carries those.
//! - A callback that logs re-entrantly on the same thread is dropped, with a
//!   diagnostic on stderr, instead of recursing until the stack dies.
//!
//! The file write happens under the state mutex. The console echo and the
//! callback both run outside it, so two threads can interleave there, and a slow
//! host callback never blocks another thread's file write.
//!
//! ## Line format
//!
//! `YYYY-MM-DD HH:MM:SS.mmm LEVEL message`, where `LEVEL` is `INFO`, `WARNING`
//! or `ERROR`. The timestamp is local time on Windows (`GetLocalTime`). On every
//! other platform it is UTC, because `std` has no local-time conversion; see
//! `now_local`. salma ships Windows-only, and the non-Windows arm exists only so
//! the crate still compiles elsewhere.
//!
//! The file is opened in append mode and rotates at 10 MiB, keeping
//! `salma.log.1` through `.3`.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::utils::module_directory;

/// Host log callback.
///
/// The ABI spelling is the parameter type of [`crate::capi::setLogCallback`],
/// `Option<unsafe extern "C" fn(*const c_char)>`, which the MO2 plugin declares
/// as `ctypes.CFUNCTYPE(None, ctypes.c_char_p)`. All three must agree.
pub type LogCallback = unsafe extern "C" fn(*const c_char);

/// Rotate once the current file reaches this size, in bytes (10 MiB).
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

/// Keep `salma.log.1` through `salma.log.3`.
const MAX_ROTATED_FILES: u32 = 3;

/// The file handle and its byte counter, which only ever move together. Grouping
/// them in one struct is what makes the state mutex cover both.
struct FileState {
    /// Absolute path to the `logs` directory.
    directory: PathBuf,
    /// Append-mode handle, `None` when the file could not be opened.
    file: Option<File>,
    /// Approximate bytes written since the last rotation.
    bytes_written: u64,
}

/// The logger singleton.
pub struct Logger {
    state: Mutex<FileState>,
    /// Callback function-pointer address, 0 when cleared. Stored lock-free so
    /// `set_callback` never blocks a logging thread.
    callback: AtomicUsize,
}

/// An address inside this module, used to resolve the DLL that owns this code.
///
/// The log directory has to follow mo2-salma.dll, not the host executable: MO2
/// loads the DLL out of its plugins tree while the process is ModOrganizer.exe,
/// and the log belongs next to the DLL either way. Anchoring on a function
/// defined here is what makes that resolution point at the right module.
fn module_anchor() {}

// Re-entrancy guard for the host callback. A callback that itself logs would
// otherwise recurse until the stack overflows. The flag is per-thread because
// callbacks run on whichever thread logged.
thread_local! {
    static IN_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Severity tag written into the file. The console and callback paths receive
/// the raw message with no level prefix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        }
    }
}

impl Logger {
    /// The process-wide logger, constructed on first use.
    ///
    /// Construction resolves `<module dir>/logs`, creates it, and opens
    /// `salma.log` in append mode, seeding the rotation counter from the
    /// existing file size so a restart does not restart the rotation cycle.
    ///
    /// Construction always succeeds; there is no failing path and no `Result`.
    /// If the `logs` directory cannot be created, or `salma.log` cannot be
    /// opened, a `[Logger] ...` diagnostic goes to stderr and the logger is
    /// still returned, holding no file handle. In that degraded state every file
    /// write is discarded silently, while the console echo and the host callback
    /// keep working normally. No public accessor reports the state. A later
    /// successful [`Logger::clear_log`] re-opens the handle and ends it.
    pub fn instance() -> &'static Logger {
        LOGGER.get_or_init(Logger::new)
    }

    fn new() -> Logger {
        let anchor = module_anchor as *const () as *const core::ffi::c_void;
        let directory = module_directory(anchor).join("logs");

        if let Err(err) = fs::create_dir_all(&directory) {
            eprintln!(
                "[Logger] Failed to create log directory {}: {err}",
                directory.display()
            );
        }

        let (file, bytes_written) = open_append(&directory.join("salma.log"));
        Logger {
            state: Mutex::new(FileState {
                directory,
                file,
                bytes_written,
            }),
            callback: AtomicUsize::new(0),
        }
    }

    /// Register (or with `None`, clear) the host callback.
    ///
    /// A single lock-free atomic store, so it never contends with an in-flight
    /// log call and never blocks. Clearing re-enables file logging.
    ///
    /// The callback is process-global, not per-thread. Two caller obligations:
    ///
    /// - The callback must be thread-safe. It runs outside this logger's state
    ///   mutex, on whichever thread logged, so two threads can be inside it at
    ///   the same time.
    /// - The function pointer is stored as a raw address and must stay valid
    ///   until it is replaced or cleared with `None`. Leaving a dangling pointer
    ///   registered is undefined behavior.
    ///
    /// A callback that logs re-entrantly on the same thread is dropped, not
    /// invoked; see `invoke_callback`.
    pub fn set_callback(&self, callback: Option<LogCallback>) {
        let addr = match callback {
            Some(f) => f as usize,
            None => 0,
        };
        self.callback.store(addr, Ordering::SeqCst);
    }

    /// Whether a host callback is currently registered.
    ///
    /// Exists so the ABI test can assert that `setLogCallback` reached the
    /// logger without transmuting the stored address itself.
    pub fn has_callback(&self) -> bool {
        self.callback.load(Ordering::SeqCst) != 0
    }

    /// Currently registered callback, if any.
    fn callback(&self) -> Option<LogCallback> {
        let addr = self.callback.load(Ordering::SeqCst);
        if addr == 0 {
            None
        } else {
            // SAFETY: the address was stored from a `LogCallback` in
            // `set_callback` and function pointers are not invalidated by the
            // round trip through `usize`.
            Some(unsafe { std::mem::transmute::<usize, LogCallback>(addr) })
        }
    }

    /// Log at INFO.
    pub fn log(&self, message: &str) {
        self.emit(Level::Info, message);
    }

    /// Log at WARNING.
    pub fn log_warning(&self, message: &str) {
        self.emit(Level::Warning, message);
    }

    /// Log at ERROR.
    pub fn log_error(&self, message: &str) {
        self.emit(Level::Error, message);
    }

    /// The shared body of the three level methods.
    fn emit(&self, level: Level, message: &str) {
        // 1. Under the lock: snapshot the callback, and write the file line
        //    only when no callback is registered.
        let cb = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let cb = self.callback();
            if cb.is_none() {
                state.write_line(level, message);
            }
            cb
        };

        // 2. Console echo, outside the lock: interleaved console output is
        //    acceptable, holding the mutex across slow I/O is not.
        if level == Level::Error {
            eprintln!("{message}");
        } else {
            println!("{message}");
        }

        // 3. Callback, if registered.
        if let Some(cb) = cb {
            self.invoke_callback(cb, message);
        }
    }

    fn invoke_callback(&self, cb: LogCallback, message: &str) {
        if IN_CALLBACK.with(|f| f.get()) {
            eprintln!("[Logger] Re-entrant callback dropped: {message}");
            return;
        }
        // A C string cannot carry an interior NUL, so a message holding one is
        // delivered truncated at the first NUL rather than dropped.
        let Ok(cstr) = CString::new(message) else {
            let truncated: String = message.chars().take_while(|c| *c != '\0').collect();
            let Ok(cstr) = CString::new(truncated) else {
                return;
            };
            self.call_guarded(cb, &cstr, message);
            return;
        };
        self.call_guarded(cb, &cstr, message);
    }

    fn call_guarded(&self, cb: LogCallback, cstr: &CString, message: &str) {
        IN_CALLBACK.with(|f| f.set(true));
        // A host callback that unwinds must not tear down the log call site.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: `cb` came from `set_callback` and `cstr` is a valid
            // nul-terminated string that outlives the call.
            unsafe { cb(cstr.as_ptr()) }
        }));
        IN_CALLBACK.with(|f| f.set(false));
        if result.is_err() {
            eprintln!("[Logger] Callback threw for: {message}");
        }
    }

    /// Truncate `salma.log` and reopen it.
    ///
    /// Returns `true` only when the truncation and the reopen both succeeded.
    /// A `false` return does not mean the log file is closed: on the
    /// truncation-failure path the file is reopened in append mode and logging
    /// continues, with `bytes_written` reseeded from the reopened file's size so
    /// the rotation counter still matches what is on disk.
    ///
    /// Blocks on the state mutex and does file I/O.
    pub fn clear_log(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let path = state.directory.join("salma.log");
        state.file = None; // flush + close

        if File::create(&path).is_err() {
            // Reopen in append mode even on failure, so logging survives.
            let (file, bytes) = open_append(&path);
            state.file = file;
            state.bytes_written = bytes;
            return false;
        }

        let (file, _) = open_append(&path);
        let opened = file.is_some();
        state.file = file;
        state.bytes_written = 0;
        opened
    }

    /// Absolute path of the active log file.
    pub fn log_path(&self) -> PathBuf {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.directory.join("salma.log")
    }

    /// Absolute path of the log directory.
    pub fn log_directory(&self) -> PathBuf {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.directory.clone()
    }
}

impl FileState {
    /// Write one formatted line and rotate if the file has grown past the cap.
    /// The caller must hold the state mutex. Does nothing when there is no file
    /// handle.
    ///
    /// **One-write invariant, do not break it.** `salma.log` has two independent
    /// appending writers: this module, and the `Logger` compiled into
    /// mo2-server, which resolves the same file because the DLL is deployed next
    /// to the exe. Both open the file with O_APPEND (see `open_append`), and
    /// O_APPEND makes one write atomic but cannot glue two writes together. Each
    /// record must therefore reach the unbuffered `std::fs::File` in a single
    /// `write_all` of the whole line, trailing newline included, which
    /// `format_line` already appends. Do not wrap the handle in a `BufWriter`,
    /// and do not split the newline into a second write: that produces torn
    /// records, 129 damaged out of 11,547 lines in a sample log.
    /// `Logger::write_log_unlocked` in `src/Logger.cpp` holds the same
    /// invariant on the server side.
    fn write_line(&mut self, level: Level, message: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let line = format_line(now_local(), level.as_str(), message);
        if file.write_all(line.as_bytes()).is_err() {
            return;
        }
        // The newline is part of `line` and the whole record goes out in one
        // call, so the counter takes the line length with no `+ 1` adjustment.
        // Adding one would drift the rotation point away from the real size.
        self.bytes_written += line.len() as u64;
        self.rotate_if_needed();
    }

    /// Rotate the log once it passes [`MAX_LOG_SIZE`]. The caller must hold the
    /// state mutex.
    ///
    /// The check runs after the write, so the file always crosses the cap before
    /// it rotates. `bytes_written` is seeded from the existing file size when the
    /// handle is opened, so the count survives a process restart.
    ///
    /// ```text
    ///   bytes_written >= MAX_LOG_SIZE (10 MiB)
    ///             |
    ///             v
    ///    close salma.log
    ///             |
    ///             v
    ///    salma.log.3 -> deleted          i == MAX_ROTATED_FILES
    ///    salma.log.2 -> salma.log.3      loop runs i = 3, 2, 1;
    ///    salma.log.1 -> salma.log.2      a missing file is skipped
    ///             |
    ///             v
    ///    rename salma.log -> salma.log.1
    ///        |                      |
    ///     ok |                      | fails (file pinned by an antivirus
    ///        |                      |        scanner or a log viewer)
    ///        v                      v
    ///   reopen append          reopen append
    ///   bytes_written = 0      bytes_written unchanged
    ///                            -> the file grows past the cap, no entries
    ///                               are lost, and the next rotation catches up
    /// ```
    ///
    /// Leaving the counter alone on the failure branch is deliberate, not an
    /// oversight: it is what keeps entries from being dropped while the rename
    /// cannot happen. Resetting it would leave the counter claiming an empty
    /// file when the real one is already past the cap. Do not "fix" it.
    ///
    /// A failed delete or a failed rename of a rotated file only prints a
    /// diagnostic to stderr; rotation continues.
    fn rotate_if_needed(&mut self) {
        if self.bytes_written < MAX_LOG_SIZE {
            return;
        }
        self.file = None; // flush + close

        let dir = &self.directory;

        // Shift the rotated files down: .3 is deleted, .2 -> .3, .1 -> .2.
        for i in (1..=MAX_ROTATED_FILES).rev() {
            let src = dir.join(format!("salma.log.{i}"));
            if !src.exists() {
                continue;
            }
            if i == MAX_ROTATED_FILES {
                if let Err(err) = fs::remove_file(&src) {
                    eprintln!(
                        "[Logger] Failed to remove rotated log {}: {err}",
                        src.display()
                    );
                }
            } else {
                let dst = dir.join(format!("salma.log.{}", i + 1));
                if let Err(err) = fs::rename(&src, &dst) {
                    eprintln!(
                        "[Logger] Failed to rename {} -> {}: {err}",
                        src.display(),
                        dst.display()
                    );
                }
            }
        }

        // Rotate the current log to .1. A rename can still fail on Windows if
        // an antivirus scanner or a log viewer pins the file. When it does, the
        // counter is deliberately left alone: the existing file is reopened in
        // append mode and grows past the cap, which loses no entries and lets
        // the next rotation catch up.
        let current = dir.join("salma.log");
        if let Err(err) = fs::rename(&current, dir.join("salma.log.1")) {
            eprintln!("[Logger] Failed to rotate salma.log -> salma.log.1: {err}");
            let (file, _) = open_append(&current);
            self.file = file;
            return;
        }

        let (file, _) = open_append(&current);
        self.file = file;
        self.bytes_written = 0;
    }
}

/// Open `path` in append mode, reporting the existing size so the rotation
/// counter continues from where a previous process left off.
///
/// Never fails. On an open error it prints a diagnostic to stderr and returns
/// `(None, 0)`, which puts the logger into the degraded, file-less mode
/// described on [`Logger::instance`]. The handle is deliberately unbuffered:
/// see the one-write invariant on `FileState::write_line`.
fn open_append(path: &Path) -> (Option<File>, u64) {
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => {
            let size = file.metadata().map(|m| m.len()).unwrap_or(0);
            (Some(file), size)
        }
        Err(err) => {
            eprintln!(
                "[Logger] Failed to open log file: {} ({err})",
                path.display()
            );
            (None, 0)
        }
    }
}

/// Broken-down local time: calendar fields plus milliseconds, the exact set the
/// log line format needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    millis: u32,
}

/// Current local time.
///
/// On Windows this is `GetLocalTime`, which applies the machine's timezone and
/// DST rules, so a salma line is comparable to any other line in the log. `std`
/// has no local-time conversion, so there is no portable alternative.
#[cfg(windows)]
fn now_local() -> Stamp {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st = unsafe { std::mem::zeroed() };
    // SAFETY: GetLocalTime only writes the SYSTEMTIME out-parameter.
    unsafe { GetLocalTime(&mut st) };
    Stamp {
        year: st.wYear as i32,
        month: st.wMonth as u32,
        day: st.wDay as u32,
        hour: st.wHour as u32,
        minute: st.wMinute as u32,
        second: st.wSecond as u32,
        millis: st.wMilliseconds as u32,
    }
}

/// Non-Windows fallback: UTC, since there is no local-time source in `std`.
/// salma is Windows-only; this exists so the crate still builds elsewhere.
#[cfg(not(windows))]
fn now_local() -> Stamp {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    Stamp {
        year,
        month,
        day,
        hour: (rem / 3600) as u32,
        minute: ((rem % 3600) / 60) as u32,
        second: (rem % 60) as u32,
        millis: dur.subsec_millis(),
    }
}

/// Days-since-epoch to civil date (Howard Hinnant's algorithm).
#[cfg(not(windows))]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((y + i64::from(m <= 2)) as i32, m as u32, d as u32)
}

/// Format one log line, newline included:
/// `{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {level} {message}\n`.
///
/// The trailing newline is part of the format string because each record must
/// reach the file in a single write; see the one-write invariant on
/// `FileState::write_line`.
fn format_line(s: Stamp, level: &str, message: &str) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {} {}\n",
        s.year, s.month, s.day, s.hour, s.minute, s.second, s.millis, level, message
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> Stamp {
        Stamp {
            year: 2026,
            month: 7,
            day: 4,
            hour: 9,
            minute: 5,
            second: 3,
            millis: 7,
        }
    }

    /// The exact shape of a real `logs/salma.log` line, zero-padded in every
    /// field including the 3-digit milliseconds.
    #[test]
    fn line_format_matches_the_cpp_layout() {
        assert_eq!(
            format_line(stamp(), "INFO", "[install] Archive: mod.7z"),
            "2026-07-04 09:05:03.007 INFO [install] Archive: mod.7z\n"
        );
    }

    #[test]
    fn line_format_pads_every_field() {
        let s = Stamp {
            year: 999,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
            millis: 999,
        };
        assert_eq!(
            format_line(s, "ERROR", "x"),
            "0999-12-31 23:59:59.999 ERROR x\n"
        );
    }

    #[test]
    fn level_tags_match_the_cpp_strings() {
        assert_eq!(Level::Info.as_str(), "INFO");
        assert_eq!(Level::Warning.as_str(), "WARNING");
        assert_eq!(Level::Error.as_str(), "ERROR");
    }

    /// An empty message still produces a well-formed line, ending in a single
    /// space before the newline.
    #[test]
    fn empty_message_still_formats() {
        assert_eq!(
            format_line(stamp(), "INFO", ""),
            "2026-07-04 09:05:03.007 INFO \n"
        );
    }

    #[test]
    fn rotation_constants_match_the_cpp() {
        assert_eq!(MAX_LOG_SIZE, 10 * 1024 * 1024);
        assert_eq!(MAX_ROTATED_FILES, 3);
    }

    #[cfg(windows)]
    #[test]
    fn now_local_returns_a_plausible_stamp() {
        let s = now_local();
        assert!(s.year >= 2024, "year {} looks wrong", s.year);
        assert!((1..=12).contains(&s.month));
        assert!((1..=31).contains(&s.day));
        assert!(s.hour < 24 && s.minute < 60 && s.second < 61);
        assert!(s.millis < 1000);
    }

    /// The singleton anchors its directory on the module that owns this code,
    /// so the path ends in `logs` and is absolute.
    #[test]
    fn instance_resolves_a_logs_directory() {
        let dir = Logger::instance().log_directory();
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("logs"));
        assert_eq!(
            Logger::instance().log_path().file_name().unwrap(),
            "salma.log"
        );
    }

    // Registering and clearing the callback is covered once, by
    // `capi::tests::set_log_callback_reaches_the_logger`, which drives the same
    // code through the actual ABI export. Duplicating it here would race: the
    // callback is process-global and cargo runs tests in this binary in
    // parallel, so two tests toggling it would see each other's writes.
}
