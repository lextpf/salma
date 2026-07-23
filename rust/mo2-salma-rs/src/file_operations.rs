//! Queued file copy operations - Rust port of `src/FileOperations.hpp` /
//! `src/FileOperations.cpp`.
//!
//! Collects file and folder copy operations into a queue and executes them all
//! at once sorted by priority, which is how a FOMOD installer applies
//! file-level overrides: the higher-priority operation is copied LAST and wins
//! at a shared destination.
//!
//! ## `execute()` sorts by priority ONLY
//!
//! [`FileOperations::execute`] stable-sorts ascending by
//! [`FileOperation::priority`] and does NOT look at
//! [`FileOperation::document_order`]; it relies on the stable sort preserving
//! insertion order among equal priorities (`FileOperations.cpp:59-62`). This is
//! deliberately different from `FomodService::execute_file_operations`, which
//! is a SEPARATE executor that sorts by `(priority, document_order)`. Both
//! behaviors are real and both are reproduced; see PARITY-NOTES "Task 14".
//!
//! ## Errors never abort
//!
//! Every C++ entry point is documented "does not throw": each I/O step is
//! wrapped in a `try`/`catch (const fs::filesystem_error&)` that logs and
//! either returns early or continues. The port returns `()` from every function
//! and swallows [`std::io::Error`] at exactly the same points, so no `Result`
//! and no panic escapes toward FFI.
//!
//! ## Dropped logging
//!
//! There is no Rust logger yet (Task 17). Every `Logger::instance().log*` call
//! is dropped; the branch that produced it is kept with a `// dropped log site`
//! comment so Task 17 can restore it verbatim.
//!
//! ## Disk-full detection mapping
//!
//! The C++ compares the caught `filesystem_error`'s `error_code` against the
//! portable `std::errc::no_space_on_device` (`FileOperations.cpp:24-27`); the
//! MSVC Win32 error mapping folds both `ERROR_DISK_FULL` (112) and
//! `ERROR_HANDLE_DISK_FULL` (39) into that condition. Rust has no
//! `error_condition` equivalent, so [`is_disk_full`] tests
//! `io::ErrorKind::StorageFull` (std maps the same two Win32 codes to it) OR
//! the raw OS code directly, with the accepted code list `cfg`-gated per
//! platform (`{112, 39}` on Windows, `ENOSPC` = 28 elsewhere) so a Windows
//! `ERROR_OUT_OF_PAPER` (also 28) cannot be mistaken for a full disk. See
//! PARITY-NOTES "Task 14".

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::types::{FileOpType, FileOperation};

/// Sticky process-wide disk-full flag. Mirror of the C++ anonymous-namespace
/// `g_disk_full`: once set, callers can decide the whole install is
/// unrecoverable without inspecting individual error logs. Cleared only by
/// [`FileOperations::reset_disk_full`].
static DISK_FULL: AtomicBool = AtomicBool::new(false);

/// Raw OS error codes that mean "the volume ran out of space".
///
/// Windows: `ERROR_HANDLE_DISK_FULL` (39) and `ERROR_DISK_FULL` (112), the two
/// Win32 codes MSVC maps to `std::errc::no_space_on_device`.
#[cfg(windows)]
const DISK_FULL_OS_CODES: &[i32] = &[39, 112];

/// Raw OS error codes that mean "the volume ran out of space".
///
/// Non-Windows: `ENOSPC` (28). Kept `cfg`-gated because 28 is
/// `ERROR_OUT_OF_PAPER` on Windows, which must NOT set the sticky flag.
#[cfg(not(windows))]
const DISK_FULL_OS_CODES: &[i32] = &[28];

/// True when `err` means "the volume ran out of space". Mirror of the C++
/// file-scoped `is_disk_full(const std::error_code&)`.
///
/// Accepts either the std-normalized [`io::ErrorKind::StorageFull`] or a raw OS
/// code from [`DISK_FULL_OS_CODES`]; the raw check is the belt-and-braces half,
/// since a code std does not classify would otherwise arrive as
/// [`io::ErrorKind::Uncategorized`].
fn is_disk_full(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::StorageFull {
        return true;
    }
    match err.raw_os_error() {
        Some(code) => DISK_FULL_OS_CODES.contains(&code),
        None => false,
    }
}

