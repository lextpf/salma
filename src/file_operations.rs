/*!
 * @brief executes file operations for mod installation.
 * @author Alex (https://github.com/lextpf)
 *
 * higher priorities execute later and replace lower-priority output. FileOperations::execute
 * keeps insertion order for equal priorities. fomod_service also uses document order.
 *
 * copy errors are logged and absorbed. a storage-full error sets a process-wide sticky flag that
 * the installation service converts to failure.
 */

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::logger::Logger;
use crate::types::{FileOpType, FileOperation};

// sticky process-wide disk-full flag.
// once set, a caller can decide the whole install is unrecoverable without inspecting individual
// error logs.
static DISK_FULL: AtomicBool = AtomicBool::new(false);

// raw OS error codes that mean "the volume ran out of space".
#[cfg(windows)]
const DISK_FULL_OS_CODES: &[i32] = &[39, 112];

// raw OS error codes that mean "the volume ran out of space".
// kept `cfg`-gated because 28 is `ERROR_OUT_OF_PAPER` on windows, which must not set the sticky
// flag.
#[cfg(not(windows))]
const DISK_FULL_OS_CODES: &[i32] = &[28];

// true when err means "the volume ran out of space".
fn is_disk_full(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::StorageFull {
        return true;
    }
    match err.raw_os_error() {
        Some(code) => DISK_FULL_OS_CODES.contains(&code),
        None => false,
    }
}

// set the sticky flag when err is a disk-full error.
fn note_disk_full_if_applicable(err: &io::Error) {
    if is_disk_full(err) {
        DISK_FULL.store(true, Ordering::Relaxed);
    }
}

