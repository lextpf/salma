/*!
 * @brief routes process-wide logs to a file or host callback.
 * @author Alex (https://github.com/lextpf)
 *
 * a callback replaces file output. callback and console calls occur outside the state lock.
 * reentrant callback logging is dropped. file output rotates at 10 MiB and keeps three backups.
 */

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::utils::module_directory;

/**
 * @brief host log callback.
 * @author Alex (https://github.com/lextpf)
 *
 * the ABI spelling is the parameter type of [`crate::capi::setLogCallback`], `Option<unsafe extern
 * "C" fn(*const c_char)>`, which the MO2 plugin declares as `ctypes.CFUNCTYPE(None,
 * ctypes.c_char_p)`.
 */
pub type LogCallback = unsafe extern "C" fn(*const c_char);

// rotate once the current file reaches this size, in bytes (10 MiB).
const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

const MAX_ROTATED_FILES: u32 = 3;

// the file handle and its byte counter, which only ever move together.
struct FileState {
    // absolute path to the logs directory.
    directory: PathBuf,
    file: Option<File>,
    // approximate bytes written since the last rotation.
    bytes_written: u64,
}

/**
 * @struct Logger
 * @brief the logger singleton.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-lock-outline: thread safety
 *
 * the file state is protected by a mutex. callback and console calls run outside that lock.
 * callbacks can run on several threads at once. reentrant callback logging is dropped.
 *
 */
pub struct Logger {
    state: Mutex<FileState>,
    // callback function-pointer address, 0 when cleared.
    // stored lock-free so `set_callback` never blocks a logging thread.
    callback: AtomicUsize,
}

// module_anchor supplies an address from mo2-salma.dll for module path resolution.
// the log directory has to follow mo2-salma.dll, not the host executable: MO2 loads the DLL out of
// its plugins tree while the process is ModOrganizer.exe, and the log belongs next to the DLL
// either way.
fn module_anchor() {}