/// Set the sticky flag when `err` is a disk-full error. Mirror of the C++
/// file-scoped `note_disk_full_if_applicable(const fs::filesystem_error&)`.
fn note_disk_full_if_applicable(err: &io::Error) {
    if is_disk_full(err) {
        DISK_FULL.store(true, Ordering::Relaxed);
    }
}

/// Queued file copy operations with priority sorting. Mirror of
/// `mo2core::FileOperations`.
///
/// Instances are NOT thread-safe (the C++ says the same); the associated copy
/// functions are, since they only touch local state plus the atomic disk-full
/// flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOperations {
    /// Queued operations awaiting [`FileOperations::execute`]. Mirror of the
    /// C++ `ops_`.
    ops: Vec<FileOperation>,
}

impl FileOperations {
    /// Construct an empty operation queue.
    pub fn new() -> Self {
        FileOperations::default()
    }

    /// Enqueue a file operation for deferred execution. Mirror of
    /// `FileOperations::add`.
    ///
    /// The operation is appended to the queue; no I/O happens until
    /// [`FileOperations::execute`] is called.
    pub fn add(&mut self, op: FileOperation) {
        self.ops.push(op);
    }

    /// Execute all queued operations in priority order. Mirror of
    /// `FileOperations::execute`.
    ///
    /// Stable-sorts ascending by [`FileOperation::priority`] ALONE, so
    /// higher-priority operations are copied last and win at a shared
    /// destination, and equal-priority operations keep their insertion order.
    /// [`FileOperation::document_order`] is deliberately unused here.
    ///
    /// Individual failures are skipped and never abort the batch, and the queue
    /// is ALWAYS cleared afterwards, even when operations failed.
    pub fn execute(&mut self) {
        // `Vec::sort_by_key` is a stable sort, matching `std::stable_sort`:
        // equal-priority operations stay in insertion order, which is the
        // document-order tiebreaker callers rely on.
        self.ops.sort_by_key(|op| op.priority);

        // dropped log site: log("[install] Executing {} file operations in
        // priority order...", ops_.size())

        for op in &self.ops {
            // The C++ wraps each operation in try/catch so one failure cannot
            // abort the batch; copy_file / copy_folder already swallow their
            // own I/O errors here, so the loop body is inherently non-aborting.
            // dropped log site: log_error("[install] Failed to execute file
            // operation: {} -> {}: {}") on the caught exception.
            match op.op_type {
                FileOpType::File => {
                    Self::copy_file(Path::new(&op.source), Path::new(&op.destination));
                }
                FileOpType::Folder => {
                    Self::copy_folder(Path::new(&op.source), Path::new(&op.destination));
                }
            }
        }

        self.ops.clear();
    }

    /// Discard all queued operations without executing. Mirror of
    /// `FileOperations::clear`.
    pub fn clear(&mut self) {
        self.ops.clear();
    }

    /// Number of queued operations. Mirror of `FileOperations::count`, which
    /// returns `int`.
    pub fn count(&self) -> i32 {
        self.ops.len() as i32
    }

    /// Copy a single file, creating parent directories as needed. Mirror of
    /// `FileOperations::copy_file`.
    ///
    /// - A missing source is a warning + silent skip, NOT an error.
    /// - A `create_directories` failure returns early WITHOUT copying (and
    ///   without touching the disk-full flag, matching the C++, which only
    ///   inspects the copy error).
    /// - The copy ALWAYS overwrites an existing destination.
    pub fn copy_file(src: &Path, dst: &Path) {
        // Missing source is logged and silently skipped (not an error).
        // dropped log site: log_warning("[install] Missing file: {}")
        if !src.exists() {
            return;
        }
        // `dst.parent()` is `Some("")` for a bare filename; `create_dir_all("")`
        // is a no-op Ok, matching C++ `create_directories("")` returning false
        // without an error. `None` (dst is a root) skips the call entirely.
        if let Some(parent) = dst.parent() {
            if fs::create_dir_all(parent).is_err() {
                // dropped log site: log_error("[install] Failed to create
                // directory {}: {}")
                return;
            }
        }
        // `fs::copy` overwrites, matching `copy_options::overwrite_existing`.
        if let Err(err) = fs::copy(src, dst) {
            note_disk_full_if_applicable(&err);
            // dropped log site: log_error("[install] Copy error: {}")
        }
    }