/**
 * @struct FileOperations
 * @brief queued file copy operations with priority sorting.
 * @author Alex (https://github.com/lextpf)
 *
 * an instance is not thread-safe.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileOperations {
    ops: Vec<FileOperation>,
}

impl FileOperations {
    pub fn new() -> Self {
        FileOperations::default()
    }

    pub fn add(&mut self, op: FileOperation) {
        self.ops.push(op);
    }

    /**
     * @fn execute(&mut self)
     * @brief apply low priorities first and preserve enqueue order for ties.
     * @author Alex (https://github.com/lextpf)
     *
     * stable-sorts ascending by [`FileOperation::priority`] alone, so higher-priority operations
     * are copied last and win at a shared destination, and equal priorities keep their insertion
     * order.
     *
     * ### :material-alert-circle-outline: failure handling
     *
     * copy errors are logged and absorbed. storage-full errors set the process-wide sticky flag
     * for the installation service to check.
     */
    pub fn execute(&mut self) {
        // `Vec::sort_by_key` is stable: equal-priority operations stay in insertion order, which is
        // the tiebreaker callers rely on.
        self.ops.sort_by_key(|op| op.priority);

        Logger::instance().log(&format!(
            "[install] Executing {} file operations in priority order...",
            self.ops.len()
        ));

        for op in &self.ops {
            // copy_file and copy_folder absorb their own I/O errors, so the loop body cannot fail
            // and there is no per-operation failure to report here.
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

    pub fn clear(&mut self) {
        self.ops.clear();
    }

    pub fn count(&self) -> i32 {
        self.ops.len() as i32
    }

    /**
     * @fn copy_file(&Path, &Path)
     * @brief overwrite the destination and treat a missing source as a warning.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn copy_file(src: &Path, dst: &Path) {
        if !src.exists() {
            Logger::instance().log_warning(&format!("[install] Missing file: {}", src.display()));
            return;
        }
        // `dst.parent()` is `Some("")` for a bare filename, and `create_dir_all("")` is a no-op
        // `Ok`. `None` means `dst` is a root, which skips the call entirely.
        if let Some(parent) = dst.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                Logger::instance().log_error(&format!(
                    "[install] Failed to create directory {}: {err}",
                    dst.display()
                ));
                return;
            }
        }
        // `fs::copy` overwrites an existing destination.
        if let Err(err) = fs::copy(src, dst) {
            note_disk_full_if_applicable(&err);
            Logger::instance().log_error(&format!("[install] Copy error: {err}"));
        }
    }

    /**
     * @fn copy_folder(&Path, &Path)
     * @brief copy recursively without following symlinked entries.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn copy_folder(src: &Path, dst: &Path) {
        if !src.exists() {
            Logger::instance().log_warning(&format!("[install] Missing folder: {}", src.display()));
            return;
        }
        if let Err(err) = fs::create_dir_all(dst) {
            Logger::instance().log_error(&format!(
                "[install] Failed to create directory {}: {err}",
                dst.display()
            ));
            return;
        }

        // pre-order walk over an explicit stack: a directory is created at its destination before
        // its children are visited, so empty subdirectories are reproduced. sibling order within a
        // directory is unspecified.
        let mut stack: Vec<PathBuf> = vec![src.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let read_dir = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(err) => {
                    // an inaccessible directory is skipped rather than ending the traversal.
                    if err.kind() == io::ErrorKind::PermissionDenied {
                        continue;
                    }
                    // any other read error abandons the whole copy.
                    note_disk_full_if_applicable(&err);
                    Logger::instance().log_error(&format!(
                        "[install] Failed to iterate directory {}: {err}",
                        src.display()
                    ));
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
                        Logger::instance().log_error(&format!(
                            "[install] Failed to iterate directory {}: {err}",
                            src.display()
                        ));
                        return;
                    }
                };
                // `DirEntry::file_type` does not follow symlinks, so a linked entry is skipped
                // instead of being copied through.
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
                        Logger::instance().log_error(&format!("[install] Copy error: {err}"));
                        continue;
                    }
                    stack.push(path);
                } else {
                    if let Some(parent) = dest_path.parent() {
                        if let Err(err) = fs::create_dir_all(parent) {
                            note_disk_full_if_applicable(&err);
                            Logger::instance().log_error(&format!("[install] Copy error: {err}"));
                            continue;
                        }
                    }
                    if let Err(err) = fs::copy(&path, &dest_path) {
                        note_disk_full_if_applicable(&err);
                        Logger::instance().log_error(&format!("[install] Copy error: {err}"));
                    }
                }
            }
        }
    }

    /**
     * @fn copy_directory_contents(&Path, &Path)
     * @brief follow top-level directory links but skip links below them.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn copy_directory_contents(src: &Path, dst: &Path) {
        if let Err(err) = fs::create_dir_all(dst) {
            Logger::instance().log_error(&format!(
                "[install] Failed to create directory {}: {err}",
                dst.display()
            ));
            return;
        }
        let read_dir = match fs::read_dir(src) {
            Ok(rd) => rd,
            Err(err) => {
                note_disk_full_if_applicable(&err);
                Logger::instance().log_error(&format!(
                    "[install] Failed to iterate directory {}: {err}",
                    src.display()
                ));
                return;
            }
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    note_disk_full_if_applicable(&err);
                    Logger::instance().log_error(&format!(
                        "[install] Failed to iterate directory {}: {err}",
                        src.display()
                    ));
                    return;
                }
            };
            let path = entry.path();
            let Some(name) = path.file_name() else {
                continue;
            };
            let target = dst.join(name);
            // `Path::is_dir` follows links, so a symlink to a directory takes the folder branch.
            if path.is_dir() {
                Self::copy_folder(&path, &target);
            } else {
                Self::copy_file(&path, &target);
            }
        }
    }

    /**
     * @fn move_directory_contents(&Path, &Path)
     * @brief fall back to copy-and-remove after non-disk-full rename errors.
     * @author Alex (https://github.com/lextpf)
     *
     * every rename error except disk-full tries copy then remove. a disk-full error sets the
     * process-wide flag and skips the child without copying.
     */
    pub fn move_directory_contents(src: &Path, dst: &Path) {
        if !src.exists() {
            Logger::instance().log_warning(&format!(
                "[install] Missing folder for move: {}",
                src.display()
            ));
            return;
        }
        if let Err(err) = fs::create_dir_all(dst) {
            // unlike copy_directory_contents, this create failure does set the disk-full flag.
            note_disk_full_if_applicable(&err);
            Logger::instance().log_error(&format!(
                "[install] Failed to create directory {}: {err}",
                dst.display()
            ));
            return;
        }
        let read_dir = match fs::read_dir(src) {
            Ok(rd) => rd,
            Err(err) => {
                note_disk_full_if_applicable(&err);
                Logger::instance().log_error(&format!(
                    "[install] Failed to iterate directory for move {}: {err}",
                    src.display()
                ));
                return;
            }
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    note_disk_full_if_applicable(&err);
                    Logger::instance().log_error(&format!(
                        "[install] Failed to iterate directory for move {}: {err}",
                        src.display()
                    ));
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
                Logger::instance().log_error(&format!(
                    "[install] Move error (disk full) {} -> {}: {rename_err}",
                    path.display(),
                    target.display()
                ));
                continue;
            }
            // an ordinary cross-volume move lands here too, so this line is what tells the two
            // apart in a log.
            Logger::instance().log_warning(&format!(
                "[install] rename {} -> {} failed ({rename_err}); falling back to copy",
                path.display(),
                target.display()
            ));
            if path.is_dir() {
                Self::copy_folder(&path, &target);
            } else {
                Self::copy_file(&path, &target);
            }
            // best-effort source cleanup: a failure leaves the entry for the install's
            // temp-directory removal. a link is unlinked as a link, never recursed into. a real
            // directory recurses. everything else - a real file, a file symlink, or a directory
            // symlink or junction, all of which report `is_dir() == false` under `symlink_metadata`
            // - is removed as a single entry. `remove_file` handles files and, on every platform,
            // file symlinks plus unix directory symlinks. a windows directory reparse point is
            // directory-attributed, so `DeleteFileW` cannot delete it; the `remove_dir`
            // (RemoveDirectoryW) fallback unlinks the reparse point without following it. on unix
            // the `remove_file` unlink already succeeds and the fallback never runs.
            let remove_result = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_dir() => fs::remove_dir_all(&path),
                Ok(_) => fs::remove_file(&path).or_else(|_| fs::remove_dir(&path)),
                Err(err) => Err(err),
            };
            if let Err(err) = remove_result {
                Logger::instance().log_warning(&format!(
                    "[install] Move fallback could not remove source {}: {err}",
                    path.display()
                ));
            }
        }
    }

    /**
     * @fn disk_full_encountered() -> bool
     * @brief true when any copy or move has hit "no space on device".
     * @author Alex (https://github.com/lextpf)
     *
     * sticky and process-global.
     */
    pub fn disk_full_encountered() -> bool {
        DISK_FULL.load(Ordering::Relaxed)
    }

    /**
     * @fn reset_disk_full()
     * @brief clear the process-global disk-exhaustion latch.
     * @author Alex (https://github.com/lextpf)
     *
     */
    pub fn reset_disk_full() {
        DISK_FULL.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_root(tag: &str) -> PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("salma_t14_{tag}_{}_{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(path, contents).expect("write file");
    }

    fn read_file(path: &Path) -> String {
        fs::read_to_string(path).expect("read file")
    }

    fn file_op(src: &Path, dst: &Path, priority: i32, document_order: i32) -> FileOperation {
        FileOperation {
            op_type: FileOpType::File,
            source: src.to_string_lossy().into_owned(),
            destination: dst.to_string_lossy().into_owned(),
            priority,
            document_order,
        }
    }

    #[test]
    fn execute_sorts_ascending_by_priority_so_highest_wins() {
        let root = temp_root("prio");
        let low = root.join("low.txt");
        let high = root.join("high.txt");
        write_file(&low, "LOW");
        write_file(&high, "HIGH");
        let dest = root.join("out/shared.txt");

        let mut ops = FileOperations::new();
        // high priority added first; the ascending sort must still copy it last.
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
        // descending document_order verifies that execute ignores it. stable insertion order makes
        // the last operation win.
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

        FileOperations::copy_file(&src_b, &dest);
        assert_eq!(read_file(&dest), "B");
        let _ = fs::remove_dir_all(&root);
    }

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

    #[cfg(windows)]
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    #[cfg(not(windows))]
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

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
            // symlink creation needs SeCreateSymbolicLinkPrivilege on windows; without it this case
            // cannot be exercised, so it is skipped rather than failed.
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

    #[test]
    fn copy_directory_contents_flattens_into_dst_and_leaves_src() {
        let root = temp_root("cdc");
        let src = root.join("src");
        write_file(&src.join("top.txt"), "T");
        write_file(&src.join("sub/leaf.txt"), "L");
        let dest = root.join("dst");

        FileOperations::copy_directory_contents(&src, &dest);

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
        // there is no source-existence check, and the destination is created before the iteration
        // that fails.
        let root = temp_root("cdcmissing");
        let src = root.join("absent");
        let dest = root.join("dst");

        FileOperations::copy_directory_contents(&src, &dest);

        assert!(dest.is_dir(), "dst is created before the source is touched");
        assert_eq!(fs::read_dir(&dest).expect("read dst").count(), 0);
        let _ = fs::remove_dir_all(&root);
    }

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
        // renaming onto an existing non-empty directory fails on every platform, which drives the
        // copy plus remove fallback without needing a second volume.
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
        // a directory symlink reports is_dir() == false under symlink_metadata, so the cleanup must
        // not route it to remove_file alone: on windows a directory reparse point is
        // directory-attributed and remove_file (DeleteFileW) cannot delete it. the remove_file ->
        // remove_dir fallback unlinks the link either way.
        let root = temp_root("mdcdirlink");
        let real_target = root.join("target");
        write_file(&real_target.join("inner.txt"), "T");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        let link = src.join("child");
        if !try_symlink_dir(&real_target, &link) {
            // directory-symlink creation needs SeCreateSymbolicLinkPrivilege on windows; skip
            // rather than fail when unavailable.
            let _ = fs::remove_dir_all(&root);
            return;
        }
        // force the copy+remove fallback: a non-empty dst/child makes the per-child fs::rename fail
        // on every platform.
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
        // the symlink target itself is untouched (remove_all unlinks the link, never recurses into
        // it).
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
        // note: a real ENOSPC cannot be provoked from a unit test, so the set-from-I/O path is
        // exercised only by is_disk_full's mapping above. the flag is process-wide, so this test
        // only asserts the cleared state it establishes itself.
        FileOperations::reset_disk_full();
        assert!(!FileOperations::disk_full_encountered());
    }
}
