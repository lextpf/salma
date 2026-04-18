//! Content-root detection for NON-FOMOD archives.
//!
//! Rust port of `src/ModStructureDetector.hpp`/`.cpp`. Used by
//! [`crate::installation_service`] to find which subdirectory of an extracted
//! archive holds the actual mod content, so a wrapper folder
//! (`ModName-v1.2/meshes/...` instead of `meshes/...`) does not get installed
//! one level too deep.
//!
//! Log call sites mirror the C++ tags and wording so MO2's log window reads
//! the same. The one unavoidable difference is the error TEXT inside the
//! "Cannot scan" warning: the C++ interpolates `filesystem_error::what()`, this
//! interpolates `std::io::Error`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::logger::Logger;

/// The well-known game-data folder names that mark a directory as a mod root,
/// in the C++ declaration order (`ModStructureDetector.cpp:14-26`).
///
/// Order is observable only through short-circuiting, which cannot change the
/// boolean result, so it is preserved for faithfulness rather than behavior.
const MOD_FOLDERS: [&str; 11] = [
    "SKSE",
    "meshes",
    "textures",
    "interface",
    "sound",
    "scripts",
    "seq",
    "F4SE",
    "SFSE",
    "OBSE",
    "materials",
];

/// Whether `dir` looks like a mod root, i.e. contains any of [`MOD_FOLDERS`].
///
/// Mirror of `ModStructureDetector::has_mod_structure`
/// (`ModStructureDetector.cpp:10-33`). Two C++ behaviors are reproduced
/// deliberately:
///
/// - Matching is CASE-INSENSITIVE because the C++ relies on `fs::exists` over
///   NTFS rather than folding the name itself. Rust's [`Path::exists`] goes to
///   the same Win32 call, so `meshes`, `Meshes`, and `MESHES` all match here
///   too, with no explicit `to_lower`.
/// - The probe is `exists`, not `is_dir`, so a plain FILE named `textures`
///   makes the directory a "mod root". The C++ has the same hole; it is
///   reproduced, not fixed.
///
/// Divergence (documented in PARITY-NOTES "Task 15"): the C++ calls the
/// THROWING `fs::exists` overload, which propagates a `filesystem_error` for
/// failures other than not-found and thereby aborts the enclosing scan.
/// [`Path::exists`] instead reports `false` for every error, so a permission
/// fault on one probe lets the scan continue. Unreachable in practice: the
/// input is always a freshly-extracted temp tree owned by this process.
pub fn has_mod_structure(dir: &Path) -> bool {
    MOD_FOLDERS.iter().any(|folder| dir.join(folder).exists())
}