    /// Recursively copy a folder tree. Mirror of `FileOperations::copy_folder`.
    ///
    /// Recreates the full directory structure of `src` under `dst`, overwriting
    /// existing files. A missing source warns and skips; symlinked entries are
    /// SKIPPED (the C++ `is_symlink(entry.symlink_status())` test); traversal
    /// uses the equivalent of `directory_options::skip_permission_denied`.
    pub fn copy_folder(src: &Path, dst: &Path) {
        // dropped log site: log_warning("[install] Missing folder: {}")
        if !src.exists() {
            return;
        }
        if fs::create_dir_all(dst).is_err() {
            // dropped log site: log_error("[install] Failed to create directory
            // {}: {}")
            return;
        }

        // Explicit stack in place of `fs::recursive_directory_iterator`. A
        // directory entry is created at its destination BEFORE its children are
        // visited (as with the C++ pre-order traversal), so empty
        // subdirectories are reproduced. Sibling order within a directory is
        // unspecified in both implementations.
        let mut stack: Vec<PathBuf> = vec![src.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let read_dir = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(err) => {
                    // `skip_permission_denied`: an inaccessible directory is
                    // skipped rather than ending the traversal.
                    if err.kind() == io::ErrorKind::PermissionDenied {
                        continue;
                    }
                    // Any other iteration error escapes the C++ iterator and is
                    // caught by the outer handler, which ABORTS the whole copy.
                    note_disk_full_if_applicable(&err);
                    // dropped log site: log_error("[install] Failed to iterate
                    // directory {}: {}")
                    return;
                }
            };
            for entry in read_dir {
                let entry = match entry {
                    Ok(e) => e,
                    Err(err) => {
                        if err.kind() == io::ErrorKind::PermissionDenied {
                            continue;
                        }
                        note_disk_full_if_applicable(&err);
                        // dropped log site: log_error("[install] Failed to
                        // iterate directory {}: {}")
                        return;
                    }
                };
                // `DirEntry::file_type` does not follow symlinks, matching
                // `entry.symlink_status()`.
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_symlink() {
                    continue;
                }
                let path = entry.path();
                let Ok(relative) = path.strip_prefix(src) else {
                    continue;
                };
                let dest_path = dst.join(relative);
                if file_type.is_dir() {
                    if let Err(err) = fs::create_dir_all(&dest_path) {
                        note_disk_full_if_applicable(&err);
                        // dropped log site: log_error("[install] Copy error:
                        // {}")
                        continue;
                    }
                    stack.push(path);
                } else {
                    if let Some(parent) = dest_path.parent() {
                        if let Err(err) = fs::create_dir_all(parent) {
                            note_disk_full_if_applicable(&err);
                            // dropped log site: log_error("[install] Copy
                            // error: {}")
                            continue;
                        }
                    }
                    if let Err(err) = fs::copy(&path, &dest_path) {
                        note_disk_full_if_applicable(&err);
                        // dropped log site: log_error("[install] Copy error:
                        // {}")
                    }
                }
            }
        }
    }

    /// Copy the immediate contents of a directory into `dst`. Mirror of
    /// `FileOperations::copy_directory_contents`.
    ///
    /// Unlike [`FileOperations::copy_folder`] this copies INTO `dst` rather
    /// than recreating the source root: files are copied directly,
    /// subdirectories recursively via `copy_folder`.
    ///
    /// Quirks reproduced from the C++: there is NO source-existence check (a
    /// missing `src` still creates `dst`, then fails at iteration), and the
    /// `create_directories(dst)` failure path does NOT consult the disk-full
    /// flag even though the sibling `move_directory_contents` does.
    pub fn copy_directory_contents(src: &Path, dst: &Path) {
        if fs::create_dir_all(dst).is_err() {
            // dropped log site: log_error("[install] Failed to create directory
            // {}: {}")
            return;
        }
        let read_dir = match fs::read_dir(src) {
            Ok(rd) => rd,
            Err(err) => {
                note_disk_full_if_applicable(&err);
                // dropped log site: log_error("[install] Failed to iterate
                // directory {}: {}")
                return;
            }
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    note_disk_full_if_applicable(&err);
                    // dropped log site: log_error("[install] Failed to iterate
                    // directory {}: {}")
                    return;
                }
            };
            let path = entry.path();
            let Some(name) = path.file_name() else {
                continue;
            };
            let target = dst.join(name);
            // `fs::is_directory(entry)` FOLLOWS symlinks (it converts the entry
            // to its path), so a symlink to a directory takes the folder
            // branch; `Path::is_dir` follows too.
            if path.is_dir() {
                Self::copy_folder(&path, &target);
            } else {
                Self::copy_file(&path, &target);
            }
        }
    }

    /// Move the immediate contents of a directory. Mirror of
    /// `FileOperations::move_directory_contents`.
    ///
    /// Same shape as [`FileOperations::copy_directory_contents`], but tries
    /// [`fs::rename`] per child first. A same-volume rename is a metadata
    /// operation, so the final `unfomod -> mod_path` step costs effectively no
    /// disk. The fallback is copy + remove on ANY rename error (cross-device is
    /// the motivating case, but a locked or non-empty target benefits too),
    /// EXCEPT a disk-full rename error, which sets the sticky flag and skips the
    /// child instead of attempting a copy that cannot succeed.
    pub fn move_directory_contents(src: &Path, dst: &Path) {
        // dropped log site: log_warning("[install] Missing folder for move: {}")
        if !src.exists() {
            return;
        }
        if let Err(err) = fs::create_dir_all(dst) {
            // Unlike copy_directory_contents, the C++ DOES check the disk-full
            // condition on this create failure.
            note_disk_full_if_applicable(&err);
            // dropped log site: log_error("[install] Failed to create directory
            // {}: {}")
            return;
        }
        let read_dir = match fs::read_dir(src) {
            Ok(rd) => rd,
            Err(err) => {
                note_disk_full_if_applicable(&err);
                // dropped log site: log_error("[install] Failed to iterate
                // directory for move {}: {}")
                return;
            }
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    note_disk_full_if_applicable(&err);
                    // dropped log site: log_error("[install] Failed to iterate
                    // directory for move {}: {}")
                    return;
                }
            };
            let path = entry.path();
            let Some(name) = path.file_name() else {
                continue;
            };
            let target = dst.join(name);
            let rename_err = match fs::rename(&path, &target) {
                Ok(()) => continue,
                Err(err) => err,
            };
            if is_disk_full(&rename_err) {
                DISK_FULL.store(true, Ordering::Relaxed);
                // dropped log site: log_error("[install] Move error (disk full)
                // {} -> {}: {}")
                continue;
            }
            // dropped log site: log_warning("[install] rename {} -> {} failed
            // ({}); falling back to copy") - kept so Task 17 can restore the
            // same-volume-failure vs cross-volume-move distinction.
            if path.is_dir() {
                Self::copy_folder(&path, &target);
            } else {
                Self::copy_file(&path, &target);
            }
            // Best-effort source cleanup, mirroring `fs::remove_all` with an
            // ignored error_code: if it fails the temp dir is cleaned up when
            // the install scope ends. `remove_all` uses symlink_status, so a
            // symlink is unlinked as a link, never recursed into.
            //
            // A real directory recurses (`remove_dir_all`). Everything else - a
            // real file, a file symlink, or a directory symlink/junction (all of
            // which report `is_dir() == false` under `symlink_metadata`) - is
            // removed as a single entry. `remove_file` handles files and, on
            // every platform, file symlinks plus Unix directory symlinks; a
            // Windows directory reparse point is directory-attributed and
            // `remove_file` (DeleteFileW) cannot delete it, so fall back to
            // `remove_dir` (RemoveDirectoryW), which unlinks the reparse point
            // without following it - matching `fs::remove_all`'s symlink_status
            // behavior. On Unix the `remove_file` unlink already succeeds, so the
            // fallback never runs there.
            let remove_result = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_dir() => fs::remove_dir_all(&path),
                Ok(_) => fs::remove_file(&path).or_else(|_| fs::remove_dir(&path)),
                Err(err) => Err(err),
            };
            if remove_result.is_err() {
                // dropped log site: log_warning("[install] Move fallback could
                // not remove source {}: {}")
            }
        }
    }

    /// True iff any copy or move call has hit "no space on device". Mirror of
    /// `FileOperations::disk_full_encountered`.
    ///
    /// Sticky and process-global. Callers check it after a batch of operations
    /// so disk-full surfaces as a hard install failure instead of letting the
    /// missing files masquerade as a successful partial install.
    pub fn disk_full_encountered() -> bool {
        DISK_FULL.load(Ordering::Relaxed)
    }

    /// Reset the sticky disk-full flag. Mirror of
    /// `FileOperations::reset_disk_full`.
    ///
    /// Called at the start of an install so a prior install's disk-full does not
    /// poison the next one.
    pub fn reset_disk_full() {
        DISK_FULL.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// Create a fresh, uniquely named temp directory for one test case.
    fn temp_root(tag: &str) -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("salma_t14_{tag}_{}_{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Write `contents` to `path`, creating parent directories.
    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(path, contents).expect("write file");
    }

    /// Read a file back as a `String`; panics if it is missing.
    fn read_file(path: &Path) -> String {
        fs::read_to_string(path).expect("read file")
    }

    /// Build a File operation from `&str` paths.
    fn file_op(src: &Path, dst: &Path, priority: i32, document_order: i32) -> FileOperation {
        FileOperation {
            op_type: FileOpType::File,
            source: src.to_string_lossy().into_owned(),
            destination: dst.to_string_lossy().into_owned(),
            priority,
            document_order,
        }
    }

    // --- queue: sorting, clearing, counting --------------------------------

    #[test]
    fn execute_sorts_ascending_by_priority_so_highest_wins() {
        let root = temp_root("prio");
        let low = root.join("low.txt");
        let high = root.join("high.txt");
        write_file(&low, "LOW");
        write_file(&high, "HIGH");
        let dest = root.join("out/shared.txt");

        let mut ops = FileOperations::new();
        // High priority added FIRST; the ascending sort must still copy it last.
        ops.add(file_op(&high, &dest, 5, 0));
        ops.add(file_op(&low, &dest, 1, 1));
        ops.execute();

        assert_eq!(read_file(&dest), "HIGH");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn equal_priority_keeps_insertion_order_and_ignores_document_order() {
        let root = temp_root("stable");
        let first = root.join("first.txt");
        let second = root.join("second.txt");
        let third = root.join("third.txt");
        write_file(&first, "1");
        write_file(&second, "2");
        write_file(&third, "3");
        let dest = root.join("out/shared.txt");

        let mut ops = FileOperations::new();
        // Descending document_order on purpose: `execute` must NOT consult it,
        // so the LAST-INSERTED op wins even though its document_order is lowest.
        ops.add(file_op(&first, &dest, 0, 99));
        ops.add(file_op(&second, &dest, 0, 50));
        ops.add(file_op(&third, &dest, 0, 0));
        ops.execute();

        assert_eq!(read_file(&dest), "3");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mixed_priorities_keep_stable_order_within_a_priority() {
        let root = temp_root("mixed");
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        let c = root.join("c.txt");
        write_file(&a, "A");
        write_file(&b, "B");
        write_file(&c, "C");
        let dest = root.join("out/shared.txt");

        let mut ops = FileOperations::new();
        ops.add(file_op(&a, &dest, 1, 0));
        ops.add(file_op(&b, &dest, 0, 1));
        ops.add(file_op(&c, &dest, 1, 2));
        ops.execute();

        // Sorted order is B(0), A(1), C(1): A before C by insertion order.
        assert_eq!(read_file(&dest), "C");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn execute_clears_the_queue() {
        let root = temp_root("clearexec");
        let src = root.join("src.txt");
        write_file(&src, "X");
        let dest = root.join("dst.txt");

        let mut ops = FileOperations::new();
        ops.add(file_op(&src, &dest, 0, 0));
        assert_eq!(ops.count(), 1);
        ops.execute();
        assert_eq!(ops.count(), 0);

        // A second execute is a no-op: remove the destination and confirm it
        // does not come back.
        fs::remove_file(&dest).expect("remove dest");
        ops.execute();
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_discards_without_copying() {
        let root = temp_root("clear");
        let src = root.join("src.txt");
        write_file(&src, "X");
        let dest = root.join("dst.txt");

        let mut ops = FileOperations::new();
        ops.add(file_op(&src, &dest, 0, 0));
        ops.add(file_op(&src, &dest, 1, 1));
        assert_eq!(ops.count(), 2);
        ops.clear();
        assert_eq!(ops.count(), 0);
        ops.execute();

        assert!(!dest.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn execute_skips_failing_operation_and_continues() {
        let root = temp_root("skipfail");
        let missing = root.join("nope.txt");
        let good = root.join("good.txt");
        write_file(&good, "G");
        let dest_bad = root.join("out/bad.txt");
        let dest_good = root.join("out/good.txt");

        let mut ops = FileOperations::new();
        ops.add(file_op(&missing, &dest_bad, 0, 0));
        ops.add(file_op(&good, &dest_good, 0, 1));
        ops.execute();

        assert!(!dest_bad.exists(), "missing source must not create a dest");
        assert_eq!(read_file(&dest_good), "G");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn execute_dispatches_folder_ops_to_copy_folder() {
        let root = temp_root("execfolder");
        let src = root.join("src");
        write_file(&src.join("sub/leaf.txt"), "L");
        let dest = root.join("dst");

        let mut ops = FileOperations::new();
        ops.add(FileOperation {
            op_type: FileOpType::Folder,
            source: src.to_string_lossy().into_owned(),
            destination: dest.to_string_lossy().into_owned(),
            priority: 0,
            document_order: 0,
        });
        ops.execute();

        assert_eq!(read_file(&dest.join("sub/leaf.txt")), "L");
        let _ = fs::remove_dir_all(&root);
    }

    // --- copy_file ----------------------------------------------------------

    #[test]
    fn copy_file_missing_source_is_a_silent_skip() {
        let root = temp_root("cfmissing");
        let src = root.join("absent.txt");
        let dest = root.join("out/absent.txt");

        FileOperations::copy_file(&src, &dest);

        assert!(!dest.exists());
        assert!(
            !root.join("out").exists(),
            "the parent must not be created when the source is missing"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_file_creates_parents_and_overwrites() {
        let root = temp_root("cfover");
        let src_a = root.join("a.txt");
        let src_b = root.join("b.txt");
        write_file(&src_a, "AAAA");
        write_file(&src_b, "B");
        let dest = root.join("deep/nested/out.txt");

        FileOperations::copy_file(&src_a, &dest);
        assert_eq!(read_file(&dest), "AAAA");

        // Shorter content proves the destination is replaced, not appended to.
        FileOperations::copy_file(&src_b, &dest);
        assert_eq!(read_file(&dest), "B");
        let _ = fs::remove_dir_all(&root);
    }

    // --- copy_folder --------------------------------------------------------

    #[test]
    fn copy_folder_missing_source_is_a_silent_skip() {
        let root = temp_root("cfoldmissing");
        let src = root.join("absent");
        let dest = root.join("dst");

        FileOperations::copy_folder(&src, &dest);

        assert!(!dest.exists(), "dst is not created when src is missing");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_folder_reproduces_nested_tree_and_empty_dirs() {
        let root = temp_root("cfoldnested");
        let src = root.join("src");
        write_file(&src.join("top.txt"), "T");
        write_file(&src.join("a/one.txt"), "1");
        write_file(&src.join("a/b/two.txt"), "2");
        fs::create_dir_all(src.join("empty/deeper")).expect("mkdir empty");
        let dest = root.join("dst");

        FileOperations::copy_folder(&src, &dest);

        assert_eq!(read_file(&dest.join("top.txt")), "T");
        assert_eq!(read_file(&dest.join("a/one.txt")), "1");
        assert_eq!(read_file(&dest.join("a/b/two.txt")), "2");
        assert!(
            dest.join("empty/deeper").is_dir(),
            "empty dirs are recreated"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_folder_overwrites_existing_files_and_keeps_unrelated_ones() {
        let root = temp_root("cfoldover");
        let src = root.join("src");
        write_file(&src.join("shared.txt"), "NEW");
        let dest = root.join("dst");
        write_file(&dest.join("shared.txt"), "OLDOLDOLD");
        write_file(&dest.join("keep.txt"), "K");

        FileOperations::copy_folder(&src, &dest);

        assert_eq!(read_file(&dest.join("shared.txt")), "NEW");
        assert_eq!(
            read_file(&dest.join("keep.txt")),
            "K",
            "copy_folder merges, it does not wipe the destination"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// Create a file symlink, returning false when the platform refuses (an
    /// unprivileged Windows session without Developer Mode).
    #[cfg(windows)]
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    /// Create a file symlink, returning false when the platform refuses.
    #[cfg(not(windows))]
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    /// Create a directory reparse point, returning false when the platform
    /// refuses. Prefers a real directory symlink; falls back to a junction
    /// (`mklink /J`) when symlink creation is denied (it needs
    /// `SeCreateSymbolicLinkPrivilege`, absent in an ordinary session). A
    /// junction is the same directory-attributed reparse point - `remove_file`
    /// cannot delete it, `remove_dir` can - so it drives the identical cleanup
    /// path and needs no privilege. `mklink` is a `cmd` builtin.
    #[cfg(windows)]
    fn try_symlink_dir(target: &Path, link: &Path) -> bool {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Create a directory symlink, returning false when the platform refuses.
    #[cfg(not(windows))]
    fn try_symlink_dir(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[test]
    fn copy_folder_skips_symlinks() {
        let root = temp_root("cfoldlink");
        let src = root.join("src");
        let real = src.join("real.txt");
        write_file(&real, "R");
        let link = src.join("link.txt");
        if !try_symlink_file(&real, &link) {
            // Symlink creation needs SeCreateSymbolicLinkPrivilege on Windows;
            // without it this case cannot be exercised, so it is skipped rather
            // than failed.
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let dest = root.join("dst");

        FileOperations::copy_folder(&src, &dest);

        assert_eq!(read_file(&dest.join("real.txt")), "R");
        assert!(
            !dest.join("link.txt").exists(),
            "symlinked entries are skipped, not dereferenced"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // --- copy_directory_contents -------------------------------------------

    #[test]
    fn copy_directory_contents_flattens_into_dst_and_leaves_src() {
        let root = temp_root("cdc");
        let src = root.join("src");
        write_file(&src.join("top.txt"), "T");
        write_file(&src.join("sub/leaf.txt"), "L");
        let dest = root.join("dst");

        FileOperations::copy_directory_contents(&src, &dest);

        // The source ROOT name is not recreated under dst.
        assert!(!dest.join("src").exists());
        assert_eq!(read_file(&dest.join("top.txt")), "T");
        assert_eq!(read_file(&dest.join("sub/leaf.txt")), "L");
        assert!(
            src.join("top.txt").exists(),
            "copy leaves the source intact"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn copy_directory_contents_creates_dst_even_when_src_is_missing() {
        // C++ quirk: there is no source-existence check, and create_directories
        // runs before the iteration that fails.
        let root = temp_root("cdcmissing");
        let src = root.join("absent");
        let dest = root.join("dst");

        FileOperations::copy_directory_contents(&src, &dest);

        assert!(dest.is_dir(), "dst is created before the source is touched");
        assert_eq!(fs::read_dir(&dest).expect("read dst").count(), 0);
        let _ = fs::remove_dir_all(&root);
    }

    // --- move_directory_contents -------------------------------------------

    #[test]
    fn move_directory_contents_moves_children_and_empties_src() {
        let root = temp_root("mdc");
        let src = root.join("src");
        write_file(&src.join("top.txt"), "T");
        write_file(&src.join("sub/leaf.txt"), "L");
        let dest = root.join("dst");

        FileOperations::move_directory_contents(&src, &dest);

        assert_eq!(read_file(&dest.join("top.txt")), "T");
        assert_eq!(read_file(&dest.join("sub/leaf.txt")), "L");
        assert!(src.is_dir(), "the source root itself is not removed");
        assert_eq!(
            fs::read_dir(&src).expect("read src").count(),
            0,
            "every child is moved out of the source"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_directory_contents_falls_back_to_copy_when_rename_fails() {
        // Renaming onto an existing NON-EMPTY directory fails on every
        // platform, which drives the copy + remove fallback without needing a
        // second volume.
        let root = temp_root("mdcfallback");
        let src = root.join("src");
        write_file(&src.join("sub/new.txt"), "N");
        let dest = root.join("dst");
        write_file(&dest.join("sub/keep.txt"), "K");

        FileOperations::move_directory_contents(&src, &dest);

        assert_eq!(read_file(&dest.join("sub/new.txt")), "N");
        assert_eq!(
            read_file(&dest.join("sub/keep.txt")),
            "K",
            "the copy fallback merges into the existing directory"
        );
        assert!(
            !src.join("sub").exists(),
            "the fallback removes the moved source"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_directory_contents_removes_a_directory_symlink_child_on_fallback() {
        // A directory symlink reports is_dir() == false under symlink_metadata,
        // so the cleanup must NOT route it to remove_file alone: on Windows a
        // directory reparse point is directory-attributed and remove_file
        // (DeleteFileW) cannot delete it. The remove_file -> remove_dir fallback
        // mirrors fs::remove_all, which unlinks the link either way.
        let root = temp_root("mdcdirlink");
        let real_target = root.join("target");
        write_file(&real_target.join("inner.txt"), "T");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        let link = src.join("child");
        if !try_symlink_dir(&real_target, &link) {
            // Directory-symlink creation needs SeCreateSymbolicLinkPrivilege on
            // Windows; skip rather than fail when unavailable.
            let _ = fs::remove_dir_all(&root);
            return;
        }
        // Force the copy+remove fallback: a non-empty dst/child makes the
        // per-child fs::rename fail on every platform.
        let dest = root.join("dst");
        write_file(&dest.join("child/keep.txt"), "K");

        FileOperations::move_directory_contents(&src, &dest);

        assert!(
            !link.exists() && fs::symlink_metadata(&link).is_err(),
            "the source directory symlink must be unlinked, not left behind"
        );
        assert_eq!(
            fs::read_dir(&src).expect("read src").count(),
            0,
            "every child is moved out of the source"
        );
        // The symlink target itself is untouched (remove_all unlinks the link,
        // never recurses into it).
        assert_eq!(read_file(&real_target.join("inner.txt")), "T");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_directory_contents_missing_source_is_a_silent_skip() {
        let root = temp_root("mdcmissing");
        let src = root.join("absent");
        let dest = root.join("dst");

        FileOperations::move_directory_contents(&src, &dest);

        assert!(
            !dest.exists(),
            "unlike copy_directory_contents, move checks the source first"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // --- disk-full flag -----------------------------------------------------

    #[test]
    fn is_disk_full_maps_the_platform_error_codes() {
        for code in DISK_FULL_OS_CODES {
            assert!(
                is_disk_full(&io::Error::from_raw_os_error(*code)),
                "raw OS code {code} must read as disk-full"
            );
        }
        assert!(is_disk_full(&io::Error::new(
            io::ErrorKind::StorageFull,
            "full"
        )));
        assert!(!is_disk_full(&io::Error::new(
            io::ErrorKind::NotFound,
            "missing"
        )));
        assert!(!is_disk_full(&io::Error::other("no os code")));
    }

    #[test]
    fn reset_disk_full_clears_the_sticky_flag() {
        // Note: a real ENOSPC cannot be provoked from a unit test, so the
        // set-from-I/O path is exercised only by is_disk_full's mapping above.
        // The flag is process-wide, so this test only asserts the cleared state
        // it establishes itself.
        FileOperations::reset_disk_full();
        assert!(!FileOperations::disk_full_encountered());
    }
}