// re-entrancy guard for the host callback. a callback that itself logs would otherwise recurse
// until the stack overflows. the flag is per-thread because callbacks run on whichever thread
// logged.
thread_local! {
    static IN_CALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

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
    /**
     * @fn instance() -> &'static Logger
     * @brief the process-wide logger, constructed on first use.
     * @author Alex (https://github.com/lextpf)
     *
     */
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

    /**
     * @fn set_callback(&self, Option<LogCallback>)
     * @brief replace the callback with one atomic store; None clears it.
     * @author Alex (https://github.com/lextpf)
     *
     * a single lock-free atomic store, so it never contends with an in-flight log call and never
     * blocks.
     */
    pub fn set_callback(&self, callback: Option<LogCallback>) {
        let addr = match callback {
            Some(f) => f as usize,
            None => 0,
        };
        self.callback.store(addr, Ordering::SeqCst);
    }

    pub fn has_callback(&self) -> bool {
        self.callback.load(Ordering::SeqCst) != 0
    }

    fn callback(&self) -> Option<LogCallback> {
        let addr = self.callback.load(Ordering::SeqCst);
        if addr == 0 {
            None
        } else {
            // safety: the address was stored from a `LogCallback` in
            // `set_callback` and function pointers are not invalidated by the
            // round trip through `usize`.
            Some(unsafe { std::mem::transmute::<usize, LogCallback>(addr) })
        }
    }

    pub fn log(&self, message: &str) {
        self.emit(Level::Info, message);
    }

    pub fn log_warning(&self, message: &str) {
        self.emit(Level::Warning, message);
    }

    pub fn log_error(&self, message: &str) {
        self.emit(Level::Error, message);
    }

    fn emit(&self, level: Level, message: &str) {
        // 1. under the lock: snapshot the callback, and write the file line only when no callback
        // is registered.
        let cb = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let cb = self.callback();
            if cb.is_none() {
                state.write_line(level, message);
            }
            cb
        };

        // 2. console echo, outside the lock: interleaved console output is acceptable, holding the
        // mutex across slow I/O is not.
        if level == Level::Error {
            eprintln!("{message}");
        } else {
            println!("{message}");
        }

        // 3. callback, if registered.
        if let Some(cb) = cb {
            self.invoke_callback(cb, message);
        }
    }

    fn invoke_callback(&self, cb: LogCallback, message: &str) {
        if IN_CALLBACK.with(|f| f.get()) {
            eprintln!("[Logger] Re-entrant callback dropped: {message}");
            return;
        }
        // a C string cannot carry an interior NUL, so a message holding one is delivered truncated
        // at the first NUL rather than dropped.
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
        // a host callback that unwinds must not tear down the log call site.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // safety: `cb` came from `set_callback` and `cstr` is a valid
            // nul-terminated string that outlives the call.
            unsafe { cb(cstr.as_ptr()) }
        }));
        IN_CALLBACK.with(|f| f.set(false));
        if result.is_err() {
            eprintln!("[Logger] Callback threw for: {message}");
        }
    }

    /**
     * @fn clear_log(&self) -> bool
     * @brief truncate under the logger mutex and reopen in append mode.
     * @author Alex (https://github.com/lextpf)
     *
     * a `false` return does not mean the log file is closed: on the truncation-failure path the
     * file is reopened in append mode and logging continues, with `bytes_written` reseeded from the
     * reopened file's size so the rotation counter still matches what is on disk.
     * @return `true` only when the truncation and the reopen both succeeded.
     */
    pub fn clear_log(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let path = state.directory.join("salma.log");
        state.file = None; // flush + close

        if File::create(&path).is_err() {
            // reopen in append mode even on failure, so logging survives.
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

    pub fn log_path(&self) -> PathBuf {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.directory.join("salma.log")
    }

    pub fn log_directory(&self) -> PathBuf {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.directory.clone()
    }
}

impl FileState {
    // write one formatted line and rotate if the file has grown past the cap.
    // the caller must hold the state mutex.
    fn write_line(&mut self, level: Level, message: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let line = format_line(now_local(), level.as_str(), message);
        if file.write_all(line.as_bytes()).is_err() {
            return;
        }
        // the newline is part of `line` and the whole record goes out in one call, so the counter
        // takes the line length with no `+ 1` adjustment. adding one would drift the rotation point
        // away from the real size.
        self.bytes_written += line.len() as u64;
        self.rotate_if_needed();
    }

    // rotate the log once it passes MAX_LOG_SIZE.
    // the caller must hold the state mutex.
    fn rotate_if_needed(&mut self) {
        if self.bytes_written < MAX_LOG_SIZE {
            return;
        }
        self.file = None; // flush + close

        let dir = &self.directory;

        // shift the rotated files down: .3 is deleted, .2 -> .3, .1 -> .2.
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

        // rotate the current log to .1. a rename can still fail on windows if an antivirus scanner
        // or a log viewer pins the file.
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

// open path in append mode, reporting the existing size so the rotation counter continues from
// where a previous process left off.
// never fails.
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

// broken-down local time: calendar fields plus milliseconds, the exact set the log line format
// needs.
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

// current local time.
#[cfg(windows)]
fn now_local() -> Stamp {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // safety: SYSTEMTIME holds only integer fields, so all-zero is a valid value, and GetLocalTime
    // overwrites it on the next line.
    let mut st = unsafe { std::mem::zeroed() };
    // safety: GetLocalTime only writes the SYSTEMTIME out-parameter.
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

// format one log line, newline included: {:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {level}
// {message}\n.
// the trailing newline is part of the format string because each record must reach the file in a
// single write; see the one-write invariant on `FileState::write_line`.
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

    #[test]
    fn instance_resolves_a_logs_directory() {
        let dir = Logger::instance().log_directory();
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("logs"));
        assert_eq!(
            Logger::instance().log_path().file_name().unwrap(),
            "salma.log"
        );
    }

    // registering and clearing the callback is covered once, by
    // `capi::tests::set_log_callback_reaches_the_logger`, which drives the same code through the
    // actual ABI export. duplicating it here would race: the callback is process-global and cargo
    // runs tests in this binary in parallel, so two tests toggling it would see each other's
    // writes.
}