/// Every immediate subdirectory of `archive_root` that [`has_mod_structure`].
///
/// Mirror of `ModStructureDetector::find_main_mod_folders`
/// (`ModStructureDetector.cpp:35-61`). One level deep only: a nested wrapper
/// such as `Outer/Inner/meshes/` is NOT found, matching the C++ limitation
/// documented in `ModStructureDetector.hpp:31-33`.
///
/// Returns an empty vector when `archive_root` cannot be iterated, mirroring
/// the C++ `catch (const fs::filesystem_error&)` that logs a warning and
/// returns whatever it had collected. The C++ keeps the partial results
/// gathered before the fault; so does this, because the error can only surface
/// from the iterator itself and the vector is built incrementally.
///
/// Result ORDER follows the directory iterator on both sides (Win32
/// `FindFirstFileW`/`FindNextFileW` under both `fs::directory_iterator` and
/// [`fs::read_dir`]) and is not further sorted. Order does not leak into
/// install behavior: the caller uses index 0 only when the vector holds exactly
/// one entry, and otherwise selects by name match, treating any count other
/// than one as fatal.
pub fn find_main_mod_folders(archive_root: &Path) -> Vec<PathBuf> {
    let logger = Logger::instance();
    let mut results = Vec::new();

    let entries = match fs::read_dir(archive_root) {
        Ok(entries) => entries,
        Err(err) => {
            logger.log_warning(&format!(
                "[install] Cannot scan \"{}\": {err}",
                archive_root.display()
            ));
            return results;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                // Mid-iteration fault. The C++ `filesystem_error` would escape
                // the loop into the catch and stop the scan with partial
                // results, so stop here too rather than skipping just this
                // entry.
                logger.log_warning(&format!(
                    "[install] Cannot scan \"{}\": {err}",
                    archive_root.display()
                ));
                break;
            }
        };
        let path = entry.path();
        // `is_directory()` in the C++; `file_type()` here follows symlinks the
        // same way `directory_entry::is_directory` does (it uses `status`, not
        // `symlink_status`), so a junction pointing at a mod folder counts on
        // both sides.
        if !path.is_dir() {
            continue;
        }
        if has_mod_structure(&path) {
            logger.log(&format!(
                "[install]    candidate mod folder: \"{}\"",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            results.push(path);
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::random_hex_string;
    use std::fs as stdfs;

    /// Unique scratch directory under the OS temp dir.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("salma-msd-{tag}-{}", random_hex_string(8)));
        stdfs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    #[test]
    fn empty_directory_has_no_mod_structure() {
        let root = scratch("empty");
        assert!(!has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn each_recognized_folder_marks_a_mod_root() {
        for folder in MOD_FOLDERS {
            let root = scratch("one");
            stdfs::create_dir_all(root.join(folder)).expect("create marker");
            assert!(has_mod_structure(&root), "{folder} should mark a mod root");
            stdfs::remove_dir_all(&root).ok();
        }
    }

    #[test]
    fn unrecognized_folder_does_not_mark_a_mod_root() {
        let root = scratch("other");
        stdfs::create_dir_all(root.join("docs")).expect("create dir");
        stdfs::create_dir_all(root.join("source")).expect("create dir");
        assert!(!has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    /// The C++ probes with `fs::exists`, which on Windows is case-insensitive.
    #[test]
    fn folder_match_is_case_insensitive() {
        let root = scratch("case");
        stdfs::create_dir_all(root.join("MeShEs")).expect("create dir");
        assert!(has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    /// `fs::exists` does not require a directory, so a FILE named after a mod
    /// folder counts. Reproduced from the C++, not fixed.
    #[test]
    fn a_file_named_like_a_mod_folder_also_counts() {
        let root = scratch("file");
        stdfs::write(root.join("textures"), b"not a directory").expect("write file");
        assert!(has_mod_structure(&root));
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn find_main_mod_folders_returns_only_qualifying_subdirectories() {
        let root = scratch("find");
        stdfs::create_dir_all(root.join("ModA").join("meshes")).expect("create");
        stdfs::create_dir_all(root.join("ModB").join("textures")).expect("create");
        stdfs::create_dir_all(root.join("Docs").join("readme")).expect("create");
        stdfs::write(root.join("loose.txt"), b"x").expect("write");

        let mut found: Vec<String> = find_main_mod_folders(&root)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, vec!["ModA".to_string(), "ModB".to_string()]);
        stdfs::remove_dir_all(&root).ok();
    }

    /// One level deep only: `Outer/Inner/meshes` must NOT be detected.
    #[test]
    fn find_main_mod_folders_does_not_recurse() {
        let root = scratch("nested");
        stdfs::create_dir_all(root.join("Outer").join("Inner").join("meshes")).expect("create");
        assert!(find_main_mod_folders(&root).is_empty());
        stdfs::remove_dir_all(&root).ok();
    }

    #[test]
    fn find_main_mod_folders_on_missing_root_is_empty() {
        let root = std::env::temp_dir().join(format!("salma-msd-absent-{}", random_hex_string(8)));
        assert!(find_main_mod_folders(&root).is_empty());
    }

    /// A top-level `meshes/` makes the ROOT a mod root, but `find_main_mod_folders`
    /// reports children only, so the caller falls through to the flat copy.
    #[test]
    fn mod_folder_at_root_is_not_reported_as_a_child_candidate() {
        let root = scratch("flat");
        stdfs::create_dir_all(root.join("meshes")).expect("create");
        assert!(has_mod_structure(&root));
        assert!(find_main_mod_folders(&root).is_empty());
        stdfs::remove_dir_all(&root).ok();
    }
}
