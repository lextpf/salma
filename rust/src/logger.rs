//! Process-wide logger: `logs/salma.log` next to the DLL, plus a host callback.
//!
//! Rust port of `src/Logger.hpp`/`.cpp`. The C++ is a Meyer singleton; this is a
//! [`OnceLock`]-backed equivalent reached through [`Logger::instance`].
//!
//! ## Output routing (identical to the C++)
//!
//! For each of the three level methods:
//!
//! 1. Under the mutex, snapshot the callback. If NO callback is registered,
//!    write the formatted line to the file.
//! 2. Outside the mutex, echo the RAW message to the console: stdout for
//!    info/warning, stderr for error. This happens whether or not a callback is
//!    registered, and carries no timestamp or level prefix.
//! 3. If a callback IS registered, forward the raw message to it and write
//!    NOTHING to the file. A callback that logs re-entrantly is dropped with a
//!    diagnostic rather than recursing until the stack dies.
//!
//! So a registered callback REPLACES file logging; it does not duplicate it.
//! `setLogCallback(null)` restores file logging.
//!
//! ## Line format
//!
//! `YYYY-MM-DD HH:MM:SS.mmm LEVEL message`, LOCAL time, levels `INFO`,
//! `WARNING`, `ERROR`. The file is opened in append mode and rotates at 10 MiB,
//! keeping `salma.log.1` through `.3`.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::utils::module_directory;

/// Host log callback, mirror of the C++ `LogCallback` typedef
/// (`Logger.hpp:24`) and the `Mo2LogCallback` the C ABI accepts.
pub type LogCallback = unsafe extern "C" fn(*const c_char);

/// Rotate once the current file reaches this size (`Logger.hpp:247`).
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

/// Keep `salma.log.1` through `salma.log.3` (`Logger.hpp:248`).
const MAX_ROTATED_FILES: u32 = 3;

/// Mutex-guarded file state. The C++ guards `log_file_` and `bytes_written_`
/// with `mutex_`; grouping them in one struct makes that explicit.
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
    /// `set_callback` never blocks a logging thread, mirroring the C++
    /// `std::atomic<LogCallback>`.
    callback: AtomicUsize,
}

/// Address inside THIS module, used to resolve the DLL that owns this code.
///
/// The C++ anchors on `&Logger::instance` so the log directory follows the
/// mo2-salma DLL rather than the host executable: MO2 loads the DLL from its
/// plugins tree while the host process is ModOrganizer.exe, and the log must
/// land next to the DLL either way.
fn module_anchor() {}

// Re-entrancy guard for the host callback. A callback that itself logs would
// otherwise recurse until the stack overflows. Per-thread, because callbacks
// run on whichever thread logged (`Logger.cpp:18-29`).
thread_local! {
    static IN_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Severity tag written into the file. The console and callback paths receive
/// the raw message with no level prefix, exactly as in the C++.
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
    /// Mirror of the Meyer singleton at `Logger.cpp:33-37`. Construction
    /// resolves `<module dir>/logs`, creates it, and opens `salma.log` in
    /// append mode, seeding the rotation counter from the existing size.
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
    /// Mirror of `Logger::set_callback` (`Logger.cpp:85-88`): a lock-free
    /// atomic store, so it never contends with an in-flight log call. Clearing
    /// re-enables file logging.
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

    /// The shared body of the three level methods (`Logger.cpp:97-195`, which
    /// repeats this three times).
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

        // 2. Console echo, outside the lock: interleaving is acceptable and the
        //    C++ deliberately avoids holding the mutex across slow I/O.
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
        // A C string cannot carry an interior NUL; the C++ passes
        // `message.c_str()`, which truncates there too.
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
        // Mirror of the C++ `catch (...)` around the callback: a host callback
        // that unwinds must not tear down the log call site.
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

    /// Truncate `salma.log` and reopen it. Mirror of `Logger::clear_log`
    /// (`Logger.cpp:197-223`). Returns whether the file is open afterwards.
    pub fn clear_log(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let path = state.directory.join("salma.log");
        state.file = None; // flush + close

        if File::create(&path).is_err() {
            // Reopen in append mode even on failure, as the C++ does.
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
    /// Mirror of `Logger::write_log_unlocked` (`Logger.cpp:235-265`).
    fn write_line(&mut self, level: Level, message: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let line = format_line(now_local(), level.as_str(), message);
        if file.write_all(line.as_bytes()).is_err() {
            return;
        }
        // The C++ counts `line.size() + 1` for the newline it streams
        // separately; `line` already carries it here, so count it as-is.
        self.bytes_written += line.len() as u64;
        self.rotate_if_needed();
    }

    /// Mirror of `Logger::rotate_if_needed` (`Logger.cpp:267-333`).
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
        // an antivirus scanner or a log viewer pins the file. When it does the
        // C++ deliberately does NOT reset the counter: the existing file is
        // reopened in append mode and grows past the cap, which loses no
        // entries and lets the next rotation catch up.
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
/// counter continues from where a previous process left off
/// (`Logger.cpp:57-67`).
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

/// Broken-down local time, the fields `localtime_s` fills plus milliseconds.
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

/// Current LOCAL time.
///
/// On Windows this is `GetLocalTime`, which applies the same timezone and DST
/// rules as the C++ `localtime_s`, so timestamps stay comparable line for line.
/// `std` has no local-time conversion, so there is no portable alternative.
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

/// Format one log line, newline included.
///
/// Mirror of the `std::format` call at `Logger.cpp:252-261`:
/// `"{:04d}-{:02d}-{:02d} {:02d}:{:02d}:{:02d}.{:03d} {} {}"`.
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

    /// An empty message still produces a well-formed line ending in a single
    /// space before the newline, as the C++ format does.
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

    // Registering and clearing the callback is covered ONCE, by
    // `capi::tests::set_log_callback_reaches_the_logger`, which drives the same
    // code through the actual ABI export. Duplicating it here would race: the
    // callback is process-global and cargo runs tests in this binary in
    // parallel, so two tests toggling it would see each other's writes.
}
